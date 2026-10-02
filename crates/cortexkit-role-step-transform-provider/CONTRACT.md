# `step-transform-provider/v1` — role contract (draft)

Stability: **alpha, draft**. Written by the role's owner (Magic Context) for
review in the design room, with the runner's owner reviewing as the caller;
nothing here ships until the room signs off. This document is the role
definition; the Rust types in this crate are its wire shapes,
`test-vectors/step-transform-provider-v1/` at the repository root holds its
vectors, and a separate `-conformance` crate will hold its suite (§12).

Derives from the CK extensibility design r7.3, §3.8, §4.5, §6 and §8, its
owner corrections, and the rulings the design room settled on hooks.
`llm-runner/v1` §11.2 states what the runner owes at each hook call site;
this document states the provider's side. Appendix A lists where the two
disagree today.

Every item is marked:

- **[pinned]**: the design or a settled ruling states it. A provider must do
  it, and a runner may rely on it.
- **[open: Qn]**: the design is silent or ambiguous, and this draft writes one
  option down so the types and vectors have something to pin. Question `Qn`
  under "Open questions" lists the options.

Codes this role shares with `llm-runner/v1` (the tool-result reasons,
`pre_user_unavailable`, `invalid_params`) are taken from that crate, which
this crate depends on, so the two cannot drift apart.

A step-transform provider is any module that makes write-time changes to
the newest message of a session. Nothing here names a particular
implementation; "Gaps in Magic Context today" at the end compares the one
planned provider with this document.

## 1. Role identity and addressing

- [pinned] A provider lists `step-transform-provider/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe`, the declaration
  op and the hook op. [open: Q2] their names, drafted as
  `transform.declare` and `transform.hook`. [open: Q3] whether the
  declaration is an op at all.
- [pinned] A runner refuses a module whose `role.describe` lacks a required
  op, by name, before routing anything to it.
- [open: Q1] Every op is a request `{method, params}` on a route the runner
  opens to the provider under the session's scope.

## 2. `role.describe`

- [pinned] The answer is `{majors: [{version, ops, stability}],
  implementation_version, capabilities, runner_groups?}` (`RoleDescribe`),
  decoded leniently; only the `step-transform-provider/v1` major and its
  required ops are checked strictly (`check_describe`). Stability is data:
  `alpha`, `beta` or `stable`, read as `alpha` when absent. This draft is
  `alpha`.
- [pinned] The answer describes the provider build, never a session.
- [open: Q12] `runner_groups` lists the `llm-runner/v1` groups the provider
  needs from the runner, `transcript_reads` for a provider that reads
  history beyond what its hooks see.

## 3. Hooks

- [pinned] The hooks (`Hook`), spelled in snake_case on the wire:

| Hook | When | Acts on | Answers |
|---|---|---|---|
| `pre_user` | a user message, or a steered or queued prompt, before it is written | its text | operations |
| `post_assistant` | an assistant message completes, before it is written | the text of its existing text blocks only; never reasoning, signatures or tool calls | operations |
| `pre_tool` | a tool call, before it executes, in three phases | what executes | by phase (§6) |
| `post_tool` | a tool's result, before it is written | the result's text | operations |

- [pinned] Within a turn the runner runs `post_assistant` and writes its
  result; then for each tool call `pre_tool` in its phases, the tool,
  `post_tool`, and writes the result. `pre_user` runs on every user message
  and every steered or queued prompt, before the write and within the send's
  deadline.
- [pinned] A hook's output is fields on the one record it transforms,
  written before anything carrying it is sent, applied once and never
  changed afterwards. Resume replays hook outputs and never calls a hook
  again for a record that is durable. A step whose hook outputs never became
  durable runs its hooks again: nothing they would change was sent.
- [pinned] So a provider must answer a repeated call for the same subject
  as a fresh one, and must not assume an earlier answer was used. State it
  kept for an answer that was never used must not reach the model (kill
  point `HookStateRecorded`).
- [pinned] A steered prompt written before `pre_user` existed is never run
  through it later.

