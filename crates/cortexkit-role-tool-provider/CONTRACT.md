# `tool-provider/v1` — role contract

Stability: **alpha**. This document is the role definition; the Rust types in
this crate are its wire shapes, and `cortexkit-role-tool-provider-conformance`
is its suite. Every item below is marked:

- **Pinned** — a provider must do this, and a consumer may rely on it.
- **Open** — the role leaves it undecided. A provider must not rely on any
  particular answer, and the suite does not test it beyond what is stated.
  Open items are collected at the end with the question each one needs
  answered.

A tool provider is any module that serves tool definitions and executes calls
to them. Nothing here names a particular implementation.

## 1. Role identity

- **Pinned.** A provider lists `tool-provider/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). That grammar covers roles only; a
  tool's capability tags never go in the manifest.
- **Pinned.** Required ops: `role.describe` and `tool.catalog`
  (`REQUIRED_OPS`). A provider that can hold a call past its own reply also
  serves `tool.withdraw`. A provider that settles calls later also serves
  `late_results`.
- **Pinned.** A consumer refuses a module whose `role.describe` lacks a
  required op, by name, before routing anything to it.
- **Pinned (for the suite; see open item O1).** Role ops travel as ordinary
  tool-call requests on the provider's tool route, with `name` set to the op
  and the op's input in `arguments`, the same way `tool.withdraw` does.

## 2. `role.describe`

- **Pinned.** The answer is `{role, version, stability,
  implementation_version, ops, capabilities}` (`RoleDescribe`), with
  `role: "tool-provider"` and `version: "v1"`.
- **Pinned.** Decoded leniently: unknown fields, ops and capabilities are
  ignored; only the role, the version and the presence of the required ops are
  checked strictly. `capabilities` may be omitted. `stability` is `alpha`,
  `beta` or `stable`; an unknown level decodes, it does not fail the answer.
- **Pinned.** The answer describes the implementation, never the tools: it
  carries no tool list. It depends only on the module build, so a consumer may
  cache it for as long as it talks to the same module incarnation. Two answers
  from one incarnation are identical.
- **Pinned.** `tool-provider/v1` defines no module-level capabilities. Its
  session-level capabilities travel in the catalog answer (section 3).
- **Open (O2).** How a module that serves several majors of the role answers
  `role.describe`.

## 3. `tool.catalog`

The one catalog-and-text fetch. It serves preflight's tool section, the
runner's fetch of the session's plan item, and the live view.

- **Pinned.** Request: `{params, composition?, system_text?, digest_only?}`
  (`CatalogRequest`). `params` is the plan item's params: the shared
  vocabulary (`behavior`, `scope`, `tool_descs`, `exclude`) plus the
  provider's own axes; an unknown value is refused, never guessed.
  `composition` is absent on a preflight call. `system_text` is
  `{preset, params}` when the plan has a system-prompt item for this provider.
  `digest_only` is optional.
- **Pinned.** Answer: `{generation, composition_digest, tools: [...],
  system_text?}` (`CatalogAnswer`). A provider with only system text answers
  an empty `tools` list. When `system_text` was requested, the answer carries
  `{text, item_digest, preflight_digest, composition_digest}` for it, from the
  same configuration resolution as the catalog in the same reply.
- **Pinned.** Each tool: `{name, schema_digest, result_ops, capabilities,
  ...}` (`CatalogTool`).
  - `name` is the exact name the user tier disables the tool by.
  - `schema_digest` is the digest of the argument schema. Consumers compare it
    for equality only. Two answers from the same inputs carry the same digest.
  - `result_ops` lists the hook result operations the tool accepts, from
    `prepend`, `append`, `replace`. Absent means all three.
  - `capabilities` lists the tool's capability tags, for example
    `code.outline/v1` (unprefixed, defined only by this document) or
    `acme:code.callgraph/v1` (namespaced, free for anyone). They are data in
    this answer only.
- **Pinned.** The answer is a pure function of its inputs (the plan item, the
  composition, user and project configuration, host facts). Scope, owner and
  agent on the route are attribution only and never change it. Tool names and
  schemas never depend on the composition; descriptions and system text may.
  The same inputs give the same bytes.
- **Pinned.** A tool the project or user tier disables is absent from the
  answer.
- **Pinned.** Argument schemas are flat: a JSON object with no root-level
  `anyOf`, `oneOf` or `allOf` (`check_flat_schema`). Unions below the root are
  allowed.
- **Pinned.** A host-only parameter carries `"x-ck-audience": "host"`, is
  always optional, and is refused in a call from the model.
- **Pinned.** Session-level capabilities a provider may report for a session:
  `host_params` and `late_results`. The runner freezes them with the tools.
- **Pinned (provisional name; see O3).** The argument schema is the tool's
  `input_schema` member, and its description is `description`. The suite reads
  `input_schema` to check flatness.
- **Open (O4).** The type and semantics of `generation` beyond "a cue to
  refetch", and the separate semantics version a call pins. Decoded verbatim.
- **Open (O5).** The shape of `composition`. Carried verbatim.
- **Open (O6).** The member that carries session-level capabilities in the
  answer.
- **Open (O7).** The exact shape of a `digest_only: true` answer (which
  members are dropped besides `system_text.text`).
- **Open (O8).** Whether a root-level `"type"` array (`["object", "null"]`)
  counts as a union.

## 4. Calls

- **Pinned.** A call is a route `REQUEST` whose body is subc-protocol's
  `ToolCallRequest`: `{name, arguments, tool_call_id?, progress_token?,
  call_key?}`.
- **Pinned.** `call_key` is the top-level member added in subc-protocol
  0.26.0: opaque, 1 to 256 bytes, each printable non-space ASCII
  (0x21–0x7E), checked by the one exported validator. A present but malformed
  key is refused as `invalid_request` with `detail: {field: "call_key"}`. An
  absent key is not an error: the call executes but cannot be deduplicated or
  withdrawn. This crate carries a marked local mirror of the field and its
  validator until 0.26.0 publishes (`call.rs`).
- **Pinned.** Every record shared between parties keys on
  `(carrier principal, call_key)`. The model's `tool_call_id` is display-only.
- **Pinned.** Every request gets exactly one terminal frame (`RESPONSE`,
  `STREAM_END` or `ERROR`), and it is the last frame: on success, on refusal,
  on cancel, on drain, and when the provider stops tracking the request.
- **Pinned.** A call to a disabled tool is refused as `tool_disabled` with
  `detail: {tool}`.
- **Pinned.** Refusal codes a runner records as the call's error result and
  never re-dispatches: `tool_schema_changed {tool, expected, current}`,
  `tool_semantics_changed {tool, expected, current}`,
  `tool_unavailable {tool, reason: retired | under_review}`,
  `capability_not_admitted`.
- **Pinned.** Model-shaped arguments are coerced before anything records,
  hashes or checks them. Scope, owner and agent are never arguments; they come
  from the route's stamp.
- **Open (O9).** Where a call carries its schema pin, its semantics version and
  its plan item's params on the wire (not in `ToolCallRequest` as of 0.26.0).
- **Open (O10).** The error code for a call to a tool the provider does not
  serve at all. The suite requires only an error frame.
- **Open (O11).** How a keyless call's reply says it cannot be deduplicated or
  withdrawn.

## 5. `tool.withdraw`

Served by a provider that can hold a call past its reply.

- **Pinned.** The request is an ordinary tool request on the provider's tool
  route: `{name: "tool.withdraw", arguments: {call_key, carrier?, scope?}}`
  (`WithdrawArguments`). It is not a model tool. Its own top-level `call_key`
  is absent; a provider refuses one that is present as `invalid_request
  {field: "call_key"}`.
- **Pinned.** The record is keyed `(carrier, call_key)`. The carrier is the
  principal that raised the call, as a principal string (`reserved:broca`, …).
- **Pinned.** Who may withdraw: the carrier, or the owner of the call's scope
  (`resolve_carrier`).
  - When the caller is the carrier, the carrier is the route's stamped
    principal. An `arguments.carrier` that differs from it is refused.
  - When the caller is the scope's owner, `arguments.carrier` is required, and
    a request without it is refused.
  - `arguments.scope`, when present, must equal the route's stamped scope; a
    mismatch is refused. Scope identity is `{owner, ref, scope_epoch}`.
  - A caller that is neither the carrier nor the owner of the record's scope
    gets the **route error** `withdraw_not_permitted`, never an answer. A
    caller that is not the owner and names a carrier other than itself is such
    a caller.
- **Pinned.** The reply is a `RESPONSE` whose body is the answer
  (`WithdrawAnswer`):
  - `{answer: "withdrawn"}`: the action is guaranteed never to run;
  - `{answer: "already_started", outcome: "<code>" | "unknown"}`: `unknown`
    when the provider cannot tell whether the call settled, never a guessed
    success;
  - `{answer: "completed", result: <recorded reply>, result_retained: true}`,
    or `{answer: "completed", outcome: "<code>", result_retained: false}`;
  - `{answer: "refused", reason: "denied" | "expired" | "scope_ended" | <open>}`;
  - `{answer: "unknown_call"}`: no record has that `(carrier, call_key)`. A
    provider answers this for a key it never held, and never refuses such a
    key (the caller check aside). It never means "not sent", and it does not
    upgrade an `outcome_unknown`.
- **Pinned.** The answer is read from the stored record, so it is identical,
  byte for byte, for every permitted caller and every repeat.
- **Pinned.** `answer`, `outcome` and `reason` are open strings. An `answer`
  the decoder does not know is final but unclassified: the caller records the
  body verbatim and stops retrying. Every decoded answer ends the caller's
  retries.
- **Open (O12).** The error codes for the owner without `carrier`, the carrier
  naming another carrier, and the scope mismatch. The suite requires an error
  frame, not an answer. (A caller naming a carrier other than itself fails the
  caller check, so `withdraw_not_permitted` fits the second.)
- **Open (O13).** When the scope's owner is also the call's carrier: the suite
  follows the rule as written, so the owner always names the carrier, even
  when it is itself.
- **Open (O14).** Whether a malformed `arguments.call_key` is refused (and
  with which `field`) or answered `unknown_call`.
- **Open (O15).** What a caller does with a reply that is not an answer at all
  (no string `answer`, or a known answer missing a member it must carry):
  `WithdrawAnswer::decode` reports it as an error, and the retry policy is
  undecided.
- **Open (O16).** A withdraw on an unscoped route.

## 6. Approval execution on the provider

A provider that executes approval-gated calls itself moves each through four
durable states, each synced before the step it permits: Prepared, Authorized,
DispatchStarted, Settled.

- **Pinned.** A call that is Prepared, or Authorized without DispatchStarted,
  when the provider restarts is never run automatically. It is reported as not
  started.
- **Pinned.** A call that had started when the provider restarted reports
  what the provider observes of it; when it cannot tell whether the call
  settled, it says `unknown`, never a guessed success. It is never re-sent.
- **Pinned (the suite's reading).** After such a restart, `tool.withdraw` for
  the call answers `withdrawn` or `refused`: both guarantee the action never
  runs (`WithdrawAnswer::guarantees_not_run`). `unknown_call` fails, because
  the Prepared record was durable and must survive.
- **Open (O17).** The wire on which "not started" is reported to the owner
  (a late-result entry, an outcome code, or something else).

## 7. Late results

- **Pinned.** A provider that settles calls later declares the `late_results`
  session capability and serves `late_results {since}`. The custodian pulls;
  the provider never pushes.
- **Pinned.** The log is keyed `{owner, ref, scope_epoch, custodian, call_key,
  invocation_id, event_id}`; `call_key` is qualified by the carrier;
  `invocation_id` is present only for an onward request.
- **Pinned.** The cursor is `{provider_incarnation, seq}`. A cursor from
  another incarnation, or past a gap, is refused as
  `cursor_incarnation_changed`, and the reader re-reads from the start.
- **Pinned.** `event_id` is deterministic: the same late result always carries
  the same `event_id` across restarts and lost acks. The provider derives it
  from the call's own identity and its outcome; the derivation is the
  provider's.
- **Pinned.** One ack stream per custodian: each custodian has its own cursor
  and acks only its own entries.
- **Pinned.** An entry is served only to a caller whose verified principal is
  its custodian. An expired entry leaves `{call_key, event_id, settled_at}`
  until acked. An unacked result is kept until acked or 24 h after it settled,
  at most 1,000 unacked per session and 1,000 expired entries per scope owner.
- **Open (O18).** The reply envelope of `late_results`, the entry payload
  (including how a reduced result says so), the ack op's name and shape, and
  the types of `provider_incarnation`, `seq` and `settled_at`. No decoder
  exists until they are settled.

## 8. Crash harness and kill points

Implementations supply a `cortexkit-role-harness` `Harness`. This role's kill
points (`points.rs`), opaque strings to the harness:

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
CI against its real module over a real route; never against a double. The
crate's own tests use an in-process fake only to test the runner.

| Case | Requires |
|---|---|
| `role_describe_shape` | — |
| `role_describe_cacheable` | — |
| `catalog_schemas_flat` | — |
| `catalog_schema_digest_stable` | — |
| `catalog_disabled_tool_absent` | `disable_tool` |
| `call_disabled_tool_refused_by_name` | `disable_tool` |
| `terminal_frame_on_success` | — |
| `terminal_frame_on_refusal` | — |
| `terminal_frame_on_cancel` | `cancellation` |
| `call_key_malformed_refused` | `call_key` |
| `call_key_well_formed_accepted` | `call_key` |
| `withdraw_unknown_call` | `held_calls`, `scope_stamp`, `call_key` |
| `withdraw_answer_identical_on_repeat_and_owner` | `held_calls`, `scope_stamp`, `call_key` |
| `withdraw_owner_without_carrier_refused` | `held_calls`, `scope_stamp`, `call_key` |
| `withdraw_carrier_naming_other_carrier_refused` | `held_calls`, `scope_stamp`, `call_key` |
| `withdraw_scope_mismatch_refused` | `held_calls`, `scope_stamp`, `call_key` |
| `withdraw_top_level_call_key_refused` | `held_calls`, `scope_stamp`, `call_key` |
| `withdraw_not_permitted_is_route_error` | `held_calls`, `scope_stamp`, `call_key` |
| `crash_after_prepared_not_started` | `approval_execution`, `held_calls`, `scope_stamp`, `call_key` |
| `crash_after_authorized_not_started` | `approval_execution`, `held_calls`, `scope_stamp`, `call_key` |

The withdraw decoder's rules (including the unknown-`answer` rule) are tested
against the vectors in this crate, not against a live module.

Verdict:

- A case whose requirements the subject does not declare is **skipped** with
  the missing capabilities, never passed. A run with a skip is **incomplete**,
  not passed.
- A run **fails** if any case fails, or if it killed anything and no kill
  ended a real process: a run whose kills are all simulated cannot catch a
  provider that keeps something only in memory.
- **Open (O19).** Whether a provider that never holds calls (so its
  `held_calls` cases are always skipped) may claim conformance with an
  incomplete run.

Test vectors: `test-vectors/tool-provider-v1/` at the repository root.

## Open items

| Id | Question |
|---|---|
| O1 | Confirm that `role.describe` and `tool.catalog` travel as ordinary named requests on the tool route, as `tool.withdraw` does. |
| O2 | How a module serving several majors of the role answers `role.describe`. |
| O3 | The member names of a catalog tool's argument schema (`input_schema`) and description (`description`). |
| O4 | The type of the catalog's `generation`, and the separate semantics version: its name, type and where it appears. |
| O5 | The shape of the composition passed to `tool.catalog`. |
| O6 | The catalog answer member that carries session-level capabilities. |
| O7 | The exact shape of a `digest_only: true` answer. |
| O8 | Whether a root-level `"type"` array counts as a union. |
| O9 | Where a call carries its schema pin, semantics version and plan-item params. |
| O10 | The error code for a call to a tool the provider does not serve. |
| O11 | How a keyless call's reply says it cannot be deduplicated or withdrawn. |
| O12 | Error codes for the owner without `carrier`, a carrier naming another carrier, and a scope mismatch on `tool.withdraw`. |
| O13 | `tool.withdraw` when the scope's owner is also the carrier. |
| O14 | A malformed `arguments.call_key` on `tool.withdraw`: refusal (which field) or `unknown_call`. |
| O15 | The caller's retry policy for a withdraw reply that is not an answer. |
| O16 | `tool.withdraw` on an unscoped route. |
| O17 | The wire on which a provider reports a not-started call to the owner after a restart. |
| O18 | The `late_results` reply envelope, entry payload, ack op and value types. |
| O19 | Whether an incomplete run (only capability skips) may count as conformance for a provider that never holds calls. |
