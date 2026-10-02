# `tool-provider/v1` — role contract

Stability: **alpha**. This document is the role definition; the Rust types in
this crate are its wire shapes, `test-vectors/tool-provider-v1/` at the
repository root holds its vectors, and `cortexkit-role-tool-provider-conformance`
is its suite. Every item below is **pinned**: a provider must do it, and a
consumer may rely on it. Nothing in this revision is open.

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

- The answer is `{majors: [{version, ops, stability}], implementation_version,
  capabilities?}` (`RoleDescribe`). `majors` lists every major of the role
  the module serves, each with its own `ops` and `stability`; `version` is
  spelled as in the manifest, for example `tool-provider/v1`. The caller picks
  the highest major it understands for a new session.
- Decoded leniently: unknown fields, majors, ops and capabilities are
  ignored; only the presence of the `tool-provider/v1` major and its required
  ops are checked strictly. `capabilities` may be omitted. `stability` is
  `alpha`, `beta` or `stable`; an unknown level decodes.
- The answer describes the implementation, never the tools: it carries no
  tool list. It depends only on the module build, so a consumer may cache it
  for as long as it talks to the same module incarnation; two answers from one
  incarnation are identical.
- `tool-provider/v1` defines no module-level capabilities. Session-level
  capabilities travel in the catalog answer.

## 3. `tool.catalog`

- Request: `{params, preset?, composition?, system_text?, digest_only?}`
  (`CatalogRequest`).
  - `params` is the plan item's params: the shared vocabulary (`behavior`,
    `scope`, `tool_descs`, `exclude`) plus the provider's own axes. An unknown
    value is refused, never guessed.
  - `preset` is the plan item's named variant, defined by the provider. It
    sits beside `params`, not inside it, so it never collides with a
    provider's own axis, and mirrors `system_text`'s `{preset, params}`.
    Absent means the provider's default variant; an encoder omits the member
    rather than sending `null`. A preset the provider does not define is
    refused as `invalid_request {field: "preset"}`, never guessed, like an
    unknown `params` value (`test-vectors/tool-provider-v1/catalog-requests.json`).
  - `composition` is the session's composition, carried verbatim as an opaque
    JSON object. Providers never interpret it beyond resolving their own text
    against it. Absent on a preflight call.
  - `system_text` is `{preset, params}` when the plan has a system-prompt item
    for this provider.
- Answer (`CatalogAnswer`): `{generation, catalog_digest, composition_digest?,
  tools: [...], system_text?, capabilities?}`.
  - `generation` is an opaque string that changes whenever the catalog's
    content changes.
  - `catalog_digest` is an opaque digest of the answer's content. A full
    answer and a `digest_only` answer to the same inputs carry the same value,
    so a caller holding a full answer can check it later with one cheap
    `digest_only` fetch. When the request asks for `system_text`, that item is
    part of the answer's content, so `catalog_digest` covers it too: a change
    to the text alone changes `catalog_digest`, and a `digest_only` fetch
    with the same `system_text` request detects it.
  - `composition_digest` is SHA-256 over the RFC 8785 (JCS) canonical JSON
    of the request's `composition` object exactly as sent, written as 64
    lowercase hex characters (`composition_digest`). Nothing is stripped
    first. The runner computes the same value over the composition it sent
    and compares (`test-vectors/tool-provider-v1/composition-digest.json`
    carries each composition, its JCS bytes and the digest). Absent when the
    request carried no composition.
  - `capabilities` is a top-level object of session-level capabilities, keyed
    by name: `host_params`, `late_results`. A capability is declared by the
    value `true`; any other value is not a declaration. The runner freezes
    them with the tools.
  - `system_text`, when requested, is `{text, item_digest, preflight_digest,
    composition_digest, tool_names}`, from the same configuration resolution
    as the catalog in the same reply. `tool_names` lists, sorted and without
    duplicates, the model-facing names of the tools the text was composed
    for; it is required whenever `text` is present. A runner compares it with
    the names of the tools it fetched from the same provider and refuses the
    plan as `plan_stale` with a `text_tool_names` difference (`missing`,
    `unexpected`) when they differ, so text written for one tool set is
    never served beside another.
  - A provider with only system text answers an empty `tools` list.
- `digest_only: true` answers `{generation, catalog_digest}` with no tools.
- Each tool (`CatalogTool`): `{name, schema_digest, semantics, result_ops?,
  capabilities, description?, input_schema}`.
  - `name` is the exact name the user tier disables the tool by.
  - `schema_digest` is the digest of the **structure** of `input_schema`
    (see "Schema digest" below): 64 lowercase hex characters. Description
    text never changes it.
  - `semantics` is an integer, bumped whenever the tool's behaviour changes
    without a schema change.
  - `result_ops` lists the hook result operations the tool accepts, from
    `prepend`, `append`, `replace`. Absent means all three.
  - `capabilities` lists capability tags: unprefixed (`code.outline/v1`,
    defined only by this document) or namespaced (`acme:code.callgraph/v1`,
    free for anyone). Data in this answer only. This document defines these
    unprefixed tags, each a promise any provider can make about a tool:

    | Tag | The tool |
    |---|---|
    | `shell.exec/v1` | runs a shell command in the session's workspace and returns its output and exit status |
    | `code.read/v1` | returns file contents, whole or by range |
    | `code.edit/v1` | changes files in the workspace |
    | `code.search/v1` | finds code by text, regular expression or meaning |
    | `code.files/v1` | lists or finds files by path pattern |
    | `code.outline/v1` | returns the structure of a file or directory (symbols, headings) |
    | `code.callgraph/v1` | answers caller, callee and impact questions about code |
    | `code.diagnostics/v1` | returns compiler or linter diagnostics |
    | `browser.use/v1` | drives a web browser session: navigates, reads the page and acts on it |
    | `computer.use/v1` | drives desktop applications: reads their state and acts on them |
    | `git.history/v1` | reads repository history: commits, file contents at a revision, and diffs between revisions, without changing the repository |

    A defined tag promises only the activity in its row. Everything else is
    the provider's own: its arguments, permission grants, consent flow,
    element or object identifiers, output format and refusal wording. A
    consumer that matched a defined tag must not depend on any of those. A
    provider that wants consumers to match its specific contract adds its own
    namespaced tag beside the defined one (for example
    `cerebellum:browser.use/v1`), and a consumer that depends on that contract
    pins the namespaced tag.

    Any other tag is `<namespace>:<name>/v<N>`: a namespace of lowercase letters
    and digits in `-`-separated words, a name of `.`-separated words of
    lowercase letters, digits and `_`, and a version with no leading zero
    (`acme:code.callgraph/v1`). The crate lists the defined tags as
    `DEFINED_CAPABILITY_TAGS` and checks a tag with `check_capability_tag`;
    the conformance suite refuses a catalog with any tag that fails it.
  - `input_schema` is the argument schema; `description` its description.