## 4. Subscriptions and the declaration

- [pinned] The plan's `step_transform_items` name the step-transform
  providers in order, each with its preset, params and `subscriptions:
  [{hook, phase?, tools?, ops, on_unavailable, budget_ms}]`
  (`Subscription`), the shape prefrontal's `fetch-plan-v1` vectors carry.
  The runner calls only matching providers.
- [pinned] The provider's declaration bounds what a plan may subscribe to.
  [open: Q3] It is `transform.declare {preset?, params, composition?}`,
  answered `{subscriptions: [{hook, phase?, tools?, ops, on_unavailable?,
  budget_ms}]}` (`Declaration`), fetched with the plan's other items at
  admission.
- [pinned] Shape rules, for declared and planned subscriptions alike:
  `phase` on `pre_tool` and nowhere else; `tools` only on `pre_tool` and
  `post_tool`; at least one op on the hooks that answer with operations;
  and `ops` empty on `pre_tool`, whose answers are by phase (§6): `mutate`
  answers pass or a rewritten input, `validate` pass or deny, `approve` pass
  or a question. A planned `pre_tool` subscription with an op is refused
  `ops_on_pre_tool`.
- [pinned] A provider that wants two `pre_tool` phases declares two
  subscriptions.
- [pinned] The declaration is the source of each subscription's
  `on_unavailable` (`pass` or `refuse`) and `budget_ms` (a positive number
  of milliseconds per call). A declaration may omit `on_unavailable`, which
  then means `refuse`, except on `post_assistant`, which always passes: a
  declaration or plan naming `refuse` there is malformed. Whoever composes
  the plan copies both values from the declaration into every planned
  subscription, and they are frozen with the plan. The runner uses the
  frozen values, capping the budget with its own engine-wide limit.
- [pinned] At admission the runner checks the frozen plan against the
  provider's current declaration for the item's preset and params. A
  planned subscription fits a declared one with the same hook and phase
  whose `tools` cover the planned tools (absent covers every tool; an absent
  planned list needs an absent declared one), whose `ops` include every
  planned op, and whose `on_unavailable` and `budget_ms` the planned ones
  equal or tighten: `refuse` is stricter than `pass`, and a budget no
  greater than the declared one is within bounds (`Declaration::bound`). An
  equal or stricter subscription admits and stays frozen as planned.
- [pinned] The runner refuses a plan the current declaration no longer
  covers with `plan_stale`, with one entry in `detail.differences` per such
  subscription (`StaleDifference`):
  - `{kind: "subscription_loosened", provider, hook, phase, field}`: the
    frozen `tools`, `ops`, `on_unavailable` or `budget_ms` (`field`) is looser
    than declared;
  - `{kind: "subscription_missing", provider, hook, phase}`: the declaration
    has no subscription with the planned hook and phase;
  - `{kind: "preset_missing", provider, preset}`: the provider no longer
    knows the item's preset, and refused `transform.declare` with
    `invalid_params {field: "preset"}`. This difference occurs once per item,
    regardless of its subscription count.
  Subscription-scoped differences always carry `phase`, set to `null` for
  hooks without phases. All `plan_stale` differences are internally tagged
  with `kind`; fetched-item kinds are `composition_digest`, `tools`,
  `capabilities` and `text_tool_names`, each with `provider` and its existing
  payload. `field` names only a subscription field.
- [pinned] A plan that is malformed in itself is refused `invalid_params
  {field: "plan.step_transform_items", item, subscription, problem}`
  (`InvalidSubscriptionDetail`), `item` and `subscription` being indices
  and `problem` named as `SubscriptionProblem::name` spells it. Such a
  problem wins over any staleness, and the first in plan order is reported
  (`check_item`). Tools or ops beyond the declaration are instead stale
  (`subscription_loosened`), because rebuilding the plan against the current
  declaration is the remedy.
- [pinned] The declaration is a pure function of the preset, the params,
  the provider's configuration and the composition. The same inputs give
  the same bytes, and no scope, agent or session identity enters it.

## 5. Operations, exclusivity and order

