# Changelog

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
