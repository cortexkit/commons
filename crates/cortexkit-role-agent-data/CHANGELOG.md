# Changelog

## 0.2.4

- Add optional per-item `dropped` counts and the validating `with_dropped`
  builder. This is a compatible patch: `ImportedItem` is already
  `#[non_exhaustive]` and built through `new`, whose signature is unchanged.
- Validate the non-empty map's 16-reason cap, 1–64 byte lowercase ASCII
  reason grammar, and positive counts up to 2^53 - 1, naming violated rules.
- Require moves with any reported drops to be shown as completed with losses,
  listing participant-defined reasons and counts without branching on them.
  Roll out coordinator readers before participants send `dropped`.
- Add drop-report validation vectors to the bundled `test-vectors` kit.
  No separate agent-data conformance or test-kit crate needs a bump. The wire
  version remains `agent-data/v1`; items without drops keep their previous bytes.

## 0.2.3

- Use the shared helper's named `SessionIdentity` fields; runner ids are bare
  attested module ids, not principal display forms. Wire fields are unchanged.
- Session refs now use the shared llm-runner 0.7.0 derivation from runner module
  id, harness and session, not the project folder. Refresh the reference/reply
  vectors and document one-time live-session re-keying; ended refs stay v1.
- Update the llm-runner dev-dependency to 0.7.0. Public wire types are unchanged.

## 0.2.2

- Dev-dependency on cortexkit-role-llm-runner 0.4.0 (was 0.3.0), for the
  `session_ref` vector test. Nothing changes for callers: no public type or
  signature names an llm-runner type, and the wire is unchanged.

## 0.2.1

- Dev-dependency on cortexkit-role-llm-runner 0.3.0 (was 0.2.1), for the `session_ref` vector test. Nothing changes for callers: the published API and the wire are unchanged.

## 0.2.0

- Breaking: `DivergedEntry` is no longer `Copy` and gains an optional,
  validated human-readable `reason` for `unknown` and `advanced` replies.
- Add diverged-reason validation vectors; the wire version remains
  `agent-data/v1` and entries without a reason keep their previous bytes.

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
