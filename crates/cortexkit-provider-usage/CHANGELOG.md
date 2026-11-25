# Changelog

## 0.17.0

- `RateWindow` gains `model_scope`, serialized as `modelScope` and omitted when
  unset: which of the account's models draw on the window, as far as the
  upstream says. `all` means every model counts against it; `subset` means
  only some do and the reading doesn't list them, so the window's identifier
  and title are the only names for that group. Absence means the producer
  didn't say, never `all`.
- New `model_scope` vocabulary: `all`, `subset`. The list is open: an unknown
  value decodes intact, and a consumer must handle it.
- Minor bump: `RateWindow` gains a public field, so code that builds it with a
  struct literal must add `model_scope`. A body from before this release still
  decodes.

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
