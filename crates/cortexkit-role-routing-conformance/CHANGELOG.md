# Changelog

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
