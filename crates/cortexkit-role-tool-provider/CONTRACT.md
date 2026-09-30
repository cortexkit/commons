# `tool-provider/v1` — role contract

Stability: **alpha**. This document is the role definition; the Rust types in
this crate are its wire shapes, `test-vectors/tool-provider-v1/` at the
repository root holds its vectors, and `cortexkit-role-tool-provider-conformance`
is its suite. Every item below is **pinned**: a provider must do it, and a
consumer may rely on it. The section "Choices to confirm" at the end lists
the few details this revision had to fix itself because no decision covered
them; each is pinned as written until the role owner confirms or changes it.

A tool provider is any module that serves tool definitions and executes calls
to them. Nothing here names a particular implementation.

## 1. Role identity and addressing

- A provider lists `tool-provider/v1` in its manifest's `capabilities.provides`
  (`PROVIDES`). That grammar covers roles only; a tool's capability tags never
  go in the manifest.
- Required ops: `role.describe` and `tool.catalog` (`REQUIRED_OPS`). A
  provider that can hold a call past its own reply also serves
  `tool.withdraw`. A provider that settles calls later also serves
  `late_results` and `late_results.ack`.
- Every role op is a named request on the provider's tool route,
  `{name: <op>, arguments: {...}}`, exactly like a tool call. Role ops are
  never model tools: no catalog lists them.
- A consumer refuses a module whose `role.describe` lacks a required op, by
  name, before routing anything to it.

## 2. `role.describe`

- The answer is `{role, versions, stability, implementation_version, ops,
  capabilities}` (`RoleDescribe`), with `role: "tool-provider"`. `versions`
  lists every major of the role the module serves (`["v1"]`,
  `["v1", "v2"]`); the caller picks the highest it understands for a new
  session.
- Decoded leniently: unknown fields, ops and capabilities are ignored; only
  the role, `v1` among the versions, and the required ops are checked
  strictly. `capabilities` may be omitted. `stability` is `alpha`, `beta` or
  `stable`; an unknown level decodes.
- The answer describes the implementation, never the tools: it carries no
  tool list. It depends only on the module build, so a consumer may cache it
  for as long as it talks to the same module incarnation; two answers from one
  incarnation are identical.
- `tool-provider/v1` defines no module-level capabilities. Session-level
  capabilities travel in the catalog answer.

## 3. `tool.catalog`

- Request: `{params, composition?, system_text?, digest_only?}`
  (`CatalogRequest`).
  - `params` is the plan item's params: the shared vocabulary (`behavior`,
    `scope`, `tool_descs`, `exclude`) plus the provider's own axes. An unknown
    value is refused, never guessed.
  - `composition` is the session's composition, carried verbatim as an opaque
    JSON object. Providers never interpret it beyond resolving their own text
    against it. Absent on a preflight call.
  - `system_text` is `{preset, params}` when the plan has a system-prompt item
    for this provider.
- Answer (`CatalogAnswer`): `{generation, composition_digest?, tools: [...],
  system_text?, capabilities?}`.
  - `generation` is an opaque string that changes whenever the catalog's
    content changes.
  - `capabilities` is a top-level object of session-level capabilities, keyed
    by name, each declared with the value `true`: `host_params`,
    `late_results`. The runner freezes them with the tools.
  - `system_text`, when requested, is `{text, item_digest, preflight_digest,
    composition_digest}`, from the same configuration resolution as the
    catalog in the same reply.
  - A provider with only system text answers an empty `tools` list.
- `digest_only: true` answers `{generation, catalog_digest}` with no tools.
- Each tool (`CatalogTool`): `{name, schema_digest, semantics, result_ops?,
  capabilities, description?, input_schema}`.
  - `name` is the exact name the user tier disables the tool by.
  - `schema_digest` is the digest of `input_schema`, compared for equality
    only. Two answers from the same inputs carry the same digest.
  - `semantics` is an integer, bumped whenever the tool's behaviour changes
    without a schema change.
  - `result_ops` lists the hook result operations the tool accepts, from
    `prepend`, `append`, `replace`. Absent means all three.
  - `capabilities` lists capability tags: unprefixed (`code.outline/v1`,
    defined only by this document) or namespaced (`acme:code.callgraph/v1`,
    free for anyone). Data in this answer only.
  - `input_schema` is the argument schema; `description` its description.
- The answer is a pure function of its inputs (the plan item, the
  composition, user and project configuration, host facts). Scope, owner and
  agent on the route are attribution only. Tool names and schemas never depend
  on the composition; descriptions and system text may. The same inputs give
  the same bytes.
- A tool the user or project tier disables is absent from the answer.
- Argument schemas are flat (`check_flat_schema`): a JSON object with no
  root-level `anyOf`, `oneOf` or `allOf`, and no root-level `type` array (a
  type array is a union). Unions below the root are allowed.
