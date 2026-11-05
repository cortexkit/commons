# `step-transform-provider/v2` — role contract

Stability: **alpha**. This document defines the role. The Rust types in this
crate are its wire shapes; `test-vectors/step-transform-provider-v2/` holds
its vectors. Every **[pinned]** item is a binding requirement.

A step-transform provider makes write-time changes to the newest message of
a session through four hooks. `llm-runner/v1` defines the runner's call-site
obligations; this document defines the provider's side. Shared runner error
codes come from that role's crate so their spellings cannot drift.

## Terms

- **Plan composer**: writes and freezes the session plan, checks runner groups
  and subscribed hooks, orders providers and fills user-tier fields.
- **Runner group**: a capability group in a runner's `role.describe`, such as
  `transcript_reads`. Providers list requirements in `runner_groups`.
- **Reduction owner**: the session's compaction provider, the only party that
  may remove or rewrite history already written. On `pre_user` and
  `post_assistant`, non-owners may only prepend or append. `post_tool`
  replacement is governed separately by user grants; `pre_tool` answers
  are governed by phase.
- **Signed thinking**: reasoning blocks carrying a model provider signature;
  changing even one byte invalidates it, so hooks never edit these blocks.

## 1. Role identity and addressing

- [pinned] A provider lists `step-transform-provider/v2` (`PROVIDES`) in
  `capabilities.provides`. `VERSION` is `v2`. Required ops (`REQUIRED_OPS`):
  `role.describe`, `transform.declare`, `transform.hook`. A consumer refuses a
  provider missing any required op, by name, before calling it.
- [pinned] There is one hook route per session and role,
  opened under the session's scope and bound to `(harness, session)`, with
  `role_versions` selecting role `step-transform-provider`, version `v2`.
  Every hook concerns only that route's session. The daemon attests the
  caller module; the project comes from its scope stamp (`workdir`, and
  `project_id` when present). No request body carries caller identity.
