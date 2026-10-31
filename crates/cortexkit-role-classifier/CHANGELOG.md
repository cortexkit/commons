# Changelog

## 0.5.1

- Requires subc-protocol 0.34 for development checks only; no API or wire change.

## 0.5.0

Breaking, with `cortexkit-model-batch` 0.3.0: an item image may be a
reference `{handle}` to an image another module holds (RULES §13.1,
CONTRACT §3).

- `question::Image` gains `Reference(ImageReference)` and is now
  `#[non_exhaustive]`; `InlineImage` refuses unknown members, so an object
  mixing both forms is refused naming `items[i].images[j]`. `Image::decode`
  and `Image::approx_bytes` return `None` for a reference, and
  `Image::canonical` returns `CanonicalImage`. `question` re-exports
  `ImageReference`, `CanonicalImage`, `DecodedImage`, `ResolveFailure`,
  `check_decoded_with`, `is_image_handle`, `IMAGE_HANDLE_MAX_LEN` and
  `REFERENCE_UNSUPPORTED`.
- `role.describe` gains the optional group `image_reference: {}`
  (`RoleDescribe::image_reference`, `with_image_reference`,
  `declares_image_reference`, `describe::ImageReferenceSupport`). A module
  that resolves references states it; a caller sends a reference only to a
  module that does.
- `check_request` refuses a reference as an unsupported image
  (`invalid_params` naming `items[i].images[j]`): the answer of a module
  that does not declare `image_reference`. A declaring module uses
  `check_request_with_references`, then `resolve_item_images` and
  `check_resolved_item_images`, so resolved bytes get exactly the inline
  checks, with the same refusals and paths.
- New admission codes in `ADMISSION_CODES`: `image_unavailable`
  (permanent) and `image_source_unavailable` (transient, with
  `detail.retry_after_ms`), both naming `items[i].images[j]`.
- Body identity hashes a reference as `{handle}`, never its resolved bytes,
  so a reference and an inline copy of the same picture are different
  batches.
- `parse_request` names `{handle}` among the image forms in its refusal.
- Vectors: `admission-refusals.json` adds one refusal for each new code;
  `SHA256SUMS` is refreshed.

## 0.4.3

- Moves to subc-protocol 0.33; take subc-protocol 0.33, subc-transport 0.14, subc-control 0.33 and subc-client-rs 0.31 together.
- Patch release: subc-protocol is used only by the capability-identifier parity
  test, not the public API.
- No behaviour changes; existing test-vector bytes are unchanged.

## 0.4.2

- Added `provider::models::JEV_1_13_0` (`"jev-1.13.0"`), the `model` value
  for catalog row `typesafe/jev-1.13.0`. The crate offered only
  `JEV_LATEST` (`"jev-latest"`), TypeSafe's moving alias, and the contract
  and vectors paired the pinned row with it, so a runner following them
  sent the alias. When TypeSafe moves the alias, the row's pricing and
  behaviour no longer match the model that answers. TypeSafe accepts
  versioned ids in `model` and names the answering version in the
  response's `model`.
- `JEV_LATEST` stays, documented as the alias for callers that do not pin
  a version; a runner serving a catalog row never sends it.
- CONTRACT.md §9 maps the row to `"jev-1.13.0"` and adds a rule: a runner
  sends the pinned `model` value of the row it serves, never a moving
  alias, and when a response's `model` names a different model than the
  one sent, it keeps the answers and logs the mismatch with both values.
- Vectors: every TypeSafe provider request and response `model` is now
  `"jev-1.13.0"`, `provider-examples.json` no longer names the alias in its
  `about`, and `SHA256SUMS` is refreshed. No type or wire shape changed.

## 0.4.1

- `CONTRACT.md` now directs callers to the shared rules for role-specific
  management routes and model-id discovery. A caller selects the serving
  module by checking which matching role's `role.describe` lists the model,
  not from a catalog row's serving-module field.

## 0.4.0

- Breaking: `request::check_catalog_row` takes the shared `CatalogRow`
  (re-exported from `request`) instead of `Option<&str>`, because an
  `Option` could not tell a missing row from a row that states no kind. No
  catalog row is `model_unknown`; a row with no `kind` is
  `model_not_classifier` with `detail.model` and no invented kind, as the
  shared batch rules already said. Added a matching test vector.

## 0.3.0

