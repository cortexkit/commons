# Changelog

## 0.1.3

- Contract wording only (§7.7), no code, wire or vector change: an import's
  receiver decides `incompatible_version` from the cut's authenticated bytes,
  reading only as far as it needs to decide, because `import` carries no
  `format`. It stages privately and installs nothing on that refusal.

## 0.1.2

- Allow `digest_mismatch` on `activate` and `abort` when a participant's own
  staged bytes or undo images no longer match their recorded digests.
- Add `errors::driver_action_for(op, code)`: activation holds as a fork for
  the operator without retrying or returning the claim; abort waits for undo
  repair before retrying, leaving the move stuck and the source agent frozen.
  The existing `RefusalCode::driver_action()` remains unchanged.
- Add optional `DigestMismatchDetail` with open `reason` strings and known
  reason constants, plus separate `ErrorBody` construction and decoding
  methods. The five required `Detail` variants remain source-compatible.
- Ship additive `digest-mismatch.json` refusal and per-op driver-action
  vectors. Existing vector bytes and checksum entries remain unchanged.

## 0.1.1

- Document the closed set of `sync_class` values (`agent`,
  `shared_one_writer`, `shared_merged`, `machine_local`) in CONTRACT.md §7.1,
  and add `describe::sync_class` constants for their spellings. The wire is
  unchanged: `sync_class` stays a plain string.

## 0.1.0

- First version of the `agent-data/v1` contract: CONTRACT.md, typed requests
  and replies for the ten ops, the closed refusal codes with their driver
  actions and an `Unknown` variant that is never retried, the `cutId` and
  ref checks with the 256-ref cap and duplicate refs refused, the
  per-`cutId` step machine, the one-ref-set-per-`cutId` rule
  (`step::check_ref_set`), and the whole-request idempotency comparison,
  with refs compared as a set.
- Which op may refuse with which code (`errors::allowed_refusals`); `busy`
  only on `prepare`, `import` and `activate`; `cut_not_ready` and
  `incompatible_version` on `import` and `diverged`.
- `local_advanced`: the receiver's copy gained data after `diverged`
  answered `at_cut`. Allowed on `import` and `activate`; terminal for the
  step and never retried (`DriverAction::TerminalForStep`). The `release` reply is
  `{enforced: bool}`, `enforced` required.
- Test vectors ship inside the crate under `test-vectors/agent-data-v1/`,
  pinned by `SHA256SUMS`; the `test-vectors` feature exposes them as the
  `vectors` module.
