# Changelog

## 0.7.4

- Check tier-eligible account admission, fallback order and recorded chosen
  levels, absent-as-standard selection, and acceptance of tier report usage.
- Add plan-fixture configuration and faulty routers rejected by each new
  check's name. Pin the routing contract to 0.5.5.

## 0.7.3

- Depend on routing 0.5.3. Add two checks (58 total) for `select_task_ref`:
  `task_ref_never_changes_selection` compares the route answered with and
  without a task reference from the same fresh state, and
  `invalid_task_ref_refused_by_name` requires `invalid_request` naming
  `task_ref` for malformed references. Both are not applicable by name when
  the provider does not declare the group. Faulty providers that change
  selection or accept invalid references each fail only their named check.
  Document the attribution and retention behavior the kit cannot inspect.

## 0.7.2

- Depend on routing 0.5.1. Add three checks (56 total) for the optional
  `select_exclude_providers` group: excluding the provider of the route a
  select is otherwise answered gives a route from another provider, and
  excluding every provider is refused `no_route` with the reason and
  `reset_at_ms` a select excluding every route key gets. Both are not
  applicable without the group. A third check, run only without the group,
  requires `exclude_providers` to be refused `invalid_request` naming it.
  Faulty providers that ignore `exclude_providers`, that answer an excluded
  provider when nothing else is left, and that accept the member without the
  group each fail by name. A new entry in `NARROWINGS`, the kit's list of
  where it checks less than the contract says, names what the group's checks
  leave out.

## 0.7.1

- Fix `prefer_never_admits_inadequate_route`: its request nothing meets now
  raises `context_window` above every fixture model's instead of asking
  `iq` 100, which a provider reading `iq` relative to its strongest routable
  model correctly answers. No check's expected answer depends on an absolute
  `iq` scale any longer; a test runs the whole kit against such a provider,
  and a new faulty provider that admits a preferred route only when nothing
  meets fails the check by name.

## 0.7.0

- Depend on routing 0.5.0. Breaking Rust minor for the new request and answer
  shapes (`SelectAnswer` and `Usage` are now non-exhaustive; `CatalogRunner`
  gains no new fields but the fake gains prefer control).
- Add eight checks (53 total): pin fallback routing with `pin_outcome` and
  `detail pin_reason`, a reset fence narrowed to the decision's route and
  account with past-reset and 24-hour-cap handling, prefer routing in list
  order without admitting inadequate routes, refusing `prefer` without
  `select_prefer`, and strict report usage. Prefer routing checks are not
  applicable without the group. Each is paired with a faulty provider that
  fails it by name.
- Extend `new_report_members_conflict` to cover `usage` as part of the report
  body.

## 0.6.1

- Fix the fixture's second tool capability tag: `runner-two` now reaches
  `web.search/v1`, the tag the tool-provider contract defines, not
  `search.web/v1`, which no runner can declare. A provider that loads the
  fixture through the routing contract's own declaration validator refused
  it, so `tool_capabilities_matched_by_tag` could not pass against a correct
  provider. A new test runs every tag every fixture runner reaches through
  that validator.

## 0.6.0

- Depend on routing 0.4.1. Breaking Rust minor: exhaustive `CatalogRunner`
  replaces its route-key harness map with the runner's `harnesses`,
  `default_harness` and `starts_sessions` declarations. Adapters must map
  actual hosted programs, not labels per route.
- Add four checks (45 total) for requested program membership and echo,
  default/singleton/absent answers without filtering, non-starting runners,
  and refusing a request member without `select_harness`. Update
  `harness_only_when_declared` to read declarations. Each is exercised by
  faulty providers whose failing check names are pinned in tests.
- Add invented multi-program, singleton and gateway fixtures. Roll out
  readers first: runners declare the new members only after their routing
  provider serves 0.4.1, because 0.4.0 refuses the entire declaration.
  Callers send `harness` only when the group is declared.

## 0.5.0

