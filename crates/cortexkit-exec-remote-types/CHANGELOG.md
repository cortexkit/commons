# Changelog

## 0.1.1

- Every enum in a caller reply or stream record now preserves unrecognised tags
  in a documented catch-all: `StreamRecord`, `Killed`, `Ran`, `OutputStream`,
  `PrepareOutcome`, and `RebuildResult` join `RefusalReason` and `Outcome`.
- Unknown stream records retain an optional `u64` sequence number for advancing
  the caller's resume cursor. A present, non-`u64` sequence remains a decode error.
  Callers skip these records, never treat them as terminal, and seek the outcome
  through status or attach if the stream ends without a known terminal record.
- Known tags with malformed or missing required fields remain decode errors;
  known wire shapes are unchanged. Six new crate-local vector pairs pin unknown
  tags and their canonical round-trips.
- `Killed`, `Ran`, `OutputStream`, and `RebuildResult` are still `Clone`, but no
  longer `Copy` because their catch-alls own a `String`.

## 0.1.0

- Initial caller-facing `exec-remote/v1` request, reply, and stream types, with
  raw-byte output, explicit nullable availability fields, and non-exhaustive
  public records and enums.
- Catch-alls for refusal reasons and terminal outcomes, with conservative
  caller grading and strict validation of malformed known outcomes.
- Packaged canonical JSON and SHA-256 golden vectors with contract tests.