- A host-only parameter carries `"x-ck-audience": "host"`, is always
  optional, and is refused in a call from the model.

## 4. Calls

- A call is a route `REQUEST` whose body is subc-protocol's
  `ToolCallRequest`: `{name, arguments, tool_call_id?, progress_token?,
  call_key?, schema_pin?}`.
- `call_key` (subc-protocol 0.26.0) is opaque, 1 to 256 bytes, each printable
  ASCII from 0x21 to 0x7E; space is not allowed. A present but malformed key
  is refused as `invalid_request` with `detail: {field: "call_key"}`. A call
  without a key executes but has no deduplication and no withdrawal, by
  construction; nothing is added to its reply.
- Every record shared between parties keys on `(carrier principal,
  call_key)`. The model's `tool_call_id` is display-only.
- `schema_pin` (subc-protocol 0.27.0, top-level, omitted when `None`) has the
  same byte bound as `call_key`. Its content is this role's: the tool name,
  the catalog `generation` the runner froze, and the tool's `semantics`,
  encoded canonically as **`tp1`** (`SchemaPin`):

  ```text
  tp1:<pct(tool)>:<pct(generation)>:<semantics>
  ```

  `pct` keeps the RFC 3986 unreserved bytes (`A-Z a-z 0-9 - . _ ~`) and
  writes every other byte of the UTF-8 string as `%` plus two uppercase hex
  digits, so `:` inside a component is always `%3A`. `semantics` is decimal,
  with no sign and no leading zeros. Tool and generation are non-empty.
  Exactly one string encodes each pin; anything else is refused. A pin that is
  malformed, or names a tool other than the call's `name`, is refused as
  `invalid_request {field: "schema_pin"}`. This crate mirrors the 0.27.0
  field and bound locally until that release publishes (`call.rs`).
- A call whose pin the provider can no longer honour is refused
  `tool_schema_changed {tool, expected, current}` or, when only `semantics`
  moved, `tool_semantics_changed {tool, expected, current}`.
- Every request gets exactly one terminal frame (`RESPONSE`, `STREAM_END` or
  `ERROR`), and it is the last frame: on success, refusal, cancel, drain, and
  when the provider stops tracking the request.
- Refusal codes: `unknown_tool {tool}` for a tool the provider does not serve;
  `tool_disabled {tool}` for a disabled tool; and, recorded by the runner as
  the call's error result and never re-dispatched, `tool_schema_changed`,
  `tool_semantics_changed`, `tool_unavailable {tool, reason: retired |
  under_review}` and `capability_not_admitted`.
- Model-shaped arguments are coerced before anything records, hashes or
  checks them. Scope, owner and agent are never arguments; they come from the
  route's stamp.

## 5. `tool.withdraw`

Served by a provider that can hold a call past its reply.

- The request is `{name: "tool.withdraw", arguments: {call_key, carrier?,
  scope?}}` (`WithdrawArguments`). It has no top-level `call_key`; one that is
  present is refused as `invalid_request {field: "call_key"}`. A malformed
  `arguments.call_key` is refused as `invalid_request {field:
  "arguments.call_key"}`.
- The record is keyed `(carrier, call_key)`; the carrier is the principal
  that raised the call.
- Caller rules (`resolve_carrier`, `check_record_scope`). Each failure is a
  **route error**, never an answer, and terminal for the caller:

  | Situation | Route error |
  |---|---|
  | `arguments.scope` present and not the route's stamped scope | `withdraw_scope_mismatch` |
  | a caller other than the scope's owner names a carrier other than itself | `withdraw_carrier_mismatch` |
  | the scope's owner names no carrier and holds no call of its own under the key | `withdraw_carrier_required` |
  | the record found is under a different scope than the route's stamp | `withdraw_not_permitted` |

  - A caller other than the scope's owner is the carrier: the record is keyed
    on the route's stamped principal, and `arguments.carrier`, if present,
    must equal it.
  - The scope's owner may withdraw any carrier's call by naming
    `arguments.carrier`. An owner that is also the carrier is treated as the
    carrier: `carrier` is optional and must match if present.
  - On an unscoped route, only the carrier may withdraw, keyed on the route's
    principal.
- The reply is a `RESPONSE` whose body is the answer (`WithdrawAnswer`):
  - `{answer: "withdrawn"}`: the action is guaranteed never to run;
  - `{answer: "already_started", outcome: "<code>" | "unknown"}`;
  - `{answer: "completed", result: <recorded reply>, result_retained: true}`,
    or `{answer: "completed", outcome: "<code>", result_retained: false}`;
  - `{answer: "refused", reason: "denied" | "expired" | "scope_ended" | <open>}`;
  - `{answer: "unknown_call"}`: no record has that `(carrier, call_key)`. A
    provider answers this for a key it never held, and never refuses such a
    key (the caller checks aside). It never means "not sent".
