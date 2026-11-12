# Changelog

## 0.15.6

- Add `lint_input_schema` (re-exported with `SchemaLint` and `SchemaLintKind`
  from the new `schema_lint` module). It reports, by JSON pointer, each
  property schema under `properties`, `items`, `prefixItems` or an
  `additionalProperties` object that declares no `type` and is not an
  `anyOf`, `oneOf` or `allOf` combinator or a `$ref`. Some model providers
  refuse such a property even though it is valid JSON Schema. The lint only
  reports: no decoder or check calls it, and no wire format changes.
- CONTRACT.md §3 adds informative guidance to declare `type` on every
  property, and §9 lists the new conformance case `catalog_schemas_portable`.

## 0.15.5

- Accept `subc-protocol >=0.34.1, <0.37` so consumers pinned to 0.36 can share
  one protocol copy. Require tool-display 0.1.5 for the same compatibility.
- Re-exported protocol types are whichever `subc-protocol` version the build
  resolves. The same applies to protocol types in public APIs: `CallMode` in
  catalog fields and helpers, `ToolCallRequest` in builders and validators,
  `OpaqueFieldError` in `SchemaPinError`, `ScopeStamp` in user-command admission,
  and `ErrorBody` in validation results, error helpers and withdraw verdicts.
  No role wire-format or admission-policy change.

## 0.15.4

- Add provider capability `session-notices/v1` for completion-only idle starts
  and non-starting progress notices, with explicit plan sender modes. Readers
  first: consumers must accept the token before providers declare it.

- Add opt-in `reply_custody`, requiring `late_results` and `call_key`, so keyed
  outcomes survive route closure and remain in late results until acknowledged
  or the existing retention limit expires.
- Define frozen-catalog Pure exemptions, provider restart handling, custodian
  deduplication and fresh-key re-dispatch. Add pure custody decisions and vectors.

## 0.15.3

- Add `stop-session/v1`, route-keyed session-wide stop requests, safe bounded
  display replies, request checks and vectors. Separate principals and sessions
  cannot stop each other's work.
- Add opt-in `call-yield/v1`, per-tool `call.yield/v1` tags, `tool.yield`
  requests and answers, carrier-only
  route lookup, and the five-second hand-back promise without cancelling work.
- Add optional `held.yielded`, capability checks and shared JSON vectors.

## Unreleased

- Contract text only (§4): a carrier never sends the same `call_key` for two
  different calls, in one session or across sessions, while any party may
  still hold a record of the first. Shared records key on `(carrier,
  call_key)` with no session, so a key unique only within one session can
  merge two calls' records.
- Contract text only (§1.1): the one-bind-identity rule says explicitly
  that it covers every route a consumer opens to the module for a session,
  whatever role each route declares, not only tool-provider routes.

## 0.15.2

- Fix the catalog contract: only requests carrying `params.capabilities` are
  filter requests, and only an empty list selects no tools. Ordinary plan
  items keep the provider's documented params and preset rules, even when it
  advertises the filter. `check_capability_params` enforces filter-only keys;
  `select_by_capabilities` leaves supplied tools unchanged on ordinary requests.
- Add the opt-in `tool-catalog-model/v1` role capability (`CATALOG_MODEL`).
  Consumers send `params.model` only to advertising providers, on ordinary
  requests. `check_model_param` validates exactly two non-empty strings,
  `provider` and `model`, with no extra members. A provider's treatment of an
  unrecognised model remains its own documented rule.
- Correct the missing-list capability sample, retain the empty-list sample,
  and add model-shape and filter-with-model samples with refreshed checksums.

## 0.15.1

- Contract §1.1: a consumer that sends tool calls, user commands,
  withdrawals or late-result requests on a route declares
  `{"tool-provider": "v1"}` in the route's role versions, and should declare
  it on every route it uses for this role. The declaration never gates
  `role.describe`, `tool.catalog` or `tool.views`. Once every consumer that
  sends this role's requests to a provider declares the role, the provider
  may refuse this role's other requests and tool calls on an undeclared
  route, including a bind that declared another version, with the new
  `tool_role_not_declared` code (`errors::TOOL_ROLE_NOT_DECLARED`,
  `errors::tool_role_not_declared`). Until then it keeps accepting them. New
  helpers: `route_declares_v1` and `op_allowed_without_role`; they classify a
  bind and an operation, and don't track which consumers have declared.
