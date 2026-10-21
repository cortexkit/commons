# `step-transform-provider/v1` — role contract

Stability: **alpha**. This document is the role
definition; the Rust types in this crate are its wire shapes,
`test-vectors/step-transform-provider-v1/` inside this crate holds its
vectors. Conformance requirements are described in §12; this crate does not
contain a provider conformance runner.

`llm-runner/v1` states what the runner owes at each hook call site;
this document states the provider's side.

Every item is marked **[pinned]**: a settled, binding requirement, even before
the crate's first release. A provider must
do it, and a runner may rely on it.

Codes this role shares with `llm-runner/v1` (the tool-result reasons,
`pre_user_unavailable`, `invalid_params`) are taken from that crate, which
this crate depends on, so the two cannot drift apart.

A step-transform provider is any module that makes write-time changes to
the newest message of a session.

Optional whole-message and lineage fields support runners that cannot serve
`session.read`. When they are absent, a hook request keeps its existing bytes,
and the provider reads history and answers exactly as it does today.

## Terms

- **Plan composer**: the component that writes a session's plan and freezes
  it. It checks each provider's runner groups against the runner, places
  the providers in order, and fills the plan's user-tier fields.
- **Runner group**: a capability group a runner advertises in its
  `llm-runner/v1` `role.describe`, such as `compaction` or
  `transcript_reads`. A provider lists the groups it needs in its own
  `role.describe.runner_groups`.
- **Reduction owner**: the session's compaction provider, the only party
  that may remove or rewrite history already written. Every other step
  transform may only add to it.
- **Signed thinking**: model reasoning blocks that carry a model provider's
  signature. Any change to them, even one byte, invalidates the signature,
  so nothing in these roles ever edits them.

## 1. Role identity and addressing

- [pinned] A provider lists `step-transform-provider/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe`, the declaration
  op `transform.declare` and the hook op `transform.hook`.
- [pinned] A runner refuses a module whose `role.describe` lacks a required
  op, by name, before routing anything to it.
- [pinned] Step-transform routes are module-level and unscoped. The opaque
  session handle in a hook call is a state key, not an authorization credential.
- [pinned] Every hook request (`transform.hook`, §7)
  carries a required string `harness`, next to `session`: the harness
  named in the session's key, which identifies the caller (for example
  `broca`). This differs from the route's bind harness, the harness a
  connection declares when it opens the route; a runner always declares
  `runner`. A request without `harness` does not decode; it is
  never defaulted.
- [pinned] A provider keys a runner conversation, and every piece of
  per-session state it keeps for one, on `(project_root, session,
  harness)`: the project root it serves the request for, the session
  handle and the harness. Two requests that differ in any of the three
  belong to different conversations.
- [pinned] Every request in this role is `{method, params}` (`OpRequest`):
  `method` is the op's name and `params` its request. `role.describe`
  takes empty params (`DescribeRequest`).

## 2. `role.describe`

- [pinned] The answer is `{majors: [{version, ops, stability}],
  implementation_version, capabilities, runner_groups?}` (`RoleDescribe`),
  decoded leniently; only the `step-transform-provider/v1` major and its
  required ops are checked strictly (`check_describe`). Stability is data:
  `alpha`, `beta` or `stable`, read as `alpha` when absent. This draft is
  `alpha`.
- [pinned] The answer describes the provider build, never a session.
- [pinned] The provider declares what it needs from the runner in
  `role.describe.runner_groups`: the runner groups it needs, such
  as `transcript_reads` for a provider that reads history beyond what its
  hooks see. A list of strings, omitted when empty.
- [pinned] The plan composer checks `runner_groups` against the runner
  groups the runner advertises when it composes the session. An unmet group fails the launch, and the failure names each
  unmet group (`unmet_runner_groups`). Runner requirements are carried
  nowhere else.

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

The admitted host runner lane is the scoped admission exception in §13.1.

Admission is the runner's check of a proposed session plan before it accepts
the session and freezes that plan for use.

- [pinned] The plan's `step_transform_items` name the step-transform
  providers in order, each with its preset, params and `subscriptions:
  [{hook, phase?, tools?, ops, on_unavailable, budget_ms}]`
  (`Subscription`), the shape the plan composer's `fetch-plan-v1` vectors carry.
  The runner calls only matching providers.
- [pinned] The provider's per-preset/params declaration bounds what a plan
  may subscribe to. It is read when composing the plan and checked again
  at admission, not carried by build-level `role.describe` or HELLO.
  The declaration op is `transform.declare {preset?, params,
  composition?}` (`DeclareRequest`). Its answer is
  `{subscriptions: [{hook, phase?, tools?, ops, on_unavailable?,
  budget_ms}]}` (`Declaration`), fetched with the plan's other items at
  admission. `params` defaults to `{}`; `preset` absent selects the default.
  `composition` is an opaque object, absent before composition exists.
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
  operations, not a new value: `prepend {block, text}`, `append {block,
  text}` or `replace {block, value}` (`Operation`), each with an optional
  short `note`. The runner applies them (`apply_ops`); each operation, with
  the module that returned it and its note, is the attribution record, so
  nothing is diffed.
