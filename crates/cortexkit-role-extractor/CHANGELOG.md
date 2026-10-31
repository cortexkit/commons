# Changelog

## 0.2.1

- Requires subc-protocol 0.34 for development checks only; no API or wire change.

## 0.2.0

Breaking, with `cortexkit-model-batch` 0.3.0: an item image may be a
reference `{handle}` to an image another module holds (RULES §13.1,
CONTRACT §3).

- `image::Image` gains `Reference(ImageReference)` and is now
  `#[non_exhaustive]`; `InlineImage` refuses unknown members, so an object
  mixing both forms is refused naming `items[i].images[j]`. `Image::decode`
  and `Image::approx_bytes` return `None` for a reference, and
  `Image::canonical` returns `CanonicalImage`. `image` re-exports the
  reference types and checks (`ImageReference`, `CanonicalImage`,
  `ResolveFailure`, `resolve_images`, `check_unresolved_images`,
  `check_resolved_images`, `check_decoded_with`, `is_image_handle`,
  `IMAGE_HANDLE_MAX_LEN`, `REFERENCE_UNSUPPORTED`).
- `role.describe` gains the optional group `image_reference: {}`
  (`RoleDescribe::image_reference`, `with_image_reference`,
  `declares_image_reference`, `describe::ImageReferenceSupport`). A module
  that resolves references states it; a caller sends a reference only to a
  module that does.
- `check_request` refuses a reference as an unsupported image
  (`invalid_params` naming `items[i].images[j]`). A declaring module uses
  `check_request_with_references`, then `resolve_item_images` and
  `check_resolved_item_images`, so resolved bytes get exactly the inline
  checks. `admit` picks between the two by the describe's group.
- `provider::check_provider_body` checks one provider request's size, and
  `ProviderRequest::with_resolved_images` puts resolved bytes in place of
  the item's images, so a declaring module checks the body it actually
  sends. `check_provider_bodies` is unchanged and uses it.
- New admission codes in `ADMISSION_CODES`: `image_unavailable`
  (permanent) and `image_source_unavailable` (transient, with
  `detail.retry_after_ms`), both naming `items[i].images[j]`.
- Body identity hashes a reference as `{handle}`, never its resolved bytes.
- `parse_request` names `{handle}` among the image forms in its refusal.
- Vectors: `admission-refusals.json` adds one refusal for each new code;
  `SHA256SUMS` is refreshed.

## 0.1.2

- Moves to subc-protocol 0.33; take subc-protocol 0.33, subc-transport 0.14, subc-control 0.33 and subc-client-rs 0.31 together.
- Patch release: subc-protocol is used only by the capability-identifier parity
  test, not the public API.
- No behaviour changes; existing test-vector bytes are unchanged.

## 0.1.1

- `CONTRACT.md` now directs callers to the shared rules for role-specific
  management routes and model-id discovery. A caller selects the serving
  module by checking which matching role's `role.describe` lists the model,
  not from a catalog row's serving-module field.

## 0.1.0

Reply side, pinned from a live probe of Clairic on 2026-10-22:

- `CONTRACT.md` §5 pins every answer type's members, as Clairic returned
  them: `time` (`"09:15:00"`), `datetime` (`"2026-03-06T16:40:00"`, no
  offset), `duration` with `unit`, a list of numbers with `confidences` and no
  `spans`,
  `estimate` with 99 `quantiles` and no `p_null`, `forecast` with parallel
  arrays and `steps[]`. `ANSWER_MEMBERS` adds `unit` and `confidences`;
  decoding stays open.
- Spans count Unicode scalar values, end exclusive, and `state_text` is the
  caller's `state` byte for byte. Spans are still forwarded untouched; the
  new `reply::span_byte_range` turns one into a UTF-8 byte range of the
  state.
- §8 pins the envelope. An error reply puts `errors` first and carries
  `result: {}`, which `Envelope` now decodes as no result, in any member
  order. `ProviderResult` gains `model`. New `provider::read_reply` reads a
  provider call's status and body into a result or the mapped item error
  (5006 at 400 and 3030 at 422 map by status to `invalid_item`), and
  `ProviderResult::into_item` makes the item.
- A date estimate (`min` and `max` as ISO dates with `step: month`) is
  admitted; the `year` step and date bounds on a forecast stay refused.
- The spend check bounds the passes by the field count
  (`cortexkit-model-batch` 0.2.0's `max_passes`): the schema bills once
  and the state on every pass. A byte-bound state past `pass_tokens` is now
  estimated like any other, so under a ceiling it is sent while it fits.
- New vector `provider-replies.json`: all twelve probe replies, status and
  body byte for byte, each with the item it reads as. `spend.json`,
  `requests.json` and `field-paths.json` follow the changes above.
- §12 keeps only what the probe left open: the `year` step, per-pass image
  billing, the cost of images larger than 64x64, image spellings and
  count, and the provider's bound on `instructions` above 20,000
  characters.

Request side:

- First version of the `extractor/v1` role contract, request side:
  `role.describe`, the `extract.run` request with its nine field types and
  their closed option table, admission in a pinned order (`request::admit`),
  the reply-size check against the daemon's 4 MiB frame, the pre-call spend
  check with the shared conservative estimate, batch identity (model,
  fields, options in force, items with canonical images), and the Clairic
  request mapping.
- The batch rules it shares with `classifier/v1` come from
  `cortexkit-model-batch` 0.2.0: the error vocabulary and the Workers AI
  code table, images, identity hashing, the estimate, absent-is-not-zero
  and number text.
- Answers keep each named member as the provider's text (`RawValue`) and
  every other member as given.