- Contract §1.1: a consumer that opens several routes to one provider on
  behalf of one session binds them all with the same `harness` and
  `session`, and never binds another session under that pair. A provider
  that keys session state by the attested principal and the pair then
  recognises those routes as one session.

## 0.15.0

- New duty for every provider, hence a minor release (§4.5): a tool result's
  model-visible size must be at most 65,536 bytes
  (`result::MAX_MODEL_VISIBLE_BYTES`). The size is an accounting measure taken
  on the decoded body the provider sent, chosen by its negotiated format:
  `result::model_visible_bytes` for a `tool-result/v1` reply (text parts in
  UTF-8, unknown parts by their RFC 8785 JSON length; image parts and
  `structuredContent` excluded), `result::legacy_model_visible_bytes` for any
  other body, and the UTF-8 length of `message` for a refusal. Providers shape
  larger output themselves, by paging, summarizing or saving it to a file with
  a pointer. `result::validate` doesn't check the limit. A consumer never
  shortens a payload; one that enforces replaces it with an explicit size
  error that keeps the call's execution state, and never sends the call again.
  New vectors: `model-visible-size.json`.
- Contract text only (§4.4): for a call whose work runs in a child process,
  "advancing work" means the provider's supervision of the call, not
  progress inside the child. The supervision loop sends keepalives while it
  keeps checking the child and handling cancellation, whether or not the
  child prints or uses CPU; no separate timer may send them. Keepalives do
  not extend the deadline for the REQUEST's terminal frame, and CANCEL must
  stop the work in the child, not only its supervision. If supervision
  stops and nothing else arrives for the call, the silence window ends the
  wait.

## 0.14.6

- Add optional `ViewManifest::tool` and non-exhaustive `ViewTool`, carrying
  the catalog's `name`, `input_schema` and `semantics`. Missing `tool` emits
  no bytes, preserving existing serialized views.
- Add `ViewTool::schema_pin`, catalog conversion, `ViewManifest::with_tool`
  and `check_pin`, reusing structural schema digests and the canonical `tp1`
  encoder. A definition that doesn't recompute to the pin cannot authorize a
  display join; the check covers the pin binding, not schema validity.
- Contract §2.1: providers send the tool definition bound to each served pin
  (its name, structural input schema and `semantics`; `description` text isn't
  part of the pin), keep serving the first one, and MUST include it for pins
  outside their current catalog, so a first fetch of an old view can recover
  its input schema. Keep serving every previously served pin for as long as
  the provider declares `tool-views/v1`. `compare_with_kept` doesn't compare
  `tool`; the display server keeps the first accepted one.
- Ship matching, mismatching and legacy view-definition vectors, shared with
  tool-display 0.1.3.

## 0.14.5

- Contract text only (§2.1): a display server keeps each view with the
  writer's `module_version` and, where the daemon's `catalog.list` entry
  carries it, the writer's declared build (`provenance.git_sha`), and fetches
  again when either changes. It may also fetch a kept view again under its
  own policy, after an age it chooses or when the view fails its own template
  validation. Previously, deploying a revised template without changing
  `module_version` did not refetch an already-kept view.

## 0.14.4

- Define what happens to a held call when its scope ends (CONTRACT §6). It
  covers every call held past its first reply under a scope, whatever its
  held reason. Only `route.closed {reason: "scope_ended"}` for that scope, or
  `scope.describe` answering `status: "ended"` at the call's `scope_epoch`,
  counts as the end; any other route close is not by itself evidence and
  doesn't trigger the rule. A call before DispatchStarted is settled as not
  started and never dispatched, even if approved later: the provider records
  a `late_results` `not_started` entry with `reason: "scope_ended"`, and a
  permitted `tool.withdraw` answers `refused`, `scope_ended`. A call at or
  after DispatchStarted is never reclassified as not started; the provider
  reports what it observes, `unknown` when it can't tell.
