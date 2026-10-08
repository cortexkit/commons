# Changelog

## 0.5.2

- Amend `tool-provider/v1` with optional per-tool reply deadlines. Absent
  metadata emits no new bytes; present metadata is catalog content, not part
  of the schema digest or pin.
- Add non-exhaustive `Reply` and `HoldArgument` types and builders,
  `check_reply`, and saturating, fallback-floored `reply_deadline_ms`.
- Add shared vectors and tests for the deadline arithmetic, for `check_reply`,
  and for unchanged catalog digests when no tool declares `reply`.
