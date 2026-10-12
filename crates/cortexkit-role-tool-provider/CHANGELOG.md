# Changelog

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