- The answer is read from the stored record, so it is identical, byte for
  byte, for every permitted caller and every repeat.
- The caller policy (`caller_verdict`):

  | Reply | Caller concludes | Retries |
  |---|---|---|
  | `withdrawn`, `refused` | the call never ran | no |
  | `completed` | the call ran | no |
  | `already_started` with an outcome | that outcome | no |
  | `already_started`, `unknown` | `outcome_unknown` | no |
  | `unknown_call` | nothing was held | no |
  | any other `answer`, or a body that is not an answer | final, unclassified; recorded verbatim | no |
  | `withdraw_not_permitted` and the three errors above | terminal refusal | no |
  | a transient route refusal (`errors::is_transient`) | nothing yet | yes |
  | any other route error | terminal refusal | no |

## 6. Approval execution on the provider

A provider that executes approval-gated calls itself moves each through four
durable states, each synced before the step it permits: Prepared,
Authorized, DispatchStarted, Settled.

- A call that is Prepared, or Authorized without DispatchStarted, when the
  provider restarts is never run automatically. The provider reports it to the
  call's custodian as a `late_results` entry of kind `not_started` with
  `reason: "restart_before_dispatch"`, and `tool.withdraw` for it answers
  `withdrawn` or `refused`.
- A call that had started when the provider restarted reports what the
  provider observes of it; when it cannot tell whether the call settled, it
  says `unknown`, never a guessed success. It is never re-sent.

## 7. Late results

A provider that settles calls later declares the `late_results` session
capability. The custodian (the scope's owner for a direct call, or the
carrier of an onward call) pulls; the provider never pushes, and serves an
entry only to a caller whose verified principal is its custodian.

- Request: `late_results {since: cursor | null, limit?}`
  (`LateResultsRequest`). `since: null` reads from the start.
- Reply: `{entries, cursor: {provider_incarnation, seq}, more: bool}`
  (`LateResultsReply`). `more` says entries beyond `limit` are waiting.
- Entry (`LateEntry`): `{kind, owner, ref, scope_epoch, custodian, call_key,
  invocation_id?, event_id, settled_at, reduced, result? | outcome?}`, plus
  `reason` on a `not_started` entry.
  - `kind` is `result`, `not_started` or `expired`, decoded open. An entry of
    an unknown kind is recorded and acked like any other, never retried.
  - `not_started` carries `{call_key, event_id, settled_at, reason}`; `reason`
    is an open string whose first value is `restart_before_dispatch`.
  - `reduced: true` means the provider kept only the outcome, in `outcome`,
    rather than the full result.
  - `call_key` is qualified by the carrier; `invocation_id` is present only
    for an onward request.
  - `event_id` is deterministic: the same late result always carries the same
    `event_id` across restarts and lost acks.
- Ack: `late_results.ack {through: cursor}` (`AckRequest`), per custodian.
  Each custodian has its own cursor and acks only its own entries.
- A cursor from another provider incarnation, or past the entries the
  provider has, is refused `cursor_incarnation_changed` (`check_since`). The
  caller re-reads from `null` and deduplicates on `(provider, custodian,
  call_key, event_id)` (`IntakeKey`).
- Retention: an unacked result is kept until acked or 24 h after it settled,
  and a scope ending does not delete it. On expiry an `expired` entry stays
  until acked. At most 1,000 unacked results per session and 1,000 expired
  entries per scope owner are kept, oldest dropped first.

## 8. Crash harness and kill points

Implementations supply a `cortexkit-role-harness` `Harness`. The suite calls
`kill_at` once per point in a run, each time on a module it spawned on a fresh
state root, and never kills one trigger at several points; `CrashDriver`
refuses anything else. This role's kill points (`points.rs`):

| Point | Durable state after the kill | Used by the v1 suite |
|---|---|---|
| `Prepared` | the held call and its filed question, nothing authorized or sent | yes |
| `Authorized` | the approval decision recorded, nothing sent | yes |
| `DispatchStarted` | sending may have begun | declared, not cut yet |

A provider whose approval state lives in a store where truncation is not
meaningful (SQLite, say) reaches these points with a fault hook followed by a
real process kill, and declares that.

## 9. Conformance suite

Crate `cortexkit-role-tool-provider-conformance`. A provider runs it in its own
CI against its real module over a real route; never against a double.