- [pinned] A consumer that opens another route to the same module for the
  same session, such as a tool route or a compaction route, binds it with
  the same `(harness, session)` as the hook route, so the module sees one
  session (the tool-provider contract's one-bind-identity rule, §1.1). The
  pair is unique only within one consumer, so a provider keys session state
  by the route's attested principal together with the pair. A module-level or
  pre-session route, such as one used for `transform.declare` before the
  session exists, binds no session and isn't covered.
- [pinned] `role.describe` and `transform.declare` are answerable on **any
  route of this role**, including module-level routes and scoped routes
  opened before the session exists. Session binding is not a bind-time
  requirement. A hook on any route other than a scoped session-bound route
  is refused **per call** with `route_unbound`, message `Expected a bound
  session`. It does not close the route or prevent later preflight calls.
- [pinned] Caller attestation and the selected role are checked at bind,
  using these refusal codes and messages:
  `unattested_runner` (`No caller module attested`) when the daemon attests
  no caller module; `unknown_route_role` (`Expected exactly one served role`)
  when the route declares none of the roles its receiver serves, or more
  than one. These are route opening/binding refusals, not hook-body checks.
  This role permits non-session routes for preflight and checks session
  scope and binding per hook call, as specified above.
- [pinned] Per-session state is keyed on `(runner, harness, session)`: the
  daemon-attested runner's **bare module id**, never a display prefix, and
  the route's bound harness and session. Folder spellings and scope metadata
  are not state-key components; aliases reach the same state. Changing any
  key component reaches a different conversation. The shared
  `cortexkit_role_llm_runner::session_ref(&SessionIdentity { runner, harness, session })`
  helper uses those fields without normalizing them. This role has no Setup
  op; the first hook can establish provider state.
- [pinned] Requests use `{method, params}` (`OpRequest`). The method selects
  the op. `role.describe` takes empty params (`DescribeRequest`). Route
  metadata is never reconstructed from a request's JSON fields.

## 2. `role.describe`

- [pinned] The answer is `{majors: [{version, ops, stability}],
  implementation_version, capabilities, runner_groups?}` (`RoleDescribe`),
  decoded leniently; only the `step-transform-provider/v2` major and its
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

`role.describe` and `transform.declare` are preflight operations, not session hooks.

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
- [pinned] The plan composer can call `transform.declare {preset,
  params: {}}` on a module-level route before any hook route exists, also
  for a worker whose session does not exist yet. It reads `hook`,
  `on_unavailable` (with the default below) and `budget_ms` from every
  declared subscription, and includes the item only when the runner's
  `step_hooks` cover **every** subscribed hook. No session context is required
  to answer this request.
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
  declaration or plan naming `refuse` there is malformed. The plan composer
  resolves the declaration's defaults and freezes policy and budget values
  equal to or stricter than those bounds. Admission validates, but does not
  rewrite, those frozen values. The runner uses them, capping the budget
  with its own engine-wide limit.
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
    This is a difference only for a named preset. A presetless item for a
    provider without a default is not stale: it is refused
    `invalid_params {field: "preset"}`, and planners must leave such an item out.
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
- [pinned] Exclusivity. The session has at most one reduction owner, its
  compaction provider: the only party that may remove or rewrite what was
  written. On `pre_user` and `post_assistant`, `replace` belongs to the
  reduction owner alone; without one, nobody has it (`check_reduction`).
  On those two hooks, non-owners may only prepend or append. `post_tool`
  replacement is governed separately by user grants; `pre_tool` answers
  are governed by phase.
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
  decides (§7). Never interpret a disallowed answer as a successful `pass`.
  Mark the hook unavailable and apply its frozen availability policy;
  proceeding under an explicitly configured `on_unavailable: pass` remains
  governed by §7 and the other gates.

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
  - `late_execution` is `execute` or `notify_only` (`LateExecution`): whether
    an approval arriving after the run moved on executes the call or only
    notifies the person that it arrived too late.
  Unknown approval policies supply no execution permission: the answer is
  disallowed and the hook unavailable. Proceeding under an explicitly
  configured `on_unavailable: pass` remains governed by §7 and the other
  gates. The elicitation role
  owns how the question is filed and answered. The runner files it with the
  final canonical arguments, target, schema pin and scope; none of those
  authority fields is supplied by the hook answer.

## 7. `transform.hook`

- [pinned] `HookCall` carries the plan item's `preset?` and `params` verbatim
  and a subject tagged by `hook`, flattened into the request:
  - `pre_user {blocks, mark?, delivery?, at_ms?}`: `mark` is opaque JSON,
    including the sender's `{kind, from}` provenance on a steer or late
    result; `delivery` is an open string, normally `queue`, `steer` or
    `interrupt`, absent for a person's message. Providers retain both to
    distinguish human messages, steers, queued prompts and late results.
    `at_ms` is the new message's time in milliseconds since the Unix epoch
    (UTC), taken on the runner's machine: the host's own creation time when
    the host stamps the message, otherwise the runner's clock when it calls
    the hook. A runner that can give it should, so a provider measuring the
    gap since the previous message reads both sides from one machine. When
    it's absent, the provider uses its own clock.
  - `post_assistant {step_id, blocks}`: only existing text blocks, never
    reasoning, signatures or tool calls;
  - `pre_tool {step_id, phase, tool, tool_call_id, call_key?, input}`:
    `input` is what would execute after every earlier mutate;
  - `post_tool {step_id, tool, tool_call_id, call_key?, blocks, is_error}`:
    the result's text parts; a single string is a one-element `blocks`.
- [pinned] `blocks` is an array of strings in text-block order. Every text
  subject requires it. `step_id`, `tool`, `tool_call_id`, `call_key` and
  `preset` are strings; `input` is arbitrary JSON; `is_error` is a boolean.
  `params` defaults to `{}`; absent `preset` selects the provider's default.
  Optional `runner_trace` is opaque JSON on **every** hook, never identity
  or authority. Omission is distinct from an explicitly supplied JSON null.
- [pinned] Optional `subject_mid` (stable message id, string) and
  `subject_ordinal` (`u64`) come together or not at all. Optional `message`
  is the whole new message in the runner's own schema and requires both.
  Explicit null is a present message. These are validation rules
  (`check_subject_fields`): violations are `invalid_params`, naming
  `message` or the missing identity field. None changes the addressable
  text blocks. `subject_part`, when supplied, is an opaque part id, nonempty
  and at most 256 UTF-8 bytes, never the model's tool call id; invalid parts
  name `subject_part`.
- [pinned] Subject ordinals follow [compaction W4.1/W4.2](../cortexkit-role-compaction/CONTRACT.md): count from **1** at
  the session's first entry, including entries omitted by a cut, result-only
  records and dead/empty entries. Markers are not entries. After a revert or
  branch switch, a replacement takes the removed entry's ordinal. Count the
  host's records exactly as W4.2 specifies, not text blocks or tool parts.
  `subject_ordinal: 0` is `invalid_params {field: "subject_ordinal"}`.
  Message ids and ordinals are independent; an id such as `m0` can name
  ordinal 1. Derive neither from the other.
- [pinned] `params.serializer_profile` names the opaque `message` schema.
  Before interpreting it, the provider must recognize the profile. If it
  needs that message and cannot read it, including an absent or unknown
  profile, it refuses `invalid_params {field: "params.serializer_profile"}`.
  A provider able to read required history elsewhere may ignore `message`.
  Supplied input never proves that a previous hook answer was used.
- [pinned] A provider may enforce a 3 MiB (3,145,728 byte) cap on a hook
  carrying `message` (`DEFAULT_HOOK_CAP_BYTES`, `check_message_size`). All
  request fields count, including trace and message: measure the compact
  JSON `HookCall`, not the `{method, params}` envelope. Over-cap requests
  get `invalid_params {field: "message"}`, never truncation. Hooks without
  `message` acquire no size policy from this rule. The cap leaves room below
  the daemon's 4 MiB per-connection queue budget.
- [pinned] `tool_call_id` is the model's display id, not unique. `call_key`
  is the runner's dispatch key; usually absent on `pre_tool` because it is
  minted after the hook from the dispatch intent, and present on `post_tool`.
- [pinned] Answers (`HookAnswer`) are `{answer: "pass"}`, `{answer: "ops",
  ops}`, `{answer: "mutate", input, note?}`, `{answer: "deny", text}` or
  `{answer: "ask", ask}`, constrained by §§5–6.
- [pinned] Timeout after frozen `budget_ms` (capped by the runner), failure,
  refusal or disallowed answer makes the hook unavailable for that call.
  Frozen `on_unavailable` decides:
  - `post_assistant`: pass unchanged and record that;
  - `pre_tool` under `refuse`: deny `pre_tool_unavailable`; under `pass`,
    execute the input;
  - `post_tool` under `refuse`: withhold the result, write an error with
    reason `post_tool_unavailable` naming the provider, saying the call
    executed;
  - `pre_user` under `refuse`: end the run `error` with provider code
    `pre_user_unavailable`; refuse a steered/queued send and write nothing.
  Under `pass`, continue without that hook's output and record that.

## 8. Purity, identity and unknown values

- [pinned] Declarations depend only on preset, params, provider configuration
  and composition. They never vary with caller, project, scope or session.
  Route identity is only a state key, never a selector for a declaration.
- [pinned] Providers distinguish prompts by their mark, delivery or wrapping;
  the runner does not classify turns for them.
- [pinned] Unknown fields are ignored. Open wire enums retain unknown strings,
  or an unknown tag and all its JSON fields. Malformed **known** variants
  still fail decoding: a missing required field cannot fall through to an
  unknown variant. Receivers log unknown values they encounter and apply
  these safe fallbacks; decoding does not imply support for their meaning:
  - unknown hook, phase, declared/planned op or `on_unavailable`: refuse
    `invalid_params`; an unknown planned value is `unknown_value` in the
    subscription problem detail, never a matching permission;
    an unknown hook in a user grant is `invalid_params` naming
    `plan.user_grants`;
  - unknown answer, operation, `on_expiry` or `late_execution`: disallow the
    entire answer before any mutation or recording; the frozen availability
    policy decides, never an implicit pass;
  - unknown `delivery` or `mark` content: preserve as opaque provenance,
    never infer that it is a person's message or grants authority;
  - unknown `plan_stale` kind or loosened field: keep it as an opaque stale
    difference; the plan still requires recomposition;
  - unknown majors, ops, capability names and stability levels in discovery:
    ignore unsupported additions, while still requiring v2 and its ops.


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
| `invalid_params` | a request field is malformed, misconfigured, or over the optional whole-message request cap | `field` | no |
| `not_subscribed` | a hook call for a hook and phase the provider did not declare for the call's preset and params | `hook`, `phase?` | no |
| `unattested_runner` | the daemon attests no module for the caller | none | no |
| `unknown_route_role` | the route declares none of the roles its receiver serves, or more than one | none | no |
| `route_unbound` | a hook call is not on a scoped session-bound route; never a bind-time refusal | none | no |

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

- [pinned] Unknown fields are ignored in discovery, declaration, subscriptions,
  hook requests and answers. Extensible enum values decode losslessly with
  the safe semantic fallbacks in §8. Codes remain open strings; an unknown
  error code is a terminal refusal of that request. Human `message` text is
  not a machine-readable discriminator; structured information goes in detail.
- [pinned] Public growable types are non-exhaustive and have constructors.
  Absent optional fields are omitted. Every known operation requires unsigned
  32-bit `block`; prepend/append require string `text`, replace string `value`,
  each with optional string `note`. Text-block indexes are zero-based; they
  are not entry ordinals.


## 11. Crash and durability guarantees

What a provider owes across its own crash. A full crash suite must check
these guarantees by killing the provider and restarting it on the same state root
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

## 12. Conformance

The non-default `conformance` feature exposes focused probes over an
implementation-specific adapter, not a complete live-provider suite. Adapters
must use real daemon-attested routes and provider state, not reconstruct
markers from input identity. A fresh store is required for state probes.

| Case | Checks |
|---|---|
| `conversation_state_ignores_project_root` | with fixed harness/session, folder aliases reach the same state and another attested runner reaches different state |
| `hook_on_non_session_route` | both non-session route kinds bind successfully; hooks return error code `route_unbound`; a subsequent discovery RPC succeeds |
| `declare_on_module_route` | module-level and pre-session scoped routes answer `transform.declare {preset, params: {}}` and `role.describe` |

Each probe has an intentionally faulty implementation that the probe rejects
with its case name. The hook route probe does not check refusal wording or
discovery completeness; the declaration probe validates discovery shape and
required ops. A future full provider suite should additionally check
pure declarations, phase answers, text-only edits, fresh retries, unused-state
isolation and a real process kill at `HookStateRecorded`. Runner-side bounds,
tiers, answer validation and ordering are checked against this crate's vectors.

## Changes from v1

v2 moves session identity from hook bodies to per-session scoped routes and
changes the capability to `step-transform-provider/v2`. It removes `session`,
`harness`, `lineage_id` and `descends_from` from bodies. v2 has no lineage or
ancestry catch-up protocol, so `transient`/`history_gap_from` are removed too. It
removes answer-observation watermarks (`served_through_ordinal`),
`unserved_subjects`, promote/burn and `pass_complete`, and removes the native
capability bundle (`claude_code`), `claude-code-step/v1`, its special admission
and `raw_message`. These protocols served standalone plugins; session-bound routes and the
compaction protocol now supply identity and history management instead.
Subject ids, part ids, opaque messages, prompt provenance and all four hooks
remain; entry ordinals now follow compaction's 1-based numbering. Unknown enum
values decode losslessly without relaxing the safe refusal boundary.