- [pinned] Text blocks. A subject's `blocks` lists only its **text**
  blocks, in message order, and an operation's `block` (an unsigned 32-bit
  integer, required) indexes that list. Non-text blocks (images, tool
  calls, signed thinking, which must never change by a single byte) are
  neither listed nor addressable, and are left byte-identical
  (`apply_to_parts`).
- [pinned] An operation acts on its block alone. `prepend` and `append`
  concatenate onto that block's text and never create a new block;
  `replace` rewrites that block's text and never changes the number of
  blocks. No operation adds, removes or reorders blocks. A `block` that is
  not an index into `blocks` is a disallowed answer
  (`block_out_of_range`).
- [pinned] Several operations, and several providers on one hook, apply in
  order: prepends to one block stack outward (each lands before the text an
  earlier prepend added), appends stack in order, and a
  `replace` sees the block's text as it stands at that point.
- [pinned] Exclusivity. The session has exactly one reduction owner, its
  compaction provider: the only party that may remove or rewrite what was
  written. On `pre_user` and `post_assistant`, `replace` belongs to the
  reduction owner alone; without one, nobody has it (`check_reduction`).
  Every other step transform is preserving: it may prepend or append.
- [pinned] Order. The runner calls a hook's providers in the frozen plan's
  exact order (`hook_order`) and never moves an item, the reduction owner
  included.
- [pinned] The plan composer places the reduction owner, when it has a
  step-transform item, first among the step-transform items, so its
  `replace` runs before every preserving prepend or append and never wipes
  them (`reduction_owner_first`). This is the plan composer's obligation, which
  this contract records; providers do not enforce it, and a runner does
  not reorder a plan that breaks it.
- [pinned] `replace` on `post_tool` is a separate permission only the user
  tier grants, per provider and hook, as `{module, hook, tools}`
  (`UserGrant`): `module` is the provider, `hook` is `post_tool`, `tools`
  the tools whose results it may replace (an empty list allows none).
  Neither project configuration nor a provider can grant it, and no
  provider has it by default.
- [pinned] The grant travels in the plan, in the dedicated top-level field
  `user_grants: [{module, hook, tools}]` and nowhere else
  (`USER_GRANTS_FIELD`, `plan_user_grants`). The plan composer fills it
  exclusively from the user tier, and it is frozen with the plan at
  admission. Ordinary plan fields never carry a grant: a grant-shaped value
  anywhere else (an item's `params`, a subscription, the composition,
  another field name) grants nothing. A provider must ignore a grant it
  finds anywhere else, and a runner reads only `user_grants`.
- [pinned] A `post_tool` answer with a `replace` that no frozen grant
  covers for the answering provider and the call's tool is a disallowed
  answer (`check_post_tool_grant`, `replace_not_granted`).
- [pinned] A tool's catalog entry declares the operations it accepts on its
  result (`result_ops` in `tool-provider/v1`, all three by default). An
  operation the tool does not accept is refused.
