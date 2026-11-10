# Changelog

## 0.10.13

- The harness-limits cases declare runner pairs with the shared spellings
  (`anthropic.messages`, `openai.responses`; `apikey`, `oauth`) instead of
  example tokens. A router that drops pairs outside `spellings::WIRE_APIS` and
  `AUTH_METHODS` was left with no usable pair, so the narrowing and fallback
  cases couldn't reach their assertions. The runner now declares all four
  combinations, so the provider case matches the subject's own catalog routes.

## 0.10.12

- `account_observe_credit_without_reset_accepted` admits
  `example-subscription/model-c`, a pair the kit's fixture catalog defines. It
  named `model-a`, which the catalog lists only under `example-provider`, so a
  router that checks the catalog refused the admission with `auth_required`
  before the credit observation was sent.

## 0.10.11

- Pin routing `=0.7.11`. Check harness-specific pair and provider narrowing
  in head choices and selection, inheritance, and whole-member fallback for
  invalid restrictions without dropping the runner.
- Add optional `configure_runner_route_facts` with constructed route facts;
  unsupported adapters skip pair checks, not provider-only checks. Exercise
  both through the kit's own in-repository test router (`tests/fake`) and reject incorrect implementations by
  check name.
- Respect head-choice admission subsets: require the excluded model in the
  runner-level baseline to prove narrowing; otherwise report the case as
  inconclusive, not passed.
- Runner declarations carrying `harness_limits` require routing 0.7.11 or
  later first; older strict parsers reject the entire runner declaration.

## 0.10.10

- Pin routing 0.7.10. Check that a credit exhaustion observation needs no
  reset evidence or retry hint, while continuation still refuses `credit`
  by `exhausted.limit_window`.
- Reject deliberately faulty routers for both checks by name.

## 0.10.9

- Pin routing 0.7.9. `select_engine_slot_malformed` now expects a slot whose
  members other than `engines` (such as describe's group facts) keep their
  engines, and still expects a slot without `engines` to be refused whole.

## 0.10.8

- Pin routing 0.7.8 and check that replacement detail accompanies only a cause,
  with a deliberately invalid seated reply rejected by
  `panel_replacement_detail_requires_cause`.
- Read the shared typed shortfall, legacy prose package cause, and replacement
  detail vectors, including rejection of a seat carrying detail.

## 0.10.7

- Pin routing 0.7.7 and check engine measurement authorization, score bounds,
  measured ranking, comparable reference sets, metric defaults and overrides,
  newest-set selection, and fresh model identities behind their describe groups.
- Recognize successful engine account admissions by their bound identifiers,
  not an unpinned state word; cover a router that logs `reserved`.

## 0.10.6

- Pin routing 0.7.6, including its shared answered-refusal retry classifier.
  No routing-provider case changes.

## 0.10.5

- Pin routing 0.7.5. No case changes.

## 0.10.4

- Pin routing 0.7.4 and add defaulted model-input facts for question limits
  and stated input prices.
- Check `questions` and `max_cost_usd` eligibility at their boundaries,
  including unstated prices under a cap, with faulty-router self-tests for
  ignored constraints. Cases are not applicable when the requirement or
  fixture hook is unavailable.

## 0.10.3

- Pin routing 0.7.3 and add group-gated `select_engine` cases for atomic
  catalog slots, every requirement, ordered filters, privacy, reachability,
  answer location/expiry, fixture-driven ranking, reports and explanations.
- Check media-provider slot authorization, decision/module/pair binding,
  expiry for new admissions and unchanged replay after expiry.
- Add default-failing `configure_engine_facts` and `advance_clock` hooks and
  the non-exhaustive `EngineFacts` fixture with constructors and builders.
- Ship `rule-map.json` for all pinned `select_engine` ids, with map coverage
  and named rejection tests for every mapped case and requirement member.

## 0.10.2

- Pin routing 0.7.2. Check `observe_credential_refused`'s describe prerequisite,
  credential-wide blocking across runners and models after an observation,
  and refusal of `retry_at_ms` on the new kind. Each check is gated on the
  group and paired with a deliberately faulty router rejected by name.

## 0.10.1

- Pin routing 0.7.1 and check run-attributed report acceptance and validation,
  consumer-model answers and unknown consumers, and head-choice billing.
- Pair the new group-gated checks with deliberately faulty routers and document
  the limits of observing usage deduplication through the public protocol.

## 0.10.0

- Pin routing 0.7.0 and check optional panels, budget quotes, explanations,
  account status, head choices, select additions and backoff observations.
- Add `CatalogModel.family` (a breaking Rust minor change),
  `configure_panel_class` and `configure_pin_slot` fixture hooks.
- Pair every new check with a deliberately faulty router and document the
  observable coverage limits.

## 0.9.0

- Pin routing 0.6.0 (including its Rust answer-construction API break) and check tier-aware `route.account_for` admission,
  ordered fallback and legacy callers without the `service_tier` group.
- Add deliberately faulty routers rejected by each new check's name.
- Check the decision's non-standard chosen tier on account answers with and
  without a repeated request tier, with faulty routers for both paths.

## 0.8.1

- Pin routing 0.5.7 and check opt-in account continuation: compatible offers,
  stable same-id retries, unknown hold times and refusal without discovery.
- Add continuation fixtures and deliberately faulty routers for each check.
- Reject changed questions under reused selection ids while preserving the
  first answer, with a named faulty-router check. Clarify group applicability,
  hold-check scope and continuation fixture hooks.

## 0.8.0

- Requires subc-protocol 0.34; breaking because `Reply::Error` contains its
  `ErrorBody`. Pins routing 0.5.6. No conformance-check changes.

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
