# Changelog

## 0.2.3

- Add optional applied and provider-confirmed served service tiers to step rows.
  A reported wire is a runner discovery entry with `served: "reported"` and
  permits served metadata only when that response states it (runner CONTRACT §9.3.2).
  Prefer a provider-stated served tier; the price stays unconfirmed if its tier
  or rate is unrecognised. Without served, applied is only an unconfirmed estimate,
  defaulting to standard when absent. Keep unfamiliar strings; never invent a rate.
- Keep tier records as strings, with dev-only drift checks against runner
  0.7.36 and producer validation of applied spellings. Existing rows and the closed
  step-finished bus event keep their bytes when no tier is supplied.
  Producers emit fast/ultrafast, omit standard and validate with
  `StepRow::check_closed`; usage-steps needs no runner crate at runtime.

## 0.2.2

- Clarify that `input` excludes cache reads and writes, matching the runner's
  disjoint input counters. Document normalization of inclusive source counts,
  deduplication of cache-write aliases, and separate cost pricing. Text only;
  wire shapes and validation are unchanged.

## 0.2.1

- `codes::SUBSCRIBE_REFUSALS` and the contract's subscribe opening refusals
  now include `invalid_request`, which the session-filter pairing rule
  already sends when a subscribe request names `harness` without `session`,
  or `session` without `harness`. The wire behaviour is unchanged.
- The rejected-vector test checks each request's refusal code against its own
  operation's list instead of the union of both, which is what hid the gap.

## 0.2.0

Contract version 1. Adds an optional session filter to both operations, so a
client can follow one session's steps without paging every producer's rows,
and keys a runner step without a host record on the runner's `attempt_id`.
The session filter itself is additive: an unfiltered request encodes as it did
in 0.1.0, and its vectors are unchanged. The identity change is breaking.

- Breaking: `live_row_identity(runner, harness, harness_step_ref, attempt_id)`
  returns `Option<(String, String)>`. Without a host record, `step_ref` is the
  event's `attempt_id` (llm-runner 0.7.3), not `<run_id>#<step_id>#<attempt>`,
  because that triple can repeat after a session is restored. An event with
  neither `harness_step_ref` nor `attempt_id` gives `None`.

- `ReadRequest` and `SubscribeRequest` carry optional `harness` and `session`
  members, omitted from the encoding when absent and accepting `null`, and a
  `for_session(harness, session)` builder that sets both.
- The pair comes together or not at all. The request types still decode either
  one alone, like any `limit`; `check_pairing` on either type reports the one
  that is missing, and the provider refuses such a request `invalid_request`.
- A filtered read returns only rows whose `harness` and `session` equal the
  request's; `next` and `through` keep their meaning, `through` still the
  newest committed cursor of the whole feed. `limit` bounds the rows returned.
- A filtered subscription sends a notice only when a row for that session is
  committed; coalescing and the `lagged` and `cancelled` endings are unchanged.
- An unfiltered request encodes exactly as in 0.1.0; its vectors are
  unchanged. A provider that predates the filter refuses a filtered request
  `malformed`, and a client must never treat unfiltered rows as that session's.
- `CONTRACT.md` documents the filter, the `invalid_request` pairing refusal and
  the rollout rule; the vectors add a filtered read request, a filtered
  subscribe request, an empty filtered reply, and one single-member request
  per operation.

## 0.1.0

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
