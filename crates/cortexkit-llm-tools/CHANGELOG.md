# Changelog

## 0.1.1

- New permanent failure `cost_unmeasured` for all three tools (`errors::COST_UNMEASURED`, listed in `errors::TOOL_RESULT_CODES`). A provider answered, but nothing gives the billed cost, so the answer is withheld instead of returned with a guessed cost. Usage is recorded as reported, the caller charges the request's `max_cost_usd` against its budget, and a retry with the same call key replays the failure. Additive: no existing code or shape changes.

## 0.1.0

- The `llm.classify/v1`, `llm.extract/v1` and `llm.complete/v1` tool shapes, common admission refusals, billing rules, test vectors and validation.
