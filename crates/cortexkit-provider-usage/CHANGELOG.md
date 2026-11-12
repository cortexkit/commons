# Changelog

## 0.16.0

- `SavedResets` gains `kinds`: the same saved resets grouped by what a reset
  restores, each with a count and soonest expiry (`SavedResetKind`). It is
  serialized as `kinds` and omitted when empty, which means the upstream states
  no kind. When present, the counts add up to `availableCount`.
- New `saved_reset_kind` vocabulary: `session`, `weekly`, `full`,
  `model_weekly`. The list is open: an unknown kind decodes intact, and a
  consumer must render it.
- Minor bump: `SavedResets` gains a public field, so code that builds it with a
  struct literal must add `kinds`. A body from before this release still
  decodes.