- Add `late_results::reasons::SCOPE_ENDED`. `not_started` reasons stay open;
  a consumer treats an unknown reason as a call that never ran.
- Clarify existing text: `refused` means the original call will never run,
  not that the withdrawal was refused; `unknown_call` says nothing about
  whether a call was sent; a call recovered as not started after a restart
  is never dispatched, even if approved later.
- No wire shape or type changes. A provider that ran a held call after its
  scope ended must stop doing so.

## 0.14.3

- Add opt-in `call-keepalive/v1`: providers send a PUSH at least every 30 seconds
  while a call's work advances, stuck work goes silent, and runners stop
  waiting after 90 seconds of silence.
  Keepalive bodies are not progress and never enter history, logs, or model input.
  Providers send them only as the call's own work advances, never from an
  independent timer, and still send the request's terminal frame within the
  tool's declared reply bound.
- Add keepalive constants, body helpers, and `reply_deadline_ms_with_keepalive`.
  With both sides opted in, calls without a declared reply use the one-day ceiling;
  declared reply limits and the existing deadline helper are unchanged.
- Require runners to send CANCEL whenever they give up on a request that has no
  terminal frame yet. Declaring providers must stop the call's work; runners
  ignore late frames after recording `outcome_unknown` and never retry the call.
  A call held past its first reply is still withdrawn with `tool.withdraw`.
- Ship keepalive recognition and deadline vectors. Existing public types and wire
  shapes are unchanged.

## 0.14.2

- A view can carry `template_revision` (u64, default 0, omitted when 0). For
  a pin, `record_schema` and `reads` never change; `template` changes only
  with a higher revision, so a template fix can reach rows already recorded
  under that pin without a `semantics` bump. Added
  `ViewManifest::with_template_revision`, `ViewManifest::compare_with_kept`
  and `ViewChange`.
- Contract text (§2.1): a provider that has ever served a view for a pin
  keeps serving one, with the same record schema and reads, for as long as
  it declares `tool-views/v1`, and answers `"unknown"` only for a pin it
  never served. The display server refetches a writer's views when its
  `module_version` changes and keeps the highest compatible revision.

## 0.14.1

- Add optional `tool-views/v1`, `tool.views`, view manifests and vectors, using
  the record schemas of `cortexkit-role-tool-display` 0.1.0. Existing public
  structs, enums, required ops and existing wire shapes are unchanged.
- Pin the provider obligations for asynchronous display writes and declared
  reads on app-server-carried routes, with a link to `tool-display/v1`.

## 0.14.0

- Requires subc-protocol 0.34; breaking because `CallMode` and `ToolCallRequest`
  are re-exported and public helpers use protocol types. No role wire change.

## 0.13.3

- Add non-terminal `held` late entries so a provider can say that a call moved
  from waiting for a person to running, or back again, without answering it.
  Each update has its own replay-stable event id and takes effect at `settled_at`.
- Readers opt in first: providers may send updates only for held-first-reply
  calls whose bind declared `held-update/v1`, `held-reply/v1` and `tool-result/v1`.
  A decoder using the 0.13.2 shape still classifies the new kind as unknown and
  keeps its payload.
- Add `EntryKind::Held`, `LateEntry::new_held`, `LateEntry::held` and
  `LateEntry::validate`, shared `HeldReply::validate`, `HELD_UPDATE_CAPABILITY`
  and `consumer_reads_held_update_v1`. The held reason and detail obey the same
  rules in first replies and late updates. Time budgets remain consumer policy.
- Add `held-update-v1.json` vectors for the consent-to-running-to-result flow,
  malformed entries and the three-capability gate. Existing vectors are unchanged.

## 0.13.2

- Add the optional `rich_ui: {dialect, catalog_digest}` member on the
  system-text answer (`SystemTextAnswer.rich_ui`, `SystemTextAnswer::with_rich_ui`)
  and the validated `rich_ui::RichUiStamp` type. `dialect` is 1 to 32 printable
  ASCII characters with no whitespace, the dialect the rich UI catalog declares
  about itself and not the syntax crate's `rich-ui/1` scan mode;
  `catalog_digest` is 64 lowercase hex characters. A malformed stamp is refused
  on decode naming `rich_ui.dialect` or `rich_ui.catalog_digest`.
