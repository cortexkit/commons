# Changelog

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
