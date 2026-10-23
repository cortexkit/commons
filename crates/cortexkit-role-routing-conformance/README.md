# cortexkit-role-routing-conformance

The conformance kit for the `routing/v1` role
(`crates/cortexkit-role-routing/CONTRACT.md`). A routing provider adds it as
a dev-dependency and runs it in its own CI against its real module over
real routes. It is never run against a double; the in-process fake under
`tests/fake/` exists only to test the kit itself.

```toml
[dev-dependencies]
cortexkit-role-routing-conformance = "0.1.0"
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
    `stamp.project_id` when present and unscoped otherwise. The kit opens
    routes as a consumer module, a second consumer module, and two runner
    modules (`fixture`);
  - `reset()`: forget every consumer, catalog entry, pin, cooldown,
    decision, affinity, report, admission and observation;
  - `configure_consumers(consumers)`: exactly these consumer ids, each with
    its `priced_only`;
  - `configure_catalog(catalog)`: exactly these models (with their ability,
    limits, tool calls, modalities, structured output and pricing), runners
    (the route keys each runs, and the tool providers it reaches with the
    tool capability tags each serves), and subscription accounts. Every
    runner listed is an attested runner module (contract section 9: its
    route carries a principal the daemon verified, and the provider knows it
    as a runner);
  - `pin_for_project(project, consumer, pin)`: an operator pin that applies
    to the consumer's selects within the project;
  - `cool_down(route_key, until_ms)`: the route serves nothing before then.

Before every check the kit resets the provider and configures the fixture
in `src/fixture.rs`. The provider's choice must be stable: the same request
against the same state answers the same route and runner.

## Checks

| Check | Contract rule |
|---|---|
| `describe_lists_consumers_and_fields` | §4: describe lists every configured consumer with `priced_only`, every required op and every requirement field |
| `unknown_requirement_refused_by_name` | §5.1: an unknown requirement is `invalid_requirements {field}`, never ignored |
| `unknown_consumer_refused` | §5.1: an unknown consumer is `unknown_consumer` |
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

## The kit's own tests

`tests/suite.rs` runs the kit against the fake: a faithful fake passes every
check, and each deliberate break in `fake::Defects` turns exactly its check
red, for a reason the test names.