- A provider declares the stamp only on text that teaches that dialect against
  that catalog, copying both values from the catalog's published export.
- Absent stamps are omitted, so existing answers keep their bytes. New vector
  file `rich-ui-stamp.json`; existing vector files are unchanged.

## 0.13.1

- Add optional `CatalogTool.load`, the open-string `ToolLoad` hint, and
  `CatalogTool::with_load`. `"upfront"` asks hosts that defer tool definitions
  behind a search to keep the tool visible from the first turn; hosts may ignore
  it, and unknown values round-trip unchanged.
- Absent `load` emits no new bytes and preserves existing catalog digests.
  Present hints are catalog content, not part of the schema digest or schema pin.

## 0.13.0

- Renamed the tool-result format from `content/v1` to `tool-result/v1`, with no alias. Providers and consumers that declared `content/v1` switch to `tool-result/v1` together.

## 0.12.1

- Add `SYSTEM_TEXT_DEFAULT` (`system-text-default/v1`), a manifest promise to serve default system text without a preset.
- Replace version-based rollout advice with the joint provider/runner declaration rule; document the terminal admission refusal. Existing vector bytes are unchanged.

## 0.12.0

- A system-text item's `preset` is optional (contract §3). Without one, the
  provider answers with its default system text, as a `tool.catalog` request
  without a `preset` is answered with its default variant. A present preset
  keeps its meaning, including the refusal of one the provider does not define.
- Breaking: `SystemTextItem.preset` is `Option<String>`, omitted from the
  encoding when `None`. Migration: wrap existing presets in `Some(...)`, and
  treat `None` as the provider's default text. An item with a preset encodes
  byte for byte as before.
- Add `SystemTextItem::new` and `SystemTextItem::with_preset`, and derive
  `Default` for `SystemTextItem`.
- Rollout: a sender omits `system_text.preset` only to a provider on this
  release or later. An older provider refuses a missing preset as
  `invalid_params`, so senders adopt this after providers do.
- `catalog-requests.json` adds a system-text request with a preset and one
  without; existing vectors are unchanged.

## 0.11.0

- Moves to subc-protocol 0.33; take subc-protocol 0.33, subc-transport 0.14, subc-control 0.33 and subc-client-rs 0.31 together.
- Breaking dependency update: the public `ToolCallRequest`, `CallKeyError`,
  `OpaqueFieldError` and `CallMode` re-exports, `ErrorBody` in public signatures,
  and `ScopeStamp` in
  `user_command::admitted` now come from subc-protocol 0.33.
- No behaviour changes; existing test-vector bytes are unchanged.

## 0.10.6

- Define the `llm.classify/v1`, `llm.extract/v1` and `llm.complete/v1`
  capability tags for single-call model tools (contract §3). Unlike the other
  defined tags, each also promises the request and result shapes the
  `cortexkit-llm-tools` crate defines for it.

## 0.10.5

- Define the `tool.view/v1` capability tag: the tool has an interface view
  the apps can render for its calls, published in its module's `rich_ui`
  manifest descriptor and keyed by the tool's `schema_pin` (contract §3).
- State the rollout rule for newly defined tags: a reader on an older release
  refuses an unprefixed tag that release doesn't define, so a provider emits
  a new tag only once every reader of its catalog runs a release defining it.
- A test checks that the contract's tag table and `DEFINED_CAPABILITY_TAGS`
  list the same tags in the same order.

## 0.10.4

- `CatalogAnswer.presets` declares the presets a provider serves, in the
  answer to a `tool.catalog` request without a preset (contract §4.3). A
  caller sends `preset`, on `tool.catalog` or on a tool call, only to a
  provider that lists it; no list means the provider serves none. A provider
  serves every listed preset for as long as it lists it. Callers may keep
  their existing behaviour for providers that do not list presets until
  every provider serving presets declares them.