- The answer is a pure function of its inputs (the plan item's preset and
  params, the composition, user and project configuration, host facts), so
  `catalog_digest` covers the preset: a `digest_only` request with the same
  preset carries the full answer's `catalog_digest`. Scope, owner and
  agent on the route are attribution only. Tool names and schemas never depend
  on the composition; descriptions and system text may. The same inputs give
  the same bytes.
- A tool the user or project tier disables is absent from the answer.
- **Schema digest** (`schema_digest`, `structural_schema`). The digest covers
  the argument schema's structure (property names, types, `required`, enums,
  bounds and every other keyword) and never description text:
  1. Remove the `description` keyword from every schema object: the root and
     every subschema reached through `properties`, `patternProperties`,
     `dependentSchemas`, `$defs`, `definitions` (the subschemas under each
     name), `items`, `additionalItems`, `additionalProperties`,
     `unevaluatedItems`, `unevaluatedProperties`, `not`, `if`, `then`,
     `else`, `contains`, `propertyNames`, and each element of `prefixItems`,
     `anyOf`, `oneOf`, `allOf`. Property names stay, even a property called
     `description`; data values (`enum`, `const`, `default`, `examples`) and
     unknown keywords stay verbatim.
  2. Serialize the result as RFC 8785 (JCS) canonical JSON.
  3. Take SHA-256 of those bytes, written as 64 lowercase hex characters.

  Two catalogs that differ only in description text give every tool the same
  digest; any structural change gives a different one
  (`test-vectors/tool-provider-v1/schema-digest.json`).
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
  the tool's `schema_digest` and its `semantics`, as the runner froze them.
  A pin never holds the catalog `generation` or any description text, so it
  survives a description-only deploy. It is encoded canonically as **`tp1`**
  (`SchemaPin`):

  ```text
  tp1:<pct(tool)>:<schema_digest>:<semantics>
  ```

  `pct` keeps the RFC 3986 unreserved bytes (`A-Z a-z 0-9 - . _ ~`) and
  writes every other byte of the UTF-8 string as `%` plus two uppercase hex
  digits, so `:` inside the tool name is always `%3A`. `schema_digest` is 64
  lowercase hex characters. `semantics` is decimal, with no sign and no
  leading zeros. The tool is non-empty. Exactly one string encodes each pin;
  anything else is refused. A pin that is
  malformed, or names a tool other than the call's `name`, is refused as
  `invalid_request {field: "schema_pin"}`. The field and its bound are
  subc-protocol 0.27.0's, re-exported by this crate (`call.rs`).
- A call whose pinned `schema_digest` the provider can no longer honour is
  refused `tool_schema_changed {tool, expected, current}`; one whose pinned
  `semantics` it can no longer honour, `tool_semantics_changed {tool,
  expected, current}`. Providers keep argument changes additive so old
  schemas stay servable.
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
    must equal it. Naming anyone else is `withdraw_carrier_mismatch`, whether
    or not the caller carries calls of its own, so `withdraw_not_permitted`
    is reached only when the record found is under a different scope than the
    route's stamp.
  - The scope's owner may withdraw any carrier's call by naming
    `arguments.carrier`. An owner that is also the carrier is treated as the
    carrier: `carrier` is optional and must match if present. An owner
    naming no carrier that holds no call of its own under the key gets
    `withdraw_carrier_required`, not `unknown_call`.
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
  | a transient route refusal: `scope_not_synced`, or any code subc itself retries (`module_reloading`, `module_warming`, `target_unavailable`, `module_timeout`; `errors::is_transient`) | nothing yet | yes |
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
  `provider_incarnation` is an opaque string that changes on every provider
  start; `seq` is a `u64`, `0` before any entry; `limit` is a `u32`.
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
  - `settled_at` is milliseconds since the Unix epoch (`u64`). `reduced`
    defaults to `false`.
  - Every entry, of any kind including an unknown one, carries `owner`,
    `ref`, `scope_epoch`, `custodian`, `call_key`, `event_id` and
    `settled_at`, because the caller's dedupe key needs them. An entry
    missing one does not decode.
- Ack: `late_results.ack {through: cursor}` (`AckRequest`), per custodian,
  answered with an empty `RESPONSE` body `{}`.
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
| `catalog_schemas_flat`, `catalog_schema_digest_stable`, `catalog_digest_only`, `catalog_unknown_preset_refused` | — |
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
