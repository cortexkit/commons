# Changelog

## 0.1.0 (unreleased)

First version of the live usage step feed contract, v1: `usage.steps.subscribe`
and `usage.steps.read`, the row and its last-row-wins identity, the verdict
rules and thresholds, refusal and ending codes, `LatestRows`, and the
`usage-steps-v1` vectors.

- A row's `output` includes reasoning. `reasoning` is a breakdown of `output`,
  never an addition to it, so `reasoning <= output` whenever both are present.
  This is the rule the runner contract pins for `reasoning_tokens`.
- `StepRow::check_counts` names a row whose `reasoning` exceeds its `output`,
  for the provider's own tests. Readers still decode such a row.
- `live_row_identity` gives the `(harness, step_ref)` identity of a step a
  runner announced, from the step event's members.
- A subscription the caller cancels may end with a last Error `cancelled`
  (`codes::CANCELLED`, `Refusal::cancelled`), which the daemon may drop. A
  reader treats it, after its own Cancel, as the normal end.
