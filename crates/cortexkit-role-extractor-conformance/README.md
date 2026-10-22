# cortexkit-role-extractor-conformance

The conformance suite for the `extractor/v1` role
(`crates/cortexkit-role-extractor/CONTRACT.md`): its request side, and its
reply side against Clairic's own replies. An extractor module adds it
as a dev-dependency and runs it in its own CI against its real module over
its real management route. It is never run against a double; the in-process
fake under `tests/fake/` exists only to test the suite itself.

## What a module supplies

- An `ExtractorRoute` adapter: send one `{method, params}` request over the
  real route, with `params` inserted as the exact JSON text the suite gives,
  and return the reply's exact text (or the `ERROR` body).
- An `ExtractorSubject`:
  - the route;
  - the models its catalog serves (`ServedModel`: the provider's model
    value, priced or not, image limits, pass budget, context, egress host),
    which `role.describe` must match;
  - a catalog id whose row is not an extraction model, and one whose row
    states no kind, if it has them;
  - a view of a **stand-in provider** every served model routes to. The
    stand-in records each request body it receives, exactly, under the
    `state` the body carries, and answers with what the suite scripted for
    that state (`ExtractorSubject::script`, a `Scripted` HTTP status and
    body, sent byte for byte); a state never scripted answers HTTP 500 with
    no body. The suite scripts Clairic's replies from the role crate's
    `provider-replies.json`, captured in a live probe of
    2026-10-22.

## Verdict

A check the module gives the suite no way to ask is **not applicable**,
never passed. A run with no failure and some not-applicable checks is
"passed for what the module does", naming them; only a run where every check
ran and passed is "passed".

## Checks

| Check | Not applicable when |
|---|---|
| `describe_complete` | — |
| `bad_batch_id_refused` | — |
| `unknown_model_refused` | — |
| `non_extraction_model_refused` | the catalog has no row of another kind |
| `kindless_model_refused` | the catalog has no row without a kind |
| `field_id_refused_naming_field` | — |
| `unknown_field_type_refused_before_options` | — |
| `misspelt_or_misplaced_option_refused` | the model serves only number and duration |
| `missing_required_option_refused` | the model serves no list, estimate or forecast |
| `option_value_out_of_range_refused` | — |
| `instructions_refused_empty_or_over_bound` | the bound plus a request would not fit `max_request_bytes` |
| `item_state_and_reference_date_refused` | — |
| `unknown_member_refused` | — |
| `counts_refused_with_limit` | — |
| `images_refused_before_any_call` | — |
| `oversize_request_refused` | `max_request_bytes` is over 64 MiB |
| `reply_over_frame_refused` | no batch within the limits reaches the reply frame |
| `clairic_request_mapped` | — |
| `reuse_refused_naming_part` | — |
| `image_spellings_are_one_batch` | the model takes no images |
| `zero_ceiling_sends_nothing` | no served model is priced |
| `provider_answers_forwarded` | the model does not serve all nine types, takes fewer than nine fields, or bounds an answer below the probe's |
| `spans_forwarded_unconverted` | the model serves no text fields |
| `provider_errors_mapped` | — |
| `cost_usd_from_reported_tokens` | no priced served model takes list fields |

`CHECKS` in `src/report.rs` states what each check asserts, and `NARROWINGS`
where it checks less than the contract says; every report prints both.

## The suite's own tests

`tests/suite.rs` runs the suite against the fake: a faithful fake passes
every check, and each deliberate break in `fake::Defects` turns exactly its
check red, by name.