- Add `CatalogAnswer::with_presets`, `CatalogAnswer::lists_preset`,
  `catalog::check_presets` and `catalog::valid_preset_name`: names are
  1–32 bytes matching `^[a-z][a-z0-9_-]*$`, unique, at most 16 entries, and
  a list in the answer to a preset request is malformed.
- Additive: an empty list is omitted, so existing catalogs keep their bytes
  and digests; a present list is covered by `catalog_digest`. New
  `presets.json` vector; existing vectors are unchanged.

## 0.10.3

- Optional per-tool `reference_slots` with named validation, token/source
  kinds and an eight-slot bound. Empty lists preserve catalog bytes and digests.
- Shared slot vectors, including `show_ui` data and program slots.
- Clarify that a call's late-result custodian is always its carrier, including
  calls raised inside a scope another principal owns. No wire change.

## 0.10.2

- A `held` reply now means the call's answer comes later through
  `late_results`; the reason says why. New documented reason `running`: the
  call's work continues and no person needs to act. `consent`,
  `login_handoff` and `login_capture` remain the person-wait reasons, and a
  consumer prompts a person only for those. Unknown reasons are not person
  waits. Every held reply still needs a `call_key`, the `late_results`
  capability and both route-bind declarations. Additive: no wire change.
- Add `result::held_reasons` with a constant for each documented reason, and
  `result::is_person_wait`. `held-reply-v1.json` gains a `running` reply and a
  `person_wait` table.

## 0.10.1

- The contract states the reply shape: a `RESPONSE` body is the bare answer
  object, never wrapped in `result`, and a refusal's fields other than
  `code` and `message` live inside `detail`. New `reply-shape.json` vector.
  Nothing else changes.

## 0.10.0

- Breaking: exhaustive matches on `late_results::EntryKind` need a wildcard arm.

- Add person-only catalog commands, the reserved `user_command.invoke` op,
  bounded open-state replies, scope admission and named refusals. Catalog and
  late-entry builders remain source-compatible; absent members retain all old
  bytes and digests. Command declarations are covered by the catalog digest.
- Add non-terminal late `progress` text and command result shapes, plus shared
  command wire, bounds, digest and admission vectors.
- Mark `EntryKind` non-exhaustive so future entry kinds are additive.

## 0.9.3

- Ship all shared `tool-provider/v1` vectors inside the crate package, expose
  their bytes through the opt-in `test-vectors` feature, and pin every vector
  and the README with `SHA256SUMS`. This lets consumers use the published bytes
  without a repository checkout.
- Point the wire crate's vector tests at the feature module and verify the
  package contents match the embedded bytes and their checksums.

## 0.9.2

- Add `ToolResult::held`, `held()` and `with_held`, plus the non-exhaustive
  `HeldReply` type for person-gated interim replies. Unknown snake_case reasons
  and opaque object detail round-trip; validation rejects held/error conflicts,
  bad reasons and non-object detail (including explicit null).
- Add `HELD_REPLY_V1` and `consumer_reads_held_reply_v1`: held markers require
  both route-bind consumer declarations, a `content/v1` tool, a call key and
  provider `late_results` support. Legacy routes and final refusals are unchanged.
  The final answer arrives through `late_results`, not the interim content.
- Add held reply and gating vectors. This is additive: `ToolResult` was already
  non-exhaustive, so external struct-literal construction was already disallowed.

## 0.9.1

- Add optional per-tool `emits_events` (`CatalogTool::emits_events`,
  `with_emits_events`). `false` is the provider's promise that no call to the
  tool, with any arguments and any op, causes the provider to emit an event,
  published directly or relayed from upstream; `true` means it may; absent
  means unknown, which consumers treat like `true`. Absence emits no catalog
  bytes, so existing catalogs and digests are unchanged. An explicit `null`
  decodes as absent, like every optional member; any other non-boolean value
  does not decode. Additive: `CatalogTool` is already non-exhaustive.
- Add `test-vectors/tool-provider-v1/emits-events.json` with `false`, `true`,
  absent, `null` and refused non-boolean entries.

## 0.9.0

- Add optional catalog capability filtering, discoverable through the module
  capability and the sorted `tool_capabilities` union in `role.describe`.
  Filtering is additive; providers that do not advertise it are unchanged.
