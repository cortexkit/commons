# cortexkit-llm-tools

Shared wire contracts for `llm.classify/v1`, `llm.extract/v1` and
`llm.complete/v1`. These are tool-provider tools for codemode programs and flows,
not tools on an agent's direct list. This crate makes no model calls.

Each of `classify`, `extract` and `complete` exports `TAG`, `Request`, `Result`,
`input_schema()`, `result_schema()`, `validate_request()` and `validate_result()`.
The schemas are JSON objects suitable for a provider's `tool.catalog` entry.
`MODES` is `["codemode", "flow"]`. `Usage` and `ValidationError` are shared.
`complete` also exports `Demand`, `Model`, `OUTPUT_SCHEMA_MAX_BYTES`, and
`validate_result_for_request()` to check the output branch against the request.
`errors` reuses the role error types/codes, defines common admission and
completion-only codes, and exposes their classes and a refusal constructor.

The classifier and extractor reuse the roles' `Item`, `Question`/`Field`,
`Options` and `Answer` types rather than translating their contents. Their
questions/fields still form maps; **only the item count becomes one**.
All three requests require `max_cost_usd`, the caller's remaining budget; the
underlying role types are unchanged.

Read [CONTRACT.md](CONTRACT.md) for replay, scope, billing, trust and validation
obligations. Validators check catalog-independent invariants, not routing,
provider limits, scope authority or arbitrary JSON Schema evaluation. The serving
module supplies those checks. No JSON Schema engine is a runtime dependency.

Enable `test-vectors` to consume `vectors::{FILES, SHA256SUMS}` and the per-file
constants. The package ships the same invented-example JSON files under
`test-vectors/llm-tools-v1/`.

```sh
cargo test --locked -p cortexkit-llm-tools --all-features
```
