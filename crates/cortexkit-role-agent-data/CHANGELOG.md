# Changelog

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
