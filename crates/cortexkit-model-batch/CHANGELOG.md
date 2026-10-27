# Changelog

## 0.3.0

Breaking: an item image may be a reference to an image another module
holds, so a caller need not copy its bytes into every request. The host
resolves the reference through the module that issued it, just before it
calls the model (`RULES.md` §13.1).

- New `Image::Reference(ImageReference)`, `ImageReference { handle }`. The
  handle is opaque: 1-128 characters of `[A-Za-z0-9._:-]`
  (`is_image_handle`, `ImageReference::check`, `IMAGE_HANDLE_MAX_LEN`).
- `Image` is now `#[non_exhaustive]`: a match outside this crate needs a
  wildcard arm.
- `InlineImage` and `ImageReference` refuse unknown members, so each JSON
  shape decodes as exactly one variant. Before, `{content_type, base64,
  handle}` decoded as an inline image and dropped the handle; now it, and any
  inline object with an extra member, is no image.
- `Image::decode` now returns `Option<Result<DecodedImage, String>>` and
  `Image::approx_bytes` returns `Option<usize>`: `None` for a reference,
  which has no bytes until it is resolved. An `Option` keeps "no bytes here"
  apart from a decode failure and from zero bytes, and makes every caller
  decide what a reference means to it instead of counting it as empty or
  malformed.
- `Image::canonical` now returns `CanonicalImage`: `Inline` (`{content_type,
  base64}`, as before for both byte-carrying spellings) or `Reference`
  (`{handle}`). Batch identity hashes a reference as its handle, never its
  resolved bytes, so a `batch_id` reuse check never needs an expired handle,
  and a reference and an inline copy of the same picture are different
  batches (`RULES.md` §2).
- A host that does not declare references keeps `check_images`, which now
  refuses a reference as an unsupported image (`invalid_params` naming
  `items[i].images[j]`, message `REFERENCE_UNSUPPORTED`). `check_image` and
  `check_image_with` refuse one the same way.
- A host that declares references runs `check_unresolved_images` (count,
  handles, inline images), `resolve_images` (inline images decoded,
  references resolved in order by the host's resolver, each failure a
  `ResolveFailure`), then `check_resolved_images`: the inline checks on
  every image's bytes, with the same refusals and paths. `check_decoded_with`
  checks one image's bytes; `DecodedImage::new` and `InlineImage::from_decoded`
  build the decoded form from resolved bytes and turn it back into the form
  a provider reads.
- `ImageReferenceSupport`: the `image_reference` group a role's
  `role.describe` states when the host resolves references (no members
  yet; later ones are additive).
- Two new admission codes, in `ADMISSION_VOCABULARY` and `admission_class`:
  `image_unavailable` (permanent: unknown, expired or not usable by this
  caller, one answer for all three) and `image_source_unavailable`
  (transient, with `detail.retry_after_ms`: the issuing module is
  unreachable or timed out). Both carry `detail.field`
  `items[i].images[j]`. Constructors `Refusal::image_unavailable` and
  `Refusal::image_source_unavailable`.
- Vectors: `vocabulary.json` lists the two codes; `images.json` adds a
  reference and its canonical form, refused handles, and mixed shapes that
  decode as no image.
## 0.2.1

- `RULES.md` now pins role-specific management routes and the capability
  check callers need before relying on them. Callers find which module serves
  a catalog model id by checking each module's `role.describe`; if none or
  more than one module lists the model, they refuse with permanent,
  caller-raised `model_unserved` or `model_ambiguous` errors.

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