- Make `RoleDescribe` non-exhaustive and add `RoleDescribe::new` plus optional capability builders. Migration: replace `RoleDescribe { ... }` literals with `RoleDescribe::new(majors, implementation_version).with_capabilities(...).with_tool_capabilities(...)`.
- Add a consumer helper that sorts and deduplicates requested tags, and a
  provider helper that selects every catalog tool carrying any requested tag.
  Shared vectors pin accepted and refused `params.capabilities` requests.

## 0.8.1

- Contract wording: every `tool.catalog` params key is per-provider. A provider
  accepts only the keys it documents and refuses others, and a consumer sends
  each provider only its own plan item's params. `behavior`, `scope`,
  `tool_descs` and `exclude` are conventional names, not keys every provider
  must accept. No code or wire change.

## 0.8.0

- Requires subc-protocol 0.32. Consumers must upgrade together so the public
  `CallMode` and `ErrorBody` types come from the same protocol version; no
  source changes are otherwise required.

## 0.7.3

- Add a route-bind capability constant and helper for detecting consumers that
  declare support for `content/v1` result bodies.
- Require providers to choose a tool's `content/v1` or legacy result body from
  the route bind, and document the consumer's matching declaration.

## 0.7.2

- Define three more unprefixed capability tags, so a consumer can ask for
  the activity without naming a provider: `web.search/v1` (general web
  search), `github.read/v1` (read-only GitHub issues, pull requests and
  repository data) and `code.search.public/v1` (code search across public code
  hosts).
- Standardize the refusal for a tool called in a mode it does not serve:
  `call_mode_unsupported` (`errors::CALL_MODE_UNSUPPORTED`), detail
  `{tool, mode}`. Before, each provider chose its own code.
- `code.search/v1` now says what it always meant: it searches the session's
  own workspace. Code hosted elsewhere is `code.search.public/v1`.

## 0.7.1

- Add optional per-tool `result_format` and the open `ResultFormat` vocabulary
  with `content/v1`. Omission preserves all existing catalog bytes and digests.
- Add `ToolResult`, ordered `ContentPart` and `ImageContent` builders with
  MCP wire names, raw unknown-part preservation, and shared `validate` errors
  naming the member and reason. Check error-code coupling, image base64,
  media types, dimensions and IDs, nonempty content and the strict 3 MiB bound.
- Pin reply vectors and document consumer rendering: content only for model
  input, exact order, unsupported-part notes, and consumer-owned layouts.

## 0.7.0

- Requires subc-protocol 0.31. The re-exported tool-call and `CallMode` types
  make this a breaking dependency move: use 0.7.x with protocol 0.31, and
  0.6.x with protocol 0.30.
- Add optional per-tool `CatalogTool.modes`, `with_modes`, `check_modes` and
  `serves_call_mode`. Providers check the bind mode before every tool call;
  unknown modes are never served. Re-export `CallMode` and
  `CALL_MODES_CAPABILITY` for provider implementations.
- Normalize explicit `["classic"]` to absence when decoding and building;
  omit it on encoding even after direct assignment. Existing catalog bytes
  and digests stay unchanged. Add pinned catalog-content digests for the
  default and for nondefault mode lists, plus validation and unknown-mode tests.

## 0.6.0

- Requires subc-protocol 0.30. This crate re-exports subc-protocol's tool-call
  types (`ToolCallRequest`, `validate_call_key`, `validate_schema_pin` and their
  errors and constants), so a consumer's subc-protocol version must match: use
  0.5.x with subc-protocol 0.29 and 0.6.x with 0.30. Nothing else changed.

## 0.5.2

- Amend `tool-provider/v1` with optional per-tool reply deadlines. Absent
  metadata emits no new bytes; present metadata is catalog content, not part
  of the schema digest or pin.
- Add non-exhaustive `Reply` and `HoldArgument` types and builders,
  `check_reply`, and saturating, fallback-floored `reply_deadline_ms`.
- Add shared vectors and tests for the deadline arithmetic, for `check_reply`,
  and for unchanged catalog digests when no tool declares `reply`.
