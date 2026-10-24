# Changelog

## 0.3.1

- Contract text only: in the hold rules (5.4), a per-token account counts among an entry's accounts only where the provider's account policy allows falling back to one. No code or wire change.

## 0.3.0

- Add the optional `select_hold_control` group. `role.describe` gains
  `capabilities`, the optional groups a provider serves (left out when
  empty). A provider declaring the group reads two new optional `route.select`
  members: `hold_ok` (default `true`) and `hold_wait_max_s`, the caller's own
  cap on a hold (`0` means no hold). Both are valid together: the fallback a
  `hold_ok: false` request can take may still hold, and the cap applies to
  it. A provider without the group refuses either member as
  `invalid_request {field}`, `hold_ok` first. A caller sends them only to a
  provider declaring the group. `parse_select_request_serving` takes the
  declared groups; `parse_select_request` declares none.
- Pin how a provider declaring the group decides a ladder entry whose
  eligible accounts are all blocked (contract section 5.4). Select never
  waits: it answers the entry, so the runner holds at `route.account_for`, or
  it moves to the next entry. The checks run in order: `hold_ok: false`, an
  unknown reset or one beyond the operator's `hold_when_reset_within`, then a
  wait beyond the hold limit (the smaller of `max_hold` and the request's
  cap). With `hold_ok: false` and no entry usable now, the provider answers
  the entry whose reset is soonest within the limit. When none is within the
  limit, it refuses `no_route` with the new reason `accounts_blocked` and the
  earliest known reset. A demand-routed consumer skips a blocked model. The
  decision records the caller's cap, and `route.account_for` naming it holds
  only within the smallest of `max_hold`, that cap and its own.
- Add optional `runner.harness` to the select answer: the harness the
  runner hosts the route on, filled only from the runner's own declaration.
  A caller never binds a runner session on an answer without it.
- Breaking Rust minor: `SelectRequest`, `RoleDescribe` and `RunnerRef` are
  now `#[non_exhaustive]`. Build them with `SelectRequest::new`,
  `RoleDescribe::new` and `RunnerRef::new` and their `with_*` setters;
  every optional select member now has a setter. Absent members are left
  out, so earlier requests, answers and describes serialize byte for byte as
  before. The original vectors are unchanged. `route-select-hold-control.json`
  is new, with its `SHA256SUMS` line.
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