- Depends on `cortexkit-model-batch` 0.2.0, whose estimator changed
  (`RULES.md` §5: the schema bills once, the state on every pass, up to a
  module's `max_passes`). This crate re-exports `cortexkit-model-batch`
  items at their `classifier/v1` paths (`ItemError`, `Refusal`, the image
  shape and the rest), and those are now 0.2.0's types, so the bump is
  breaking for a consumer that names both crates. Nothing else changed:
  `classifier/v1` does not use the shared estimate (its spend guard's
  estimate is the module's own, CONTRACT.md §6.9), and every vector keeps
  its bytes.

## 0.2.0

Breaking, in behaviour: two validation and identity changes, and four clef
error mappings. No wire shape changes, and every 0.1.0 vector passes byte
for byte.

- The batch rules this role shares with `extractor/v1` moved to the new
  crate `cortexkit-model-batch`, whose `RULES.md` states them once;
  `CONTRACT.md` points to it. Every moved item keeps its old path through a
  `pub use`: `errors::{INVALID_PARAMS, BATCH_ID_REUSE, MODEL_UNKNOWN,
  MODEL_NOT_CLASSIFIER, COST_EXCEEDED, ACCOUNT_WALLED, BATCH_IN_PROGRESS,
  AUTH_FAILED, MODEL_UNAVAILABLE, RATE_LIMITED, PROVIDER_ERROR,
  PROVIDER_REFUSED, INVALID_ITEM, admission_class, item_class, classes,
  ErrorClass, ItemError, RefusalDetail, Refusal}`,
  `identity::{canonical_json, digest, IdentityError}`,
  `question::{Image, InlineImage, content_types, IMAGE_MAX_BYTES,
  IMAGES_MAX_TOTAL_BYTES, IMAGE_MAX_PIXELS, check_image}`,
  `request::{BATCH_ID_MAX_LEN, is_batch_id, effective_ceiling}` and
  `provider::item_code_for_status`. `ADMISSION_CODES`, `ITEM_CODES`,
  `BatchIdentity` and the rest stay this crate's own.
- **Image validation decodes the base64.** 0.1.0 read only the base64's
  length, so malformed base64 reached the provider, against §6.7. Now
  malformed base64, a `data:` URL without `;base64`, and an image of no
  bytes are refused `invalid_params` naming `items[i].images[j]`, and image
  sizes are decoded bytes.
- **Batch identity hashes images in canonical form** (content type and
  decoded bytes). 0.1.0 hashed an image as written, so one image sent as a
  `data:` URL and re-sent as `{content_type, base64}` was a false
  `batch_id_reuse`. Migration cost: a batch recorded under 0.1.0 with image
  items, re-sent after the upgrade, is refused `batch_id_reuse`; re-send it
  under a new `batch_id`. Batches without images keep their identity.
- **clef's errors map by Workers AI code first**
  (`provider::workers_ai_item_code`), so they say what the account or model
  did rather than what the HTTP status suggests. Changed, old to new:
  - 429 `3036` (daily free allocation used): `rate_limited` to
    `account_limit_exhausted`. 0.1.0 stored `rate_limited` on the item when
    the whole account hit its limit, so a re-send after the reset got the
    stored error back instead of an answer. The new code is batch-level:
    never stored, it stops the call, and on the call's first provider call
    it refuses the call at admission.
  - 403 `5035` (needs a paid plan), `5016` (model agreement), `3023`
    (account blocked): `auth_failed` to `account_not_entitled`, permanent
    and batch-level. The credentials were fine; the account may not use the
    model.
  - 400 `5007` (no such model): `invalid_item` to `model_unavailable`.
    0.1.0 stored a permanent `invalid_item`, which tells a caller to drop a
    good item, for a model-level failure.
  - Unchanged: 404 `3042` stays `model_unavailable`, 429 `3040`
    `rate_limited`, 413 `3006` `invalid_item`, 408 `3007` and `3008`
    `provider_error`. TypeSafe's status mapping is unchanged.
- New in the vocabulary, re-exported: `ACCOUNT_LIMIT_EXHAUSTED`,
  `ACCOUNT_NOT_ENTITLED`, `BATCH_LEVEL_CODES`, `is_batch_level`,
  `ANSWER_TOO_LARGE` (never answered by this role, which states no answer
  bound) and `ItemErrorDetail`; `ItemError` has an optional `detail`,
  absent on every code this role answers with.
- New vectors: `image-identity.json` and `workers-ai-codes.json`.
- `CONTRACT.md` §12 lists two open readings: a catalog row that states no
  kind, and adopting the shared cost estimate.

## 0.1.0

- First published release of the `classifier/v1` role contract: the
  `role.describe` and `classify.run` wire types, the providers' shared
  question and answer shapes, the batch body identity, error codes and their
  classes, the request validators, and `CONTRACT.md`. Stability: alpha.
- The role's vectors ship inside the crate package under
  `test-vectors/classifier-v1/`, pinned by `SHA256SUMS`. The opt-in
  `test-vectors` feature compiles them into the `vectors` module
  (`vectors::FILES`, plus a constant per file), so a classifier module's
  tests can use the same bytes without a repository checkout.
- Catalog ids are models.dev's own provider keys and pin a model version:
  `cloudflare-workers-ai/@cf/cloudflare/clef`,
  `cloudflare-workers-ai/@cf/cloudflare/clef-flash` and `typesafe/jev-1.13.0`.
- A `role.describe` model entry's `context_tokens` is optional: it is absent
  when the catalog states no exact figure, and absent is not zero.
- `ErrorClass` is non-exhaustive. Error codes are open strings, so later
  codes are additive.
