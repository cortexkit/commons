# Changelog

## 0.2.0

- Add optional `parent_conversation_key` and quota `pool` to account
  allocation, `pool` to flat readings and used-account observations, and
  `pool`, `eligibility` and `request_input_tokens` to outcome reports.
- Pin parent preference/fallback, opaque pool grammar and unseen-pool
  acceptance, disabled-window omission, eligibility without exhaustion or
  holds, size-class fencing, and provider-first rollout. Eligibility with
  an exhaustion code or reset time is `invalid_request {field: "eligibility"}`.
  Eligibility scope follows the existing open-string enum convention.
- Breaking Rust minor: the three extended request structs were exhaustive in
  0.1.1. `AccountForRequest`, `AccountObserveRequest` and `ReportRequest` are
  now `#[non_exhaustive]` so future optional fields can be additive. Build
  requests with constructors and `with_*` setters; every optional member has
  a setter. Absent members preserve the original bytes;
  original vectors are unchanged, with new vectors and checksum pins alongside.

## 0.1.1

- Add catalog `ConsumerDeclarations`, optional defaults using selection
  demand types except `iq`, a typed all-or-nothing validator, `ROLE_DECLARATIONS_KEY`,
  and pinned declaration vectors including a byte-exact round trip.
- Define verified catalog ownership, conflict health reporting, admission
  by either attested caller or daemon-stamped scope owner, operator/request/
  declaration precedence, and rollout for configuration-only names.
- Add terminal `consumer_not_owned` and its typed detail. This is a patch:
  the refusal enum was already non-exhaustive and preserves unknown codes;
  existing request types and required `iq` validation are unchanged.
- Declarations cannot set `iq`; it is refused as an unknown member. The
  request supplies its required `iq`, with operator configuration the only
  override. Include an invalid-`iq` declaration vector.

## 0.1.0

- First release of the `routing/v1` wire contract: `role.describe`,
  `route.select`, `route.report`, `route.account_for` and
  `route.account_observe`, with request validators, refusal codes and their
  details, the `{"result": <answer>}` reply envelope, `CONTRACT.md`, and the
  `routing-v1` vectors behind the `test-vectors` feature.
