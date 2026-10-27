# Changelog

## 0.5.1

- Add the optional `select_exclude_providers` group: `exclude_providers`
  lists provider ids, each the provider part of a route key, and no answer
  names a route from a listed provider. `check_answer` refuses one that does
  (`AnswerProblem::ExcludedProvider`). When every adequate route is from a
  listed provider the select is the same `no_route` an `exclude` listing
  those routes gets, and a pin on a listed provider is `pin_unavailable`
  reason `excluded`. Entries use the route-key provider validator (new
  `select::is_provider_id`, which `is_route_key` now uses) and the list rules
  of `exclude`: no length limit, repeats not refused. A bad entry, and the
  member sent to a provider without the group, are `invalid_request` naming
  `exclude_providers`. `exclude` keeps its meaning. Build with
  `SelectRequest::with_exclude_providers`; read with `excludes_provider`.
  New vector file `route-select-exclude-providers.json`, pinned in
  `SHA256SUMS`. Additive: `SelectRequest` and `AnswerProblem` were already
  `#[non_exhaustive]`.

## 0.5.0

- Add `pin_fallback: "route"` beside a pin: when the pin cannot be served
  the provider routes normally instead of refusing `pin_unavailable`. The
  answer carries `pin_outcome: "pinned" | "fell_back"` whenever the
  request pinned, and on `fell_back` its `detail` carries `pin_reason` naming
  the pin refusal's reason. Any other `pin_fallback` value, or a fallback
  without a pin, is `invalid_request` naming the field. Without the field
  pins still refuse and a second select for the same work stays forbidden.
- Add a fenced reset on `route.report`: a report with `outcome: failed` and
  a future `reset_at_ms` fences only that decision's route on that decision's
  account until the reset. A reset further than 24 hours away is capped at
  24 hours; a reset already in the past is recorded as a failure with no
  fence. Only the decision's own consumer may report, as before.
- Say that `account_observe {kind: run_finished}` releases the runner's
  account pin, with `lease_valid_until_ms` only the backstop.
- Add the optional `select_prefer` group: `prefer` lists up to 16 route keys
  with no duplicates, each `provider/model`. Among the routes the provider
  would already answer it answers a listed adequate one in list order, never
  admitting a route the request would not otherwise get. A provider without
  the group refuses `prefer` by name. `role.describe` lists it in
  `capabilities` like the other groups.
- Replace the report `usage` with token counts and timing that match the
  runner's usage names: `input_tokens`, `cached_input_tokens`,
  `cache_write_tokens`, `output_tokens`, `reasoning_tokens`, `duration_ms`
  and `first_token_ms`, plus the existing `cost_usd` claim. Input never
  includes cache reads or writes. Every count is a non-negative integer up
  to 2^53-1; `cost_usd` stays an optional non-negative finite number, the
  caller's own claim the provider may ignore. Unknown members are refused,
  and `usage` is part of the report body for idempotency.
- Breaking Rust minor: `SelectAnswer` and `Usage` were exhaustive and are now
  `#[non_exhaustive]`. Build answers with `SelectAnswer::new` and its
  `with_*` setters, and usage with `Usage::new` and its `with_*` setters
  (including `with_cost_usd`). New vectors and checksums cover the new members
  and the new prefer file.

## 0.4.3

- Contract text only, with no code change: §9.1 defines when the parent
  conversation is on an account, for the parent-account preference. It is
  on the account its conversation key's latest `admit`, `renew` or
  `reassign` answered, whether or not the call has started, until the pin
  lapses (`admission_valid_until_ms` if never started,
  `lease_valid_until_ms` once started).

## 0.4.2

- Contract text only, with no code change: operator configuration may name
  an owner for a consumer name found only in configuration. With an owner,
  the two ownership arms of §1.1 apply to it, and every other caller is
  refused `consumer_not_owned`. With no owner, the name stays usable by any
  caller, as before. A verified declaration still decides ownership.

## 0.4.1

- Add optional runner-wide `harnesses` (1–16 unique pair-grammar tokens),
  `default_harness` (must be listed), and `starts_sessions` (defaults true,
  omitted when true). Add builders and getters without making the declaration
  exhaustive. Existing slots keep their exact 0.4.0 bytes; field-path refusals
  and unknown-member rejection remain all-or-nothing.
- Add `select_harness`: a caller may request a program, not a wire/auth pair.
  Match only a runner listing it and echo it; no hosting runner is `no_route`
  reason `harness_unavailable`, naming the requested program in the message.
  Without a request use the declared default, else singleton, else omit the
  answer's harness without filtering candidates. Non-starting runners never
  answer select; all other operations retain their rules.
- Add declaration and select vectors with refreshed checksums. Rollout is
  readers first: 0.4.0 providers refuse these declaration members, so runners
  wait until their provider serves 0.4.1. Callers send `harness` only when the
  group is declared; other providers refuse it by name.

## 0.4.0

- Add optional catalog `runner` declarations: unique open-set wire API/auth
  pairs, the promise to call `route.account_for` and use its answer,
  validated tool capability names, and optional standing provider limits.
  Pair, capability and provider lists are bounded at 64 entries; pairs and
  explicit provider lists must be non-empty. Unknown members and malformed
  values refuse the whole slot with a field path. Tool capability validation
  stays local so role crates remain independent at runtime. Development-only
  tests check that accepted names and grammar stay aligned with the
  tool-provider role.
- Make `consumers` optional (empty when omitted), allowing pure runner slots.
  Keep the `ConsumerDeclarations` name and its `consumers` vector, but make
  it non-exhaustive: use `new`, `with_consumers` and `with_runner` instead
  of struct literals. This is a breaking Rust minor. Runner declaration
  types are also non-exhaustive. Legacy consumers-only slots, including an
  explicit empty list, keep their bytes; absent optional members are omitted,
  and `standing_limits: {}` retains its empty object.
- Pin readers-first rollout: runner modules must not declare `runner` until
  the routing provider adopts 0.4.0. A 0.3.x provider rejects the whole slot.

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