- [pinned] `pre_user`, `post_assistant` and `post_tool` answer with
  operations, not a new value: `prepend(text)`, `append(text)` or
  `replace(value)` (`Operation`), each with an optional short `note`. The
  runner applies them; each operation, with the module that returned it and
  its note, is the attribution record, so nothing is diffed.
- [pinned] Several providers on one hook apply in order: prepends stack
  outward, appends stack in order, and a `replace` sees the value as it
  stands at that point.
- [pinned] Exclusivity. The session has exactly one reduction owner, its
  compaction provider: the only party that may remove or rewrite what was
  written. On `pre_user` and `post_assistant`, `replace` belongs to the
  reduction owner alone; without one, nobody has it (`check_reduction`).
  Every other step transform is preserving: it may prepend or append.
- [pinned] Preserving transforms run as a separate ordered list, in plan
  order. [open: Q9] The reduction owner's hooks run first, so a later
  preserving prepend is never wiped by a replace (`hook_order`).
- [pinned] `replace` on `post_tool` is a separate permission only the user
  tier grants, per provider and hook, as `{module, hook, tools}`. Neither
  project configuration nor the provider can grant it, and no provider has
  it by default. [open: Q10] how the grant reaches the runner.
- [pinned] A tool's catalog entry declares the operations it accepts on its
  result (`result_ops` in `tool-provider/v1`, all three by default). An
  operation the tool does not accept is refused.
- [pinned] An operation that is not allowed is refused before anything is
  applied or recorded (`check_answer`), never detected afterwards.
  [open: Q7] The runner then treats the hook as unavailable for that call.
- [open: Q6] How operations map onto a subject with several text blocks:
  `prepend` goes before the first text block's text, `append` after the last
  one's, and `replace` writes one text block in place of all of them.

## 6. `pre_tool` phases

- [pinned] Three fixed phases (`Phase`): `mutate` (rewriting hooks),
  `validate` (automatic guards that can deny), `approve` (gates that ask a
  human through elicitation). Each `pre_tool` subscription names one.
- [pinned] What each phase may answer (`check_answer`):

| Phase | `pass` | `mutate {input, note?}` | `deny {text}` | `ask {ask}` |
|---|---|---|---|---|
| `mutate` | yes | yes | no | no |
| `validate` | yes | no | yes | no |
| `approve` | yes | no | no | yes |

- [pinned] `mutate` is a pure function of the call and its frozen config:
  two runs give identical bytes. The runner never re-runs mutate for a call
  whose final input is durable.
- [pinned] Any deny wins, whatever the order. A deny is the call's error
  result with the provider's text and reason `pre_tool_denied`; nothing is
  dispatched and the run goes on.
- [pinned] A human's decline at `approve` is the runner's
  `pre_tool_declined`; an expired question under `on_expiry: deny` is
  `pre_tool_expired`; an unavailable hook under `refuse` is
  `pre_tool_unavailable`.
- [pinned] `validate` runs again right before an approved call executes,
  and a failed second validation consumes the approval.
- [pinned] The phase order is guaranteed only where the runner declares
  `ordered_hook_phases`. Elsewhere hooks on one call still form a sequential
  chain and any deny still wins, and the runner refuses at admission a
  session that subscribes a `mutate` hook and a `validate` or `approve` hook
  on the same tool.
- [open: Q8] The `ask` is `{prompt, options?, expiry_ms, on_expiry,
  material_damage, late_execution}` (`ApprovalAsk`); the elicitation role
  owns how it is filed and answered.

## 7. `transform.hook`

- [pinned] The request (`HookCall`) carries `session` (the runner's opaque
  name for the session), `lineage_id` (absent only on `pre_user` for the
  first message of a session nothing has been written to), the plan item's
  `preset` and `params` verbatim, and the subject, tagged by `hook`
  (`Subject`):
  - `pre_user {text, mark?, delivery?}`: `mark` is the sender's mark on a
    steered or queued prompt, opaque; `delivery` is `queue`, `steer` or
    `interrupt` for an owner's prompt, decoded open;
  - `post_assistant {step_id, text}`;
  - `pre_tool {step_id, phase, tool, tool_call_id, call_key?, input}`:
    `input` is the input that would execute, after every earlier mutate;
  - `post_tool {step_id, tool, tool_call_id, call_key?, text, is_error}`.
