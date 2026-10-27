# Changelog

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
