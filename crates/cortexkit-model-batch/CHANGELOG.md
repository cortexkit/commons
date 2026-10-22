# Changelog

## 0.2.0

Breaking: the cost estimate follows the billing measured on Cloudflare's
Clairic (a live probe of 2026-10-22), not a pass count derived from
the per-pass token budget.

- `estimate::estimate_item` now bills `schema + max_passes x (state +
  images x image_tokens_each + overhead)` at the input rate: the schema
  once, the state on every pass, images and overhead on every pass (an
  overstatement; per-pass image billing is not measured). 0.1.0 billed
  `(state + schema + images + overhead) x passes` with `passes = ceil(schema
  / (pass_tokens - state))`, which underestimates whenever the provider
  makes more passes than the budget predicts: probe 05's 40 small fields
  ran in 40 passes, where the budget predicts 4.
- New `ItemCostInput::max_passes` and `with_max_passes`: the module's upper
  bound on the provider's passes, default 1 (right for a one-pass
  provider). Zero is refused with the new `EstimateError::NoPasses`.
- `pass_tokens` is used only for the fit check: an exact state at or past
  it is still `EstimateError::DoesNotFit`.
- Removed `estimate::passes` and `Estimate::Unbounded`. A byte-bound state
  at or past `pass_tokens` is estimated like any other state. `Estimate` is
  now a struct, `{max_passes, input_tokens, cost_nanos}`, with
  `Estimate::new`; `crosses_ceiling` takes it as before.
- `RULES.md` §5 states the measured model and where each term comes from;
  `estimates.json` carries probe 05's figures (exact counts estimate
  exactly the 29,888 input tokens billed; byte bounds at least that).

## 0.1.0

- First release: the batch rules `classifier/v1` and `extractor/v1` share,
  stated once in `RULES.md`, with the code that implements them.
- Moved here from `cortexkit-role-classifier` 0.1.0, which re-exports each
  item at its old path: the error codes, `ErrorClass`, `ItemError`,
  `RefusalDetail`, `Refusal` and its constructors; `canonical_json`,
  `digest` and `IdentityError`; the image shape (`Image`, `InlineImage`,
  `content_types`, the size constants, `check_image`); `is_batch_id`,
  `BATCH_ID_MAX_LEN` and `effective_ceiling`; `item_code_for_status`.
- New: the Workers AI code table (`status::workers_ai_item_code`) and the
  codes it adds, `account_limit_exhausted` (transient) and
  `account_not_entitled` (permanent), both batch-level: never stored, they
  stop the call. New per-item code `answer_too_large` (permanent, stored,
  with `detail.field`, `detail.limit`, `detail.size`), and
  `model_not_extractor` for `extractor/v1`.
- New: image validation decodes the base64 and checks decoded sizes;
  `Image::canonical`, and `identity::item_digest`, which hashes an item's
  images in canonical form; `ImageLimits` and `check_images`.
- New: `admission::CatalogRow`, three cases, so a row with no kind is never
  mistaken for a missing row; `check_batch_id`, `bounded`,
  `check_request_size`, `check_max_cost_usd`.
- New: the conservative cost estimate (`estimate`), in integer nanodollars
  with checked arithmetic; `absent` (absent is not zero); `number` (provider
  number text as `RawValue`).
- Vectors in `test-vectors/model-batch-v1/`, behind the `test-vectors`
  feature.