- [pinned] `tool_call_id` is the model's id: display only, not unique.
  `call_key` is the runner's key (`llm-runner/v1` §11.3); it is usually
  absent on `pre_tool`, because it is minted from the dispatch intent,
  written after the hook, and present on `post_tool`.
- [open: Q6] Subjects are text in v1.
- [pinned] The answer (`HookAnswer`) is `{answer: "pass"}`, `{answer:
  "ops", ops}`, `{answer: "mutate", input, note?}`, `{answer: "deny",
  text}` or `{answer: "ask", ask}`. [open: Q2] the spellings.
- [pinned] A hook that times out (after the frozen `budget_ms`, capped by
  the runner), fails, or answers something it may not is unavailable for
  that call, and the frozen plan subscription's `on_unavailable` (§4)
  decides:
  - `post_assistant`: the message passes through unchanged, and the runner
    records that;
  - `pre_tool` under `refuse`: the call is denied `pre_tool_unavailable`;
    under `pass`, the input executes;
  - `post_tool` under `refuse`: the result is withheld, written as an error
    with reason `post_tool_unavailable` naming the provider; the error text
    says the call executed;
  - `pre_user` under `refuse`: the run ends `error` with `provider_code`
    `pre_user_unavailable`; a steered or queued send is refused with it and
    writes nothing.

## 8. Purity and identity

- [pinned] The declaration derives only from the preset, the params, the
  provider's configuration and the composition, never from scope, agent or
  session identity.
- [pinned] No request in this role carries scope, agent or session identity
  beyond the opaque `session` handle. A provider uses it only as a key for
  its own per-session state (a tag counter, say), never to choose a
  variant.
- [pinned] A provider tells prompts apart itself, by the mark or by their
  wrapping. The runner does not classify turns.

## 9. Error codes