- Re-export routing contract 0.4.0 through `wire`, including non-exhaustive
  `ConsumerDeclarations` and optional runner matching declarations. This is
  a breaking Rust minor for users constructing declaration struct literals;
  use the declaration constructor and builders instead. The conformance
  checks and adapter hooks are unchanged.

## 0.4.0

- Depend on routing contract 0.3.0, a breaking Rust minor (`SelectRequest`,
  `RoleDescribe` and `RunnerRef` are now non-exhaustive).
- Add nine hold checks for the optional `select_hold_control` group (41
  checks total). They cover answering a blocked ladder entry within both
  limits, with `account_for` then holding, and moving on a reset beyond
  `hold_when_reset_within`, an unknown reset, or a wait beyond the caller's
  smaller cap. They also cover `hold_ok: false` moving to an entry usable
  now, or falling back to the soonest reset; `no_route` `accounts_blocked`
  with the earliest known reset, or none; demand mode skipping a blocked
  model; and `account_for` applying the cap its decision recorded. Against
  a provider that does not declare the group they report the new
  `CheckOutcome::NotApplicable`, and the verdict stays `Passed`.
- Add `harness_only_when_declared`: `runner.harness` is the runner's
  declared harness, absent when it declared none.
- Breaking: `CheckOutcome` gains `NotApplicable` and is now
  `#[non_exhaustive]`; `CatalogRunner` gains `harnesses`, so adapters
  building it with a struct literal must add the field. Add defaulted
  subject hooks `configure_consumer_mode` (`ConsumerMode`),
  `configure_hold_limits` (`HoldLimits`) and `block_account`, the
  `fixture::hold_catalog()` the hold checks configure, and
  `SuiteReport::not_applicable`.
- Each new check is paired with a faulty provider that fails it by name.
  Three breaks rightly fail a second check, and their tests pin both: never
  holding also fails the recorded-cap check, reading `hold_ok: false` as
  `true` also fails the fallback check, and dropping `reset_at_ms` also
  fails `accounts_blocked`.
## 0.3.0

- Depend on routing contract 0.2.0, a breaking Rust minor because its extended
  request structs were exhaustive in 0.1.1. Build requests with constructors
  and setters; the three extended requests are now non-exhaustive.
- Add eight checks (31 total): parent account preference and fallback,
  eligibility without an account hold/exhaustion, unseen-pool acceptance,
  malformed-pool `detail.field`, and admission/report id reuse for new members.
  Each has a positive probe and a single-rule faulty provider.
- State which eligibility and provider/runner behaviors the current hooks
  cannot check:
  account-scope eligibility versus exhaustion, pool isolation, disabled-window
  omission and size-class fencing are not observable with the existing hooks.

## 0.2.0

- Add six declaration checks (23 checks total), each paired with a faulty provider: owning
  caller/default demand, delegated scope-owner admission, non-owner refusal,
  operator configuration precedence, ignored unverified catalog entries,
  and conflicting verified owners. The owner check also verifies
  request demand overrides a declaration.
- Add the defaulted `configure_catalog_declarations` hook with raw module
  slots and nonce verification metadata. Add the complete bind scope stamp
  `(owner, scope_ref, scope_epoch)` to `RouteStamp` and optional operator
  demand values to `ConsumerConfig`.
- Minor release because the new public struct fields require adapters using
  struct literals to migrate. Depends on `cortexkit-role-routing` 0.1.1.
- Declarations never supply `iq`; cases use `max_output_tokens` instead.
  Requests keep required `iq`. Separate `OperatorRequirements` from
  declaration defaults, and verify in the operator-precedence case that
  configuration may still override the request's `iq`.
- Exercise unconfigured and configured names alongside an unverified entry,
  ignored unverified defaults, and a verified owner's claim in either
  catalog order. Conflicting verified owners remain locked even when the
  name is configured, and `role.describe` omits their name. Pair both checks
  with single-rule faulty providers.

## 0.1.0

- First release of the `routing/v1` conformance kit: `RoutingSubject` and
  `RoutingRoute`, the fixture it configures, and seventeen checks, each
  tested against a faulty provider that breaks only its rule.