| Case | Requires |
|---|---|
| `role_describe_shape`, `role_describe_cacheable` | — |
| `catalog_schemas_flat`, `catalog_schema_digest_stable`, `catalog_digest_only` | — |
| `catalog_disabled_tool_absent`, `call_disabled_tool_refused_by_name` | `disable_tool` |
| `terminal_frame_on_success`, `terminal_frame_on_refusal` | — |
| `terminal_frame_on_cancel` | `cancellation` |
| `call_key_malformed_refused`, `call_key_well_formed_accepted` | `call_key` |
| `schema_pin_current_accepted`, `schema_pin_malformed_refused` | `schema_pin` |
| `withdraw_unknown_call`, `withdraw_answer_identical_on_repeat_and_owner`, `withdraw_owner_as_carrier_needs_no_carrier`, `withdraw_owner_without_carrier_refused`, `withdraw_carrier_naming_other_carrier_refused`, `withdraw_scope_mismatch_refused`, `withdraw_top_level_call_key_refused`, `withdraw_malformed_target_key_refused`, `withdraw_not_permitted_is_route_error` | `held_calls`, `scope_stamp`, `call_key` |
| `late_results_cursor_round_trip`, `late_results_ack` | `late_results`, `scope_stamp`, `held_calls`, `call_key`, `approval_execution` |
| `late_results_incarnation_change_refused` | `late_results`, `scope_stamp` |
| `crash_after_prepared_not_started`, `crash_after_authorized_not_started` | `approval_execution`, `held_calls`, `scope_stamp`, `call_key`, `late_results` |

Caller-side rules that no live provider can be made to exercise (an unknown
withdraw `answer`, an unknown late-result `kind`, the withdraw caller policy)
are tested against the vectors in this crate.

Verdict:

- A case whose requirements the subject does not declare is skipped, with the
  missing capabilities, never passed.
- **Passed**: every case ran and passed, and at least one kill ended a real
  process.
- **Conforming for declared capabilities** (naming the skipped tags): nothing
  failed, and every skip is a capability the subject does not declare. Never
  a plain pass.
- **Failed**: a case failed, or the run killed anything and no kill ended a
  real process (a run whose kills are all simulated cannot catch a provider
  that keeps something only in memory).

## Choices to confirm

No item from the previous open list remains open. While applying the rulings,
these details had no ruling and are fixed here as written; each needs the role
owner's confirmation:

| Id | Choice | Why it needed one |
|---|---|---|
| C1 | `provider_incarnation` is an opaque string; `seq` is a `u64`, `0` before any entry; `settled_at` is milliseconds since the Unix epoch (`u64`); `limit` is a `u32`. | The rulings named these members but not their types. |
| C2 | `late_results.ack` answers an empty `RESPONSE` body `{}`; the suite accepts any `RESPONSE`. | The rulings named the ack request, not its reply. |
| C3 | Transient withdraw refusals are `scope_not_synced` plus every code subc's own `is_retryable_route_open` retries (`module_reloading`, `module_warming`, `target_unavailable`, `module_timeout`); every other route error is terminal. | The ruling named two examples of transient refusals; withdraw is idempotent, so retrying a timeout is safe. |
| C4 | A caller other than the scope's owner that names another carrier gets `withdraw_carrier_mismatch`, whether or not it carries calls of its own. `withdraw_not_permitted` is therefore reached only when the record found is under a different scope than the route's stamp. | A provider cannot tell "a carrier naming another carrier" from "a stranger naming a carrier": both are non-owners naming someone else. |
| C5 | The scope's owner naming no carrier, with no call of its own under the key, gets `withdraw_carrier_required`, not `unknown_call`. | The owner-as-carrier rule and the carrier-required rule overlap on exactly this case. |
| C6 | The top-level catalog `capabilities` object declares a capability with the value `true`; other values are not declarations. | The ruling named the object, not its values. |
| C7 | A late-result entry of any kind, including an unknown one, must carry `owner`, `ref`, `scope_epoch`, `custodian`, `call_key`, `event_id` and `settled_at`; `reduced` defaults to `false`. An entry missing one does not decode. | The dedupe key needs these members, even for a kind the caller does not know. |

Gaps this revision found, which need a ruling:

| Id | Gap |
|---|---|
| G1 | **Schema pins and `generation`.** A pin names the catalog `generation`, which changes whenever any catalog content changes, descriptions included. The role document meant a description-only deploy to keep serving old pins. Nothing says how a provider decides it can still honour a pin whose generation is not current: whether it must remember each past generation's schema digests, or refuse `tool_schema_changed`. |
| G2 | **`catalog_digest` outside `digest_only`.** Only a `digest_only` answer carries `catalog_digest`, so a caller cannot compare a digest-only check against the answer it froze without making a second call. Whether a full answer also carries it, and which bytes it covers, is not decided. |
| G3 | **`role.describe` for several majors.** `versions` lists every major, but `stability` and `ops` are single values, so a module whose majors differ in stability or ops cannot say so. |
