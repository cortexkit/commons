# Changelog

## 0.1.0

- First version of the `agent-data/v1` contract: CONTRACT.md, typed requests
  and replies for the ten ops, the closed refusal codes with their driver
  actions and an `Unknown` variant that is never retried, the `cutId` and
  ref checks with the 256-ref cap and duplicate refs refused, the
  per-`cutId` step machine, the one-ref-set-per-`cutId` rule
  (`step::check_ref_set`), and the whole-request idempotency comparison,
  with refs compared as a set.
- Test vectors ship inside the crate under `test-vectors/agent-data-v1/`,
  pinned by `SHA256SUMS`; the `test-vectors` feature exposes them as the
  `vectors` module.