- [pinned] An answer that is not allowed is refused before anything is
  applied or recorded (`check_answer`), never detected afterwards. A
  disallowed answer makes the hook unavailable for that call, exactly like
  a timeout or a failure, and the frozen subscription's `on_unavailable`
  decides (§7). It is never dropped and treated as `pass`, so a broken
  enforcing hook cannot fail open.

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
- [pinned] An approve hook passes or asks a human; it never denies directly.
- [pinned] The approve hook's question (`ApprovalAsk`) is `{prompt,
  options?, expires_at_ms, on_expiry, material_damage, late_execution}`:
  - `prompt` is a string;
  - `options` is an enumerated list of strings, the answers offered,
    omitted when empty;
  - `expires_at_ms` is when the question expires, an absolute time in
    milliseconds since the Unix epoch (an unsigned 64-bit integer), not a
    duration;
  - `on_expiry` is one of `deny` (`OnExpiry`): the call ends
    `pre_tool_expired`;
  - `material_damage` is a boolean: whether the call can cause damage that
    cannot be undone;
  - `late_execution` is one of `execute` or `notify_only`
    (`LateExecution`): what an approval arriving after the run moved on
    does.
  `on_expiry` and `late_execution` are closed: an unknown value does not
  decode, so the answer is disallowed and the hook unavailable, and nothing
  executes on a policy the runner does not understand. The elicitation role
  owns how the question is filed and answered. The runner files it with the
  final canonical arguments, target, schema pin and scope; none of those
  authority fields is supplied by the hook answer.

## 7. `transform.hook`

Optional host service, burn, part and pass fields are specified in §13.2–§13.3.

- [pinned] The request (`HookCall`) carries `session` (the runner's opaque
  name for the session), `harness` (the caller's harness from the session's
  key, required, §1), `lineage_id` (absent only on `pre_user` for the
  first message of a session nothing has been written to), the plan item's
  `preset` and `params` verbatim, and the subject, tagged by `hook`
  (`Subject`), flattened into the request, not nested under `subject`:
  - `pre_user {blocks, mark?, delivery?}`: `mark` is the sender's mark on a
    steered or queued prompt, opaque; `delivery` is `queue`, `steer` or
    `interrupt` for an owner's prompt, decoded open;
  - `post_assistant {step_id, blocks}`: the message's text blocks only,
    never reasoning, signatures or tool calls;
  - `pre_tool {step_id, phase, tool, tool_call_id, call_key?, input}`:
    `input` is the input that would execute, after every earlier mutate;
  - `post_tool {step_id, tool, tool_call_id, call_key?, blocks, is_error}`:
    the tool result's text parts; a result that is a single string is
    `blocks: [that string]`.
- [pinned] `blocks` is an array of strings, the text of each of the
  subject's text blocks in message order (§5), required on every text
  subject. `params` defaults to `{}`. `mark` is arbitrary JSON; every other
  subject id field is a string, `input` arbitrary JSON, and `is_error` a
  boolean. `preset` absent selects the default.
- [pinned] A hook may also carry `subject_mid` (the runner's message id,
  string), `subject_ordinal` (its position in the current lineage, `u64`),
  and `message` (the whole new message as opaque JSON in the runner's own
  schema). `subject_mid` and `subject_ordinal` come together or not at all;
  `message` requires both. These pairing rules are validation rules
  (`HookCall::check_host_fields`), not shape-decoding rules. A provider must
  validate before interpreting or ingesting the request and refuse a
  violation with `invalid_params`, naming `message` or the missing identity
  field in `detail.field`. An explicit JSON `null` is a present `message`
  and must satisfy the same rule. None of these fields changes which text
  blocks an operation may address (§5).
- [pinned] The opaque value `params.serializer_profile` names the schema
  of `message`; it is part of the existing params, not a separate contract
  field. The provider must not interpret `message` unless it recognises the
  profile. If it needs the supplied message and cannot read it, including
  because the profile is absent or unrecognised, it refuses the hook as
  misconfigured with `invalid_params {field: "params.serializer_profile"}`.
  A provider that can read the required history elsewhere may ignore the
  supplied message.
- [pinned] Within the conversation's current lineage, the provider ingests
  a supplied message at most once per `(lineage, subject_mid)`, including
  across its own crash. A retry or another hook point on the same message
  still gets a fresh hook answer (§3), but never another ingestion. A mid
  already seen with a different ordinal or different supplied `message`
  bytes is refused with `invalid_params {field: "subject_mid"}`, rather than
  overwriting the earlier input. Providers comparing messages must retain
  the supplied JSON bytes, not use parsed-value equality to hide a byte
  difference. Ingesting input does not make unused hook output durable or
  permit unused output to reach the model (§11).
- [pinned] A provider may enforce a 3 MiB (3,145,728 byte) request cap on a
  hook carrying `message` (`DEFAULT_HOOK_CAP_BYTES`,
  `HookCall::check_message_size`). All fields, including `message`, count:
  the size is the length of the compact JSON encoding of the `HookCall`
  request, not the `{method, params}` envelope. If exceeded, the provider
  refuses `invalid_params {field: "message"}` and never silently truncates.
  This optional cap imposes no new size policy on hooks without `message`.
  It is below the 4 MiB per-connection queue budget of subc (the local daemon
  that routes requests between runners and providers), which
  closes a connection whose queued bytes exceed it, so a runner should not
  send a hook larger than this cap even to a provider that does not enforce it.
- [pinned] The first hook of a lineage may carry
  `descends_from: {lineage_id, through_ordinal}` (`DescendsFrom`), naming
  another lineage in the same conversation (§1). It continues that
  lineage's history up to and including the `u64` `through_ordinal`, with
  the inherited prefix's message ids and ordinals unchanged; later history
  from the named lineage is not inherited. The provider must hold that
  prefix through the named ordinal. If it does not, it refuses the hook
  with `transient` and may include `detail.history_gap_from`, the first
  ordinal it lacks, so the runner can resend from there. The provider never
  guesses ancestry or missing history. An absent `descends_from` declares
  no inherited history. The hint is meaningful only on this role's
  retryable history refusal, `transient`, and ignored on other codes.
- [pinned] `tool_call_id` is the model's id: display only, not unique.
  `call_key` is the runner's key (`llm-runner/v1` §11.3); it is usually
  absent on `pre_tool`, because it is minted from the dispatch intent,
  written after the hook, and present on `post_tool`.
- [pinned] The answer (`HookAnswer`) is `{answer: "pass"}`, `{answer:
  "ops", ops}`, `{answer: "mutate", input, note?}`, `{answer: "deny",
  text}` or `{answer: "ask", ask}`.
- [pinned] A hook that times out (after the frozen `budget_ms`, capped by
  the runner), fails, or answers something it may not (§5) is unavailable
  for that call, and the frozen plan subscription's `on_unavailable` (§4)
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
  beyond the opaque `session` handle and the caller's `harness` (§1). A
  provider uses them only as keys for its own per-session state (a tag
  counter, say), never to choose a variant.
- [pinned] A provider tells prompts apart itself, by the mark or by their
  wrapping. The runner does not classify turns.

## 9. Error codes

Codes ride as the `code` of an `ERROR` frame's body `{code, message,
detail?}` (`ErrorBody`). They are open strings: a party that meets one it does not know
treats it as a terminal refusal of that one request.

`KnownCode` enumerates the refusal codes and tool-result reasons in §9 for
classification only; `ErrorBody.code` stays an open string. `message` is a
required string and `detail` arbitrary JSON, omitted when absent.

### 9.1 Provider refusals (`errors`)

| Code | When | Detail | Retry |
|---|---|---|---|
| `invalid_params` | a request field is malformed, misconfigured, over the optional whole-message request cap, or conflicts with an ingested subject | `field` | no |
| `not_subscribed` | a hook call for a hook and phase the provider did not declare for the call's preset and params | `hook`, `phase?` | no |
| `transient` | a condition that may clear by itself, including required history the provider does not hold | `history_gap_from?` for a history gap | **yes**, within the hook's budget |

- [pinned] Any refusal of a hook call makes the hook unavailable for that
  call (§7).
- [pinned] The malformed-request code is `invalid_params`, as runners
  answer; `tool-provider/v1` spells it `invalid_request`.

### 9.2 Codes the runner answers or writes (`errors::runner_codes`)

| Code | Where |
|---|---|
| `invalid_params {field: "plan.step_transform_items", item, subscription, problem}` | admission, for a malformed subscription or a `replace` the provider may not have; tools or ops beyond the declaration are instead stale |
| `invalid_params {field: "plan.user_grants"}` | admission, for a `user_grants` field that is present but not an array of `{module, hook, tools}` (§5) |
| `plan_stale {differences}` | admission, for a subscription the current declaration no longer covers: `subscription_missing`, `preset_missing` or `subscription_loosened` (§4) |
| `pre_user_unavailable` | a refusal of a steered or queued send, and a run's `provider_code` |
| `pre_tool_denied`, `pre_tool_unavailable`, `pre_tool_declined`, `pre_tool_expired`, `post_tool_unavailable` | a tool call's error result |

## 10. Decoding

Host-lane omission and legacy compatibility are specified in §13.5.

- [pinned] **Lenient on fields**: `role.describe`, the declaration request
  and answer, hook requests and answers. A newer runner can add fields
  without breaking an older provider, and the reverse.
- [pinned] **Strict on values** this role defines and a party must act on:
  `hook`, `phase`, `op`, `on_unavailable`, `answer`, and an ask's
  `on_expiry` and `late_execution`. An unknown value does not decode. On a
  request the provider refuses it `invalid_params`; on an answer the runner
  treats the hook as unavailable. Values a runner may grow (`delivery`) are
  decoded open.
- [pinned] Public structs that can grow optional fields are non-exhaustive,
  constructed with `new` and `with_*` setters. Empty optional collections
  are omitted unless the contract makes them required. Every operation
  carries a required unsigned 32-bit `block`; `replace` carries string
  `value`, `prepend` and `append` string `text`, each with optional string
  `note`.
- [pinned] `subject_mid`, `subject_ordinal`, `message` and `descends_from`
  default to absent and are omitted when absent. Their absence preserves
  existing request bytes and behaviour. `descends_from.lineage_id` is a
  required string and `descends_from.through_ordinal` a required `u64` when
  the object is present; unknown fields remain ignored.

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

## 13. Host runner lane

This lane is an explicit exception for OpenCode 1 (`opencode`), OpenCode 2
(`opencode2`) and the attested Claude Code gateway described in §14, not a
change to ordinary runner admission or observation.

### 13.1 Admission and observation

- [pinned] compaction-provider 0.3.0 removed `compaction.step`,
  `compaction.ready`, the request fence and `wait`, and its host runner lane
  (its former §18). The references to them below describe the
  compaction-provider/v1 0.2.x lane this section was written against; a
  compaction-provider 0.3.0 provider offers none of them. This lane is kept
  as written until it is re-specified on advance notices and views.

- [pinned] Admission is keyed only on the frozen plan item's params and the
  route's bind `(project_root, session, harness)` under the route's
  authenticated principal. For direct callers, the bind harness must be
  `opencode` or `opencode2` and the principal a `direct` caller bound to that
  conversation. The other admitted pair is daemon-attested
  `reserved:thalamus` on its own connection with a `claude-code` bind harness
  and the frozen `claude_code` capability bundle (§14). Neither the
  body `harness` field nor body params select this exception or confer
  authority. The §1 triple remains the canonical external conversation identity
  and the legacy-lane provider key. Claude Code provider state (engine namespace,
  locks, message ledgers, cursors, views and receipts) is additionally scoped by
  the frozen provider generation and profile generation. Params never select
  another canonical conversation or change its project/session/agent ownership.
- [pinned] For the direct OpenCode callers, the frozen plan opts in with `params.serializer_profile:
  "opencode-aisdk"`. Answer observation additionally requires
  `params.observation: "answer"`; both opt-ins and host admission are required
  for the promote-and-burn protocol below and the compaction departures in
  §13.4. Missing or unrecognised required opt-ins grant no exception. The
  same frozen params are passed on declaration, Setup and hooks.
- [pinned] Only an admitted caller in this section may use an empty runner-group set
  without `transcript_reads` or a `compaction.ready` service (removed in compaction-provider 0.3.0). This grants no
  runner-only operation, runner/Broca transcript read or other session.
  A cross-session request refuses `invalid_params`, naming `session` (or
  `project_root` for a project mismatch). Ordinary runners still undergo full
  runner admission, including their required groups and ready service; a
  runner-principal call with body `harness: "opencode"` and host params does
  not enter this exception.
- [pinned] The direct-caller exception is no stronger than today's local
  direct-caller trust for full-request transform. It requires the daemon caller-identity
  stamp once that stamp lands; body fields are never a substitute for it.
  It does not globally publish empty runner requirements in `role.describe`.
- [pinned] From a runner principal, either host-only plan value,
  `observation: "answer"` or `serializer_profile: "opencode-aisdk"`, on
  `transform.declare` refuses `invalid_params`, with `detail.field` naming
  `params.observation` or `params.serializer_profile`, respectively.
  `compaction.setup` had the same rule (compaction-provider/v1 §18.1, removed in compaction-provider 0.3.0).
  Authentication/admission is separate from the pure declaration (§4, §8).
- [pinned] Answer observation makes zero `session.read` calls, including
  restart and bootstrap. Ingest is not observation; a hook answer alone is
  never proof that its output was served. Missing acknowledgement never
  promotes pending output and never falls back to transcript reads.

### 13.2 Durable service and discarded subjects

- [pinned] `transform.hook` may carry `served_through_ordinal: u64`, an
  optional sibling in the request, not an answer member. It is the highest
  ordinal durably committed for serving on the call's lineage. Hooks in a
  pass carry the previous committed watermark; a compaction step carries the
  watermark after that pass's append transaction. Absence means no
  ordinal-confirmed promotion, neither zero nor the newest ordinal.
- [pinned] The host advances the watermark past an ordinal only after a model
  request carrying that ordinal has completed cleanly at the provider: the
  response status was 2xx, its final stream event arrived (for Anthropic,
  `message_stop`), and the connection ended with no transport error.
  Committing the ordinal before the request is sent, or on receiving response
  headers, never advances it. A request that fails, is cancelled or is cut off
  leaves the watermark where it was. A complete response counts as clean
  whatever its stop reason, a model refusal included, because the model read
  the request; a provider error status or an in-stream error event is never
  clean, even after a 2xx status.
- [pinned] Per lineage the watermark moves only forward, except for a revert
  clamp to the revert target in the same durable transaction. A lower value
  without that revert refuses `invalid_params {field:
  "served_through_ordinal"}`; it never un-promotes output. Promotion never
  passes the newest ordinal the provider holds for the lineage. Pending
  answers promote only through the watermark in the current lineage or an
  ancestor's inherited prefix; ancestor pending answers beyond the descent
  boundary burn. These rules survive retries and provider crashes.
- [pinned] `transform.hook` may carry `unserved_subjects:
  [{subject_mid: string, hook: Hook, subject_part?: string}]`, also optional
  and not an answer member. The enclosing request supplies the lineage.
  Entries require typed identity and the existing strict `Hook` discriminant;
  unknown entry fields are ignored. Host subjects are `pre_user`,
  `post_assistant` and `post_tool`; a tool subject carries its part id (§13.3).
  An absent or empty list names no burns; omit the empty list.
- [pinned] Burning is idempotent, and an entry naming a subject never answered
  is ignored. Burns precede promotion and cadence. Burned numbers never enter
  tags or rendering; queued reductions on burned pending tags are discarded.
  Later status ingestion of the raw message never resurrects discarded hook
  output. Validation, including held-history conflicts, precedes any
  promote-and-burn mutation.
- [pinned] After a timeout or partial failure the host freezes the whole
  message raw and reports every subject, including answered ones, until a
  later call answers. It clears the resend list only then. The same two
  optional fields occurred on `compaction.step`, not Setup
  (compaction-provider/v1 §18.2; both removed in compaction-provider 0.3.0).

### 13.3 Part identity, pass barrier and size

- [pinned] `transform.hook` may carry `subject_part: string` for a host
  `post_tool` subject. When present it is opaque, non-empty, at most 256 bytes
  in UTF-8, and compared byte for byte, without normalisation. An empty or
  over-limit value refuses `invalid_params {field: "subject_part"}`.
  The same rule applies inside `unserved_subjects`, with the field named
  `unserved_subjects[i].subject_part` for a malformed entry.
- [pinned] Host answer identity is `(lineage, subject_mid, hook,
  subject_part)`. `pre_user` and `post_assistant` omit the part; omission
  denotes the no-part slot, not a fabricated tool part id. `post_tool`
  supplies the actual part id, not the model's display-only `tool_call_id`.
  Distinct parts with identical call ids and output remain distinct answers;
  burning one does not burn the other. Every hook on a message still supplies
  the same opaque pre-op ingest bytes (§7), never an ops-modified clone.
- [pinned] `transform.hook` may carry `pass_complete: boolean`, true only on
  the pass's last hook and omitted unless true. Omitted or false schedules no
  barrier. It is a scheduling hint for off-request historian evaluation,
  never an acknowledgement and never a substitute for the durable watermark.
  State sync, including the historian model chain, precedes hooks and ingest;
  only the barrier schedules evaluation over the complete pass. A missing
  barrier after unavailable hooks is covered by the next pass's barrier.
- [pinned] A pass with no hooked append may place `pass_complete: true` on
  MC's internal `state_sync` request for a zero-append chain change. That use
  is MC-internal, not an op or field added to either commons role.
  `pass_complete` is not on `compaction.step` (removed in compaction-provider 0.3.0) or any provider answer;
  `subject_part` is not top-level on a compaction step or its answer.
- [pinned] Host encoded hook and status requests are capped at 3 MiB
  (3,145,728 bytes), counting all fields, including `unserved_subjects` and
  duplicate content in `blocks`. They are never truncated or sent over cap.
  The §7 hook cap measures the compact `HookCall`, not its transport envelope;
  the host's send check additionally budgets the encoded `{method, params}`
  request. An oversized host status entry exits with
  `provider_message_too_large`, never an oversized-single-entry paging loop.
  This does not replace ordinary runners' size policy.

### 13.4 Companion compaction departures

- [pinned] Only an admitted answer-observation host uses immediate status
  paging (compaction-provider/v1 §18.3, removed in compaction-provider 0.3.0): it re-pages only on `more: true`,
  immediately with a fresh `request_id` under the same request fence, not on
  ready or `bound_ms`. The provider releases the conversation lock before
  answering and spawns no scan. Incomplete history may answer `wait` only
  when `more: true`; an incomplete final page refuses `history_unreadable`
  with `detail.history_gap_from` naming the first known missing ordinal.
  `wait` on a final page is an unavailable call, never a retry loop. Absent
  `newest` means no written message, not a reason to scan.
- [pinned] Only that lane departs from compaction-provider/v1 §6's full
  suffix/final-value rule (§18.4, removed in compaction-provider 0.3.0). `after_ordinal` is the highest contiguous
  ordinal whose messages have every hook answered (`ingested = true` in
  host records); later `messages` contains only entries not fully ingested,
  oldest first, as opaque pre-op JSON. An unavailable/frozen-raw message is
  sent even if some hooks already ingested it. No wire `ingested` member is
  added. A missing cursor is the first status, not proof of held history;
  `messages: []` with `after_ordinal == newest.ordinal` is valid after hooks
  supplied all entries. Gaps are detected from held history, not list
  sparsity, and conflicts refuse before promote-and-burn (`subject_mid` for
  changed input under a held mid, `subject_ordinal` for another mid at a held
  hook ordinal, `messages` for conflicting step entries).

### 13.5 Compatibility

- [pinned] The new fields default to absent and are omitted when absent, not
  encoded as `null`. Older decoders ignore them; that is not semantic host
  admission. Existing strict discriminants stay strict. `owned-broca` and
  absent host opt-ins retain declaration, tag/cadence, subject identity,
  historian scheduling, observation and answer bytes even with valid extra
  fields; no host promotion, burning or barrier runs there. Pi and OMP keep
  their existing paths. Rust mode does not require Broca to run.
- [pinned] The request fence, version high-water, structural checks,
  Setup-once rule (including on descent), frozen budgets and
  `on_unavailable` are unchanged. These fields add no operation or answer
  discriminant. `host-runner-lane.json` in the existing vector directory pins
  wire examples and stateful expectations separately; older wire decoding
  alone cannot verify the host protocol.

## Not in v1

- Subjects other than text: images, structured tool results.
- A deny from `pre_user` or `post_tool`. Only `pre_tool` denies.
- A provider stopping a run. A caller that wants a session stopped after a
  deny cancels the run itself.
- A rule language. A rules engine is a module that answers hooks.
- Hooks on reasoning, signatures or tool-call blocks.

## 14. Optional native Claude Code capability

### 14.1 Publication and admission

- [pinned] compaction-provider 0.3.0 removed its native Claude Code lane
  (its former §19), including the shared `claude_code` bundle on Setup,
  `raw_view`, `compaction.view.read` and its receipts, together with
  `compaction.step`. The references to them below describe the
  compaction-provider/v1 0.2.x lane; a compaction-provider 0.3.0 provider
  offers none of them. This capability is kept as written until it is
  re-specified.

- [pinned] `role.describe`, `transform.declare` requests and declarations gain
  optional typed `claude_code: ClaudeCodeCapabilities`. Every existing member
  keeps its meaning. Absence preserves the existing behavior and bytes for
  all runners; absence off this lane is never a refusal. Discovery alone is
  not permission to use the lane. An attested caller that does not select the
  capability also retains the existing behavior.
- [pinned] Activation requires BOTH the daemon-attested principal
  `reserved:thalamus` on its bound connection AND the frozen declaration's
  `claude_code.capability: "claude-code-step/v1"`. A declaration selecting this
  bundle from any other principal refuses `invalid_params`, with
  `detail.field: "claude_code"`, at admission/declare. The bind must resolve the
  project, session and agent, with `harness=claude-code`; the bundle's harness,
  principal and profile are compatibility data, never authentication. Body
  `harness`, params, tags and plan values cannot supply the attestation. Another
  connection cannot borrow the admitted connection's bundle. There is no
  authority inherited from another gateway or harness identity.
- [pinned] Select only a supported advertised bundle before managed traffic;
  echo it unchanged at declaration, freeze it in the admitted plan and pass it
  unchanged to compaction Setup. Bundle drift refuses `invalid_params` naming
  `claude_code`. An absent or unsupported capability required by a proposed
  managed attachment fails attachment by name (`invalid_params`, or
  `misconfigured` when missing or inconsistent provider settings prevent the
  required capability from working, such as an advertised message format with
  no enabled encoder/decoder). It never enables legacy whole-array transform
  or raw fallback. `ClaudeCodeLane` is non-wire state:
  its admission and per-call checks take trusted transport identity. Providers
  must additionally enforce the bound conversation, profile and observation.
- [pinned] The bundle contains `capability`, admitted `principal`, `harness`,
  `profile`, `profile_generation`, `provider_generation`, `observation`,
  `commons_revision`, `wire_majors`, `package_versions`, `epochs`,
  `exact_endpoint`, `partial_message_cut`, `caps`, `raw_view_read`, `digest`,
  `cache_policy_revision`, `canonical_reduce`, `guidance`, `notice_dispatch_ack`,
  `bootstrap_complete` and optional further `capabilities`. The `epochs` object
  MUST contain `codec`, `raw_provenance`, `render`, `tagger`, `memory` and
  `compartment`, even when tagging is inactive. Missing any component refuses
  `invalid_params` naming `claude_code`; a `render` epoch is not a substitute
  for component pins. Actual supported values must match provider discovery.
  The `tagger` epoch names the tagging rules: 1 for tags recorded once and
  never reused (llm-runner before 0.4.0), and 2
  (`claude_code::COMPUTED_TAGGER_EPOCH`) for tags computed from the
  transcript (llm-runner 0.4.0's `runner-tags/v2`, its §9.6). A provider
  that tags by the computed rules advertises 2 or later.
  No codec epoch is implied by this contract or by synthetic vectors.
  Profile/provider generations are separate data fields, not session-id edits.
  `bootstrap_complete` advertises support for the typed completion receipt in
  compaction-provider/v1 §19.7 (removed in compaction-provider 0.3.0). The format identifier belongs inside that
  receipt, not in the discovery bundle. Setup ready or compaction noop alone
  never proves bootstrap complete. Publication remains fenced until complete
  native source admission and safe view validation. Durable memory ownership
  is unchanged.
- [pinned] Capability identifiers, including the bundle identifier, further
  capability identifiers and a completion receipt's format tag, use the daemon grammar
  `<name>/v<N>`: ASCII lowercase letter first, lowercase letters/digits/hyphens,
  at most 64 name bytes, no adjacent hyphens and no trailing hyphen. `N` is
  decimal 1 through 4294967295 without leading zeros. Whitespace is invalid.
  Epochs, revisions and profile generations never enter capability names.
  Existing capability identifiers and runner-group strings keep their meaning.
- [pinned] `observation` is `answer`, `exact_endpoint` is
  `native_top_level_inclusive` and `digest` is `sha256`. Partial cuts are a
  separately advertised boolean, never inferred from the presence of an
  endpoint. An initial whole-message-only gateway advertises
  `partial_message_cut: false`. Reduction and guidance flags describe the
  canonical, actually served tool/system surfaces frozen in the plan; tags
  alone never grant reduction authority. Cache-policy revision is a pin, not
  a second cache expiry scheduler.

### 14.2 Host departures and payload limits

- [pinned] The admitted pair uses the §13 answer-observation protocol: no
  transcript reads, including restart/bootstrap, and no `compaction.ready`
  service requirement (removed in compaction-provider 0.3.0). It may use an empty runner-group set locally, without
  globally changing `role.describe.runner_groups`. Ingress `more=true` fences
  view execution and immediately requests the next contiguous page with a
  fresh request ID. This `compaction.step` paging (removed in compaction-provider 0.3.0) admits original messages;
  `compaction.view.read` separately retrieves frozen replacement bytes.
  A final-page history gap refuses `history_unreadable`, never an unbounded
  wait or transcript read.
  Ordinary runner/group admission and the direct OpenCode exception are
  unchanged.
- [pinned] `caps` declares `ingress_envelope_bytes`, `hook_envelope_bytes`,
  `single_message_bytes` and `view_page_bytes`. Full encoded ingress/hook
  envelopes, including raw bytes' JSON escaping and sidecar overhead, must
  be strictly below their declared envelope caps and below 3 MiB of headroom
  (`MAX_ENVELOPE_BYTES`, the same `DEFAULT_HOOK_CAP_BYTES` constant). Caps must
  be positive; envelope caps cannot exceed that constant, and the single
  message cap must be smaller than each envelope cap. `view_page_bytes` caps
  raw bytes per page and must be smaller than the constant; encoded page
  envelopes remain strictly below it as well.
- [pinned] A single message that cannot fit refuses `invalid_params` naming
  `messages` on a status or `message` on a hook. The host terminal classification
  is `provider_message_too_large`. Never truncate, raw-forward or use the
  ordinary compaction paging helper's oversized-first-message behavior on this
  negotiated lane. Larger ingress requires a separately negotiated chunked
  immutable payload capability, not view retrieval. None is implied here.
- [pinned] Native endpoint/raw replacement retrieval, required geometry and
  notice dispatch acknowledgements follow compaction-provider/v1 §19 (removed in compaction-provider 0.3.0). The
  `pass_complete` scheduling barrier and ordinal service watermark are not
  notice acknowledgements and do not prove HTTP dispatch. A pending notice is
  never armed off the negotiated, attested lane; a plan requesting the notice
  capability without that attestation refuses `invalid_params` naming
  `claude_code`, and an off-lane directive refuses naming `channel2_directive`.

### 14.3 Native raw-message sidecars and generation scope

- [pinned] `HookCall` gains optional `raw_message: String`, also carried by each
  compaction `StatusMessage`. Absence off the admitted Claude Code lane keeps
  legacy bytes and behavior unchanged. On the admitted lane every native hook
  requires a nonempty sidecar that parses as JSON; absent, empty or invalid
  JSON refuses `invalid_params` naming `raw_message`. Validation does not
  rewrite its bytes. Both the raw sidecar and its JSON escaping count toward
  the existing single-message and full-envelope caps.
- [pinned] `raw_message` is the message's JSON bytes exactly as the harness
  (Claude Code) sent the message to the runner, minus `cache_control` removed
  only at Anthropic transport positions.
  Those positions are the top level of `text`, `image`, `document`, `thinking`,
  `redacted_thinking`, `tool_use` and `tool_result` blocks, and of `text`, `image`
  and `document` blocks nested in `tool_result.content`. Never remove members
  inside `tool_use.input`, and never inside opaque data. Preserve every other
  byte, including member order, whitespace, string escapes, scalar content,
  signatures and redacted thinking payloads.
- [pinned] The provider MUST verify that `raw_message`, with `cache_control`
  removed only at the listed positions, equals `message`, and refuse
  `invalid_params` naming `raw_message` if not. The common helper checks only
  presence and JSON syntax; native equivalence, block spans and signed-byte
  safety are provider codec obligations, not permission for generic recursive
  cache-marker stripping.
- [pinned] The canonical external identity remains the project/session/harness
  triple with resolved agent ownership. Claude Code provider state, including
  namespace, locks, ingest ledgers, views and receipts, is additionally scoped
  by provider generation and profile generation. A new native generation never
  overwrites old state or changes memory ownership; bootstrap resumes within
  the same generation and lineage. Its typed final-ingress completion receipt,
  source evidence and conflict/replay rules are compaction-provider/v1 §19.7 (removed in compaction-provider 0.3.0).
