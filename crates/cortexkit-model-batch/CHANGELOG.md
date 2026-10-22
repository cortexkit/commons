# Changelog

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
