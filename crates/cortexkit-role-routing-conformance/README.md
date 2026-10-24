# cortexkit-role-routing-conformance

The conformance kit for the `routing/v1` role
(`crates/cortexkit-role-routing/CONTRACT.md`). A routing provider adds it as
a dev-dependency and runs it in its own CI against its real module over
real routes. It is never run against a double; the in-process fake under
`tests/fake/` exists only to test the kit itself.

```toml
[dev-dependencies]
cortexkit-role-routing-conformance = "0.3.0"
```

```rust,ignore
let report = cortexkit_role_routing_conformance::run_suite(&my_subject).await;
assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
```

## What a provider supplies

- A `RoutingRoute` adapter: send one `{method, params}` request over a real
  route to the provider and return the reply's exact body text, or the
  `ERROR` body. The provider reads the caller and the scope from the route,
  so the adapter must never shortcut into the provider's code.
- A `RoutingSubject`:
  - `route(stamp)`: open a route as `stamp.principal`, bound to
    `stamp.project_id` when present, and carrying `stamp.scope` when present.
    The scope stamp is `(owner, scope_ref, scope_epoch)`; declaration admission
    accepts either the declaring module's attested `stamp.principal` or its
    `stamp.scope.owner`. Neither identity comes from a body member. With neither
    stamp the route is unscoped. The kit opens
    routes as a consumer module, a second consumer module, and two runner
    modules (`fixture`);
  - `reset()`: forget every consumer, catalog entry, pin, cooldown,
    decision, affinity, report, admission and observation;
  - `configure_consumers(consumers)`: exactly these consumer ids, each with
    its `priced_only` and operator `requirements` (`OperatorRequirements`
    includes optional `iq` and the other demand fields in `defaults`);
  - `configure_catalog(catalog)`: exactly these models (with their ability,
    limits, tool calls, modalities, structured output and pricing), runners
    (the route keys each runs, and the tool providers it reaches with the
    tool capability tags each serves), and subscription accounts. Every
    runner listed is an attested runner module (contract section 9: its
    route carries a principal the daemon verified, and the provider knows it
    as a runner);
  - `configure_catalog_declarations(modules)`: raw `role_declarations` slots
    from daemon catalog entries, with each module's attested `principal` and
    `nonce_verified` status. Feed these through the catalog, never a call.
    The default hook returns an unsupported error for non-empty input, so
    older adapters must implement it to pass the new declaration checks;
  - `pin_for_project(project, consumer, pin)`: an operator pin that applies
    to the consumer's selects within the project;
  - `cool_down(route_key, until_ms)`: the route serves nothing before then.

Before every check the kit resets the provider and configures the fixture
in `src/fixture.rs`. The provider's choice must be stable: the same request
against the same state answers the same route and runner.

Declarations cannot set `iq`: every request supplies it, and only operator
configuration may override it. Declaration checks use `max_output_tokens`
for defaults; the operator-precedence check also overrides the request's `iq`.

## Checks

The kit runs 31 checks, including six declaration checks and eight optional-member checks.

| Check | Contract rule |
|---|---|
| `parent_account_preference` | §9.1: prefer an eligible account other than the parent's |
| `parent_account_fallback` | §9.1: fall back to the parent when it is the only eligible account |
| `eligibility_never_holds_or_exhausts` | §8: model ineligibility never holds/exhausts the account for another model |
| `unknown_pool_accepted` | §2: unseen well-formed pools are accepted on all three ops |
| `malformed_pool_refused_by_field` | §2: malformed pools are `invalid_request {field: "pool"}` on all three ops |
| `pool_body_admission_conflict` | §9.1: pool changes under a used admission id are conflicts |
| `parent_body_admission_conflict` | §9.1: parent key changes under a used admission id are conflicts |
| `new_report_members_conflict` | §8: pool, eligibility and input-size changes under a used decision id are conflicts |
| `describe_lists_consumers_and_fields` | §4: describe lists every configured consumer with `priced_only`, every required op and every requirement field |
| `unknown_requirement_refused_by_name` | §5.1: an unknown requirement is `invalid_requirements {field}`, never ignored |
| `unknown_consumer_refused` | §5.1: an unknown consumer is `unknown_consumer` |
| `declared_consumer_owner_uses_default` | §1.1: the declaring module is admitted, omitted demand uses the declaration, and request demand overrides it |
| `declared_consumer_scope_owner_admitted` | §1.1: a third-party caller carrying the declaring module's stamped scope is admitted |
| `declared_consumer_non_owner_refused` | §1.1: a third-party caller without that scope is refused terminal `consumer_not_owned` |
| `declared_consumer_unverified_entry_ignored` | §1.1: unverified entries reserve no name and apply no defaults; configuration and verified ownership remain authoritative |
| `declared_consumer_conflict_refused` | §1.1: conflicting verified owners are both refused `consumer_not_owned`, and describe omits the name |
| `operator_config_overrides_declared_default` | §1.1: operator demand overrides request demand and the declaration |
| `exclude_honoured` | §5.1: an excluded route key is never answered |
| `runners_honoured` | §5.2: the runner is one of `runners` |
| `timed_refusals_carry_reset_at_ms` | §5.3: a timed cause carries `reset_at_ms`; an untimed one does not |
| `scope_comes_from_route_stamp` | §3: a project pin applies under that project's route only, never from a body member |
| `report_from_other_principal_unknown_decision` | §6: another principal's report is `unknown_decision`, indistinguishable from a decision that never existed |
| `repeated_report_returns_first_answer` | §8: the same report again returns the first answer |
| `different_report_is_report_conflict` | §8: a different report is `report_conflict` |
| `affinity_holds_until_failure_report` | §7: affinity keeps a healthy route and ends on a failure report |
| `answer_carries_limits_and_future_expiry` | §5.2: limits from the catalog and a future `expires_at_ms` |
| `account_for_idempotent_on_admission_id` | §9.1: the same admission again returns the first answer |
| `account_for_different_body_admission_conflict` | §9.1: a different body under a used `admission_id` is `admission_conflict` |
| `account_for_foreign_decision_unknown` | §9.1: a `decision_id` from another runner, or for another model, is `unknown_decision` |
| `account_observe_idempotent_per_event_id` | §9.2: the same `event_id` again is `duplicate: true` |
| `tool_capabilities_matched_by_tag` | §5.1: tool capabilities are matched by tag, never by module name |

`CHECKS` in `src/report.rs` states what each check asserts, and `NARROWINGS`
where it checks less than the contract says; every report prints both.
The declaration checks exercise unverified entries and conflicting verified
owners, but do not check invalid catalog declarations or provider health
reports for invalid, unverified or conflicting entries.

The eligibility check reports model-scoped ineligibility for one model, then
checks that a different model on the same account remains eligible. The current
hooks cannot distinguish legitimate account-scope ineligibility from incorrect
exhaustion/hold, so that distinction is not claimed. Pool isolation,
disabled-window omission by runners and size-class fencing also need richer
provider/runner hooks. Wire vectors cover the pool and input-size fields, not
those provider or runner policies.
Parent checks use fresh admissions; they do not establish priority against an
existing child pin, `requested_account`, `renew` or `reassign`.

## The kit's own tests

`tests/suite.rs` runs the kit against the fake: a faithful fake passes every
check, and each deliberate break in `fake::Defects` turns exactly its check
red, for a reason the test names.
