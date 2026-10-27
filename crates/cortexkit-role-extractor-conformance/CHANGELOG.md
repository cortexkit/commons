# Changelog

## 0.3.0

- Depends on `cortexkit-role-extractor` 0.2.0, re-exported as `wire`, whose
  item image may be a reference `{handle}` (breaking: `Image` gains a
  variant and is non-exhaustive).
- New optional subject hook for image references:
  `ExtractorSubject::issue_image_reference` (hand bytes to the image source
  the module resolves through, get the handle back) and
  `set_image_source_down`. Both have defaults, so a subject without the
  hook keeps compiling.
- Five new checks: `image_reference_follows_declaration` (an unissued
  handle is `image_unavailable` on a module that declares
  `image_reference`, `invalid_params` on one that does not),
  `image_reference_resolved_and_answered` (the stand-in receives the
  image's bytes, never the handle),
  `image_reference_oversized_refused_like_inline` (the inline refusal at
  `items[0].images[0]`), `image_source_down_refused_transient` (with
  `retry_after_ms`), and `image_reference_identity_is_its_handle` (the same
  handle is not `batch_id_reuse`; the same picture inline is, naming
  `items[0]`). The last four are not applicable without the hook or the
  declaration.
- A new narrowing: references are checked on the first served model, one
  image per item, without the pixel limit.

## 0.2.0

- Moves to subc-protocol 0.33; take subc-protocol 0.33, subc-transport 0.14, subc-control 0.33 and subc-client-rs 0.31 together.
- Breaking dependency update: `Reply::Error` contains subc-protocol 0.33's
  `ErrorBody`. The `wire` re-export moves to cortexkit-role-extractor 0.1.2.
- No behaviour changes; existing test-vector bytes are unchanged.

## 0.1.1

- `README.md` explains that the adapter behind `ExtractorRoute` opens its
  route with `role_versions` naming `extractor/v1`; the conformance kit does
  not open routes itself.

## 0.1.0

Reply side:

- `ExtractorSubject::script` and `Scripted`: the stand-in answers a state
  with a scripted HTTP status and body, byte for byte, and an unscripted
  state with 500 and no body, as before.
- Four reply checks, each replaying Clairic's captured replies from the
  role crate's `provider-replies.json` (its `test-vectors` feature):
  `provider_answers_forwarded` (every answer type, as `read_reply` reads
  it), `spans_forwarded_unconverted` (spans in Unicode scalar values,
  `state_text` byte for byte), `provider_errors_mapped` (the error envelope
  with `result: {}`, codes 5006 and 3030 as `invalid_item`) and
  `cost_usd_from_reported_tokens` (`cost_usd` from reported
  `input_tokens`). Each has a defect in the test fake that fails it by
  name.

Request side:

- First version of the `extractor/v1` conformance suite, request side: 21
  checks of `role.describe`, admission refusals (each checked for its code,
  for the field and limit where the case expects them, and for reaching no
  provider and reserving no batch id), batch identity,
  the zero-ceiling spend check, and the Clairic request mapping, against a
  stand-in provider that records what it receives.