Codes ride as the `code` of an `ERROR` frame's body `{code, message,
detail?}`. They are open strings: a party that meets one it does not know
treats it as a terminal refusal of that one request.

### 9.1 Provider refusals (`errors`)

| Code | When | Detail | Retry |
|---|---|---|---|
| `invalid_params` | a request field is malformed, or names a preset or params value the provider does not know | `field` | no |
| `not_subscribed` | a hook call for a hook and phase the provider did not declare for the call's preset and params | `hook`, `phase?` | no |
| `transient` | a condition that may clear by itself | — | **yes**, within the hook's budget |

- [pinned] Any refusal of a hook call makes the hook unavailable for that
  call (§7).
- [pinned] The malformed-request code is `invalid_params`, as runners
  answer; `tool-provider/v1` spells it `invalid_request`.

### 9.2 Codes the runner answers or writes (`errors::runner_codes`)

| Code | Where |
|---|---|
| `invalid_params {field: "plan.step_transform_items", item, subscription, problem}` | admission, for a malformed subscription, tools or ops beyond the declaration, or a `replace` the provider may not have |
| `plan_stale {differences}` | admission, for a subscription the current declaration no longer covers: `subscription_missing`, `preset_missing` or `subscription_loosened` (§4) |
| `pre_user_unavailable` | a refusal of a steered or queued send, and a run's `provider_code` |
| `pre_tool_denied`, `pre_tool_unavailable`, `pre_tool_declined`, `pre_tool_expired`, `post_tool_unavailable` | a tool call's error result |

## 10. Decoding

- [pinned] **Lenient on fields**: `role.describe`, the declaration request
  and answer, hook requests and answers. A newer runner can add fields
  without breaking an older provider, and the reverse.
- [pinned] **Strict on values** this role defines and a party must act on:
  `hook`, `phase`, `op`, `on_unavailable`, `answer`. An unknown value does
  not decode. On a request the provider refuses it `invalid_params`; on an
  answer the runner treats the hook as unavailable. Values a runner may grow
  (`delivery`, `on_expiry`, `late_execution`) are decoded open.

## 11. Crash and durability guarantees

What a provider owes across its own crash, checked (§12) by killing the
provider and restarting it on the same state root
(`cortexkit-role-harness`). The runner's side (`StepRecorded`,
`DispatchIntent`, `ToolResultRecorded`, `SendRecorded`) is `llm-runner/v1`
§14.

1. [pinned] **Repeated calls are fresh calls.** A call repeated after a kill
   gets a well-formed answer.
2. [pinned] **Mutate is pure across a kill.** A `mutate` answer is
   byte-identical before and after.
3. [pinned] **Unused state stays unused.** State kept for an answer that was
   never sent never surfaces in a later answer.
4. [pinned] **The declaration is stable.** The same declaration request
   gives the same bytes before and after a kill.

Kill points (`points.rs`). [pinned] The list below.

| Point | Durable state after the kill | Property |
|---|---|---|
| `HookStateRecorded` | the provider's state behind a hook answer; the answer not sent | 1, 2, 3 |

A provider whose state lives in a store where truncation is not meaningful
(SQLite, say) reaches the point with a fault hook followed by a real process
kill, and declares that. A suite run fails unless at least one kill was a
real process kill.

## 12. Conformance suite (planned)

A separate `cortexkit-role-step-transform-provider-conformance` crate, run
by each provider in its own CI against its real module over real routes,
never a double. It drives the provider with a scripted runner. It will
check:

| Case | Checks |
|---|---|
| `role_describe_shape`, `role_describe_cacheable`, `extra_op_still_admitted` | §2 |
| `declaration_well_formed`, `declaration_pure` | §4, §8: two declarations with the same inputs and different `session` routes are byte-identical |
| `answers_within_declaration` | §5: every operation is one the declaration names |
| `post_assistant_text_only` | §3 |
| `mutate_pure` | §6: two runs give identical bytes |
| `phase_answers_allowed` | §6 |
| `unknown_hook_refused` | §10 |
| `not_subscribed_refused` | §9 |
| `repeat_call_fresh` | §3 |
| `crash_at_HookStateRecorded` | §11 |

Runner-side rules no live provider can exercise (bounding a plan, the
reduction rule, the order, refusing a disallowed answer) are tested against
the vectors in this crate.

## Not in v1

- Subjects other than text: images, structured tool results.
- A deny from `pre_user` or `post_tool`. Only `pre_tool` denies.
- A provider stopping a run. A caller that wants a session stopped after a
  deny cancels the run itself.
- A rule language. A rules engine is a module that answers hooks.
- Hooks on reasoning, signatures or tool-call blocks.

## Open questions

Each open question lists the options seen and the draft's choice. A
question keeps its number when it is settled and moves to "Settled
questions" below, with the decision.

- **Q1. The envelope.** (a) **draft:** `{method, params}`, as
  `llm-runner/v1` settled for its own ops; (b) `{name, arguments}`.
- **Q2. Names.** **draft:** ops `transform.declare` and `transform.hook`;
  answers `pass`, `ops`, `mutate`, `deny`, `ask`; operations `prepend`,
  `append`, `replace`.
- **Q3. Where the declaration lives.** (a) **draft:** an op answered per
  preset and params, fetched at admission, because one provider's
  subscriptions differ by preset (a head and a worker); (b) in
  `role.describe`, which describes the build and cannot vary by preset;
  (c) in the HELLO manifest, which the design uses for budgets and
  `pre_tool`'s `on_unavailable`.
- **Q6. Subject shape.** **draft:** text, with operations mapped onto
  several text blocks as §5 says.
- **Q7. A disallowed answer.** (a) **draft:** the hook is unavailable for
  that call; (b) the answer is dropped and the hook treated as `pass`.
  (b) would let a broken enforcing hook fail open.
- **Q8. Approve.** **draft:** `pass` or `ask`, never `deny`; the ask's
  fields as §6 lists, decoded leniently.
- **Q9. Where the reduction owner runs.** (a) **draft:** first, so a replace
  never wipes another provider's prepend; (b) last.
- **Q10. The user's `replace` grant on `post_tool`.** Not carried by this
  role. Options: (a) the plan carries `grants: [{module, hook, tools}]`
  from the starter; (b) the runner reads the user tier itself.
- **Q12. `runner_groups`.** As in `compaction-provider/v1`.

## Settled questions

- **Q11. Tools or ops beyond the declaration.** Settled: `plan_stale` with
  `{kind: "subscription_loosened", provider, hook, phase, field}`, naming
  `tools` or `ops`. A plan composed against an older, wider declaration is
  stale like a missing hook. Malformed plans remain `invalid_params` (§4).

- **Q4. Where `on_unavailable` and the budget live.** Settled: the
  declaration is the source, and the composer copies both into every
  planned subscription, where they are frozen. At admission an equal or
  stricter planned subscription admits; a looser one refuses `plan_stale`
  with `{kind: "subscription_loosened", provider, hook, phase, field}`; a declared hook or
  preset that is gone refuses `plan_stale` with `subscription_missing` or
  `preset_missing` (§4).
- **Q5. `ops` on `pre_tool`.** Settled: empty. `pre_tool` answers by phase;
  a planned op there is refused `invalid_params` with `ops_on_pre_tool`
  (§4).
- **Q13. Hook names.** Settled: snake_case `pre_user`, `post_assistant`,
  `pre_tool`, `post_tool`; `pre_tool` phases `mutate | validate |
  approve`; a decline at `approve` is the runner's `pre_tool_declined`.
- **Q14. Plan and declaration.** Settled: the plan carries
  `step_transform_items` with subscriptions, and the provider's declaration
  bounds what a plan may subscribe to.
- **Q15. Exclusivity.** Settled: one reduction owner per session;
  preserving transforms run as a separate ordered list.
- **Q16. Purity.** Settled: what a provider's fetch returns derives only
  from the composition, preset, params and config, never from scope, agent
  or session identity.

## Appendix A. Cross-check with `llm-runner/v1` and the fetch plan

### A.1 `llm-runner/v1`

Checked against `llm-runner/v1` at commons commit 5262544; paths are in
this repository. The runner role's hook section
(`crates/cortexkit-role-llm-runner/CONTRACT.md:542-556`) and its reasons
(`crates/cortexkit-role-llm-runner/CONTRACT.md:617-642`,
`crates/cortexkit-role-llm-runner/src/errors.rs:154-191`) agree with this
document; the codes, `plan_stale` among them, are taken from that crate.
What remains:

1. **Hook names in prose.** The runner writes PreUser, PostAssistant,
   PreTool and PostTool (`crates/cortexkit-role-llm-runner/CONTRACT.md:544-548`).
   The wire names are snake_case (Q13). Prose only; no wire disagreement.
2. **No admission refusal for step transforms.** The runner's admission
   section (`crates/cortexkit-role-llm-runner/CONTRACT.md:439-465`) names
   `plan_stale` only for fetched items that disagree with the composition,
   and nothing for a subscription the declaration no longer covers, a
   malformed subscription, a `replace` on `pre_user` or `post_assistant` by
   a provider that is not the reduction owner, or a mutate plus validate or
   approve on one tool without `ordered_hook_phases`. This document §4,
   §5, §6 and Q11.
3. **No reduction owner and no order rule.** The runner's §11.2 does not
   say that the compaction provider is the only `replace` owner on
   `pre_user` and `post_assistant`, or that it runs before the preserving
   list. This document §5.
4. **No rule for a disallowed answer.** The runner's §11.2 does not say what
   happens to an answer this role refuses. This document §5 and Q7.

### A.2 Prefrontal's `fetch-plan-v1` vectors

Checked against prefrontal at `3e69ed9d4a048a458db54203bd93a1069598222e`,
`test-vectors/fetch-plan-v1/`, including the README's "Digest migration
(2026-10-02)". `test-vectors/step-transform-provider-v1/subscriptions.json`
carries all eleven step-transform admission cases (`fetch_plan_v1`), each
with the plan's subscriptions, preset, current declaration and expected answer
copied from that revision. `fetch_plan_admission_vectors_agree` pins the full
revision and checks that `check_item` gives the same refusal bytes or admits
and freezes the subscriptions unchanged.

Resolved since the first draft:

- **`ops` on `pre_tool`.** Empty; a planned op there is malformed and refuses
  `invalid_params` with `ops_on_pre_tool` (Q5).
- **Frozen policy and budget.** The declaration supplies both; equal or
  stricter subscriptions admit unchanged, looser ones are stale (Q4).
- **Tools and ops bounds.** Wider planned bounds refuse `plan_stale` with
  `subscription_loosened`, naming `tools` or `ops` (Q11).
- **Phase attribution.** Every subscription difference includes `phase`,
  including `null` on phase-less hooks. The two-phase plan's refusal names
  only the loosened `validate` subscription, not `mutate`.
- **Missing presets.** One `{kind: "preset_missing", provider, preset}` per
  item, including the vector with two subscriptions.
- **One difference encoding.** All `plan_stale` differences use an internal
  `kind` tag and `provider`. Fetched-item kinds are `composition_digest`,
  `tools`, `capabilities` and `text_tool_names`; `field` names only a
  subscription field.
- **Malformed plans.** Every malformed-plan refusal uses `invalid_params`,
  including the fetch plan's own checks, not `invalid_request`.
- **Digest migration.** Admission shape changes replace no existing plan or
  composition digests. The two-phase plan adds new digests only.

What still differs: nothing.

### A.3 The design

1. **Where `on_unavailable` and budgets live.** The design puts
   `on_unavailable` on each `post_tool` and `pre_user` subscription and on
   each `pre_tool` provider's manifest, and budgets in the manifest
   (`magic-context/.cortexkit/alfonso/plans/ck-extensibility-design-r7.3.md:815-821`).
   This document makes the declaration the source and freezes both on each
   planned subscription (Q4).
2. **Who owns `replace`.** The design gives `replace` on PostAssistant and
   PreUser to the session's `context_management` owner
   (`ck-extensibility-design-r7.3.md:793`). This document names that owner
   the reduction owner, the session's compaction provider, as the room
   settled.

## Appendix B. Gaps in Magic Context today

Where `ck-mc` (`crates/mc-module` in the magic-context repository) differs
from this document. None of this is a change request beyond the work the
extensibility plan already schedules for Magic Context.

1. **No role claim and no role ops.** The manifest provides only a
   `ToolProvider` route role (`crates/mc-module/src/lib.rs:18317-18323`),
   and the dispatch has no `role.describe`, `transform.declare` or
   `transform.hook` (`crates/mc-module/src/lib.rs:13755-13788`).
2. **Edits are applied to the outgoing array on every pass, not returned
   as operations once.** Tag prefixes, tag overlays and user hints are
   written into blocks of the array the `transform` pass serves
   (`crates/mc-module/src/transform.rs:9748,9812,9886`), and conditional
   tagging is pinned as part of that output
   (`crates/mc-module/src/tests/broca_contract.rs:286-302`). Under this role
   each would be a `prepend` or `append` returned from a hook, applied once
   by the runner and replayed from its record.
3. **No declaration.** There are no subscriptions, no `on_unavailable` and
   no budgets anywhere in the module.
4. **Reasoning clearing is a history rewrite.** It runs while the native
   array is built (`crates/mc-module/src/transform.rs:15247`).
   `post_assistant` never touches reasoning, so under the roles this is the
   compaction provider's replacement, not a hook.
5. **A request flag instead of a preset.** The subagent path is
   `is_subagent` on the request (`crates/mc-module/src/transform.rs:684-686`).
   Under this role a subagent's behaviour comes from its plan item's preset
   or params.
6. **No kill-point hook.** Nothing stops the module at `HookStateRecorded`.
