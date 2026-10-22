# Changelog

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
