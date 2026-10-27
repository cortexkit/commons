# Changelog

## 0.2.1

- New refusal-spend rule (`CONTRACT.md` beside the billing rule): the
  serving module marks every refusal with exactly one spend marker.
  Measured (`detail.cost_usd`, charged exactly), `cost_unmeasured` (the
  code itself, charged at `max_cost_usd`), unknown (`detail.cost_unknown`
  is `true`, charged at `max_cost_usd`), or no spend (`detail.no_spend`
  is `true`, charged nothing). A serving module that cannot prove a
  failure happened before anything was sent marks it unknown, never free.
  A refusal with no marker is from a producer that does not follow the
  rule, and the caller applies its own policy to it. A repeat with the
  same call key returns the same refusal with the same spend members and
  is never billed twice.
- New `errors::refusal_spend(&Refusal) ->
  Result<RefusalSpend, SpendViolation>` checker used by the caller and the
  serving module, with `RefusalSpend::{Measured, ChargeCeiling, NoSpend,
  Undeclared}` (both `cost_unmeasured` and `cost_unknown` give
  `ChargeCeiling`) and the `COST_USD_KEY` / `COST_UNKNOWN_KEY` /
  `NO_SPEND_KEY` detail-key constants. All new types are
  `#[non_exhaustive]`. Requires `cortexkit-model-batch` 0.3.1 for the
  three detail members.
- New `refusal-spend.json` vectors with valid and invalid spend samples
  and their expected checker results; `SHA256SUMS` is refreshed.


## 0.2.0

- Depends on `cortexkit-model-batch` 0.3.0, `cortexkit-role-classifier`
  0.5.0 and `cortexkit-role-extractor` 0.2.0. The tool items embed the
  roles' `Item`, whose image may now be a reference `{handle}` (breaking:
  `Image` gains a variant and is non-exhaustive).
- `CONTRACT.md` states image references for `llm.classify/v1` and
  `llm.extract/v1`: a provider's tool input schema lists the `{handle}`
  form only in the release where it resolves references.
- New `classify::input_schema_with_image_references` and
  `extract::input_schema_with_image_references`, listing `{handle}`;
  `input_schema` is unchanged and does not list it.
- New `classify::validate_request_with_image_references` and
  `extract::validate_request_with_image_references`, which check a
  reference's handle. `validate_request` refuses a reference as an
  unsupported image, naming `item.images[j]`.
- The classify and extract code tables add `image_unavailable` (permanent)
  and `image_source_unavailable` (transient); `refusals.json` has one
  vector for each, and `SHA256SUMS` is refreshed.

## 0.1.2

- Clarify that `usage.input_tokens` excludes cache reads and writes. Document
  normalization of inclusive provider counts, deduplication of cache-write
  aliases, and separate cost pricing. Text only; wire shapes and validation
  are unchanged.

## 0.1.1

- New permanent failure `cost_unmeasured` for all three tools (`errors::COST_UNMEASURED`, listed in `errors::TOOL_RESULT_CODES`). A provider answered, but nothing gives the billed cost, so the answer is withheld instead of returned with a guessed cost. Usage is recorded as reported, the caller charges the request's `max_cost_usd` against its budget, and a retry with the same call key replays the failure. Additive: no existing code or shape changes.

## 0.1.0

- The `llm.classify/v1`, `llm.extract/v1` and `llm.complete/v1` tool shapes, common admission refusals, billing rules, test vectors and validation.
