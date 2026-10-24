# Changelog

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
