# cortexkit-role-routing-conformance

The `account_for_tier_*` checks exercise direct account admission at an
available tier, ordered fallback with the chosen tier recorded, and a caller
that omits tiers for a non-declaring router. The cases use the existing
`configure_service_tiers` plan fixture hook; for an unsupported fixture the
report marks each affected case `NotApplicable` with its name and reason,
never passed. This release pins routing 0.7.4 exactly, whose account answer
and pin use constructors instead of exhaustive Rust struct literals.

The conformance kit for the `routing/v1` role
(`crates/cortexkit-role-routing/CONTRACT.md`). A routing provider adds it as
a dev-dependency and runs it in its own CI against its real module over
real routes. It is never run against a double; the in-process fake under
`tests/fake/` exists only to test the kit itself.

```toml
[dev-dependencies]
cortexkit-role-routing-conformance = "0.10.4"
```

```rust,ignore
let report = cortexkit_role_routing_conformance::run_suite(&my_subject).await;
assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
```

## What a provider supplies

The decision-input checks `questions_requirement_honoured` and
`max_cost_usd_requirement_honoured` use optional
`RoutingSubject::configure_model_input_facts` to supply `ModelInputFacts`:
route key, served question limit, and stated input price per million tokens.
The cap check admits equality, rejects an over-cap estimate, and excludes
models with no stated input price. Each case reports not applicable with a
reason if the router does not advertise its requirement members or the
subject cannot supply these facts. The default hook returns `false`,
preserving existing adapters.

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
  - `configure_catalog(catalog)`: exactly these models (with their family, ability,
    limits, tool calls, modalities, structured output and pricing), runners
    (the route keys each runs, the tool providers it reaches with the
     tool capability tags each serves, and the runner's declared `harnesses`,
     `default_harness` and `starts_sessions`), and subscription accounts. Every
    runner listed is an attested runner module (contract section 9: its
    route carries a principal the daemon verified, and the provider knows it
    as a runner);
  - `configure_catalog_declarations(modules)`: raw `role_declarations` slots
    from daemon catalog entries, with each module's attested `principal` and
    `nonce_verified` status. Feed these through the catalog, never a call.
    Runner slots also supply raw `harness_limits`; invalid restrictions must
    leave the runner selectable with runner-level values.
    Provider-only fixtures preserve `configure_catalog`'s runnable route keys:
    supply a compatible transport/credential mapping from the runner-level
    pairs and vary only the provider allow-list. They do not depend on a
    particular pair mapping; pair-specific fixtures use the facts hook below.
  - Optional `configure_runner_route_facts(facts)`: route keys with their wire
    API and credential method, constructed with `RunnerRouteFacts::new`.
    Return `Ok(false)` when unsupported; pair-narrowing checks are then
    reported as not applicable, never passed. Provider-only narrowing and
    invalid-member fallback checks still run for every adapter. Head-choice
    assertions run when the provider declares `head_choices`.
    Narrowing first requires a runner-level offer of the model the harness
    excludes. Without that baseline offer the case is inconclusive and
    reported as not applicable, not passed. With it, harness offers must
    exclude that model and be a subset of the effective allowed set; the kit
    does not require the router to offer every otherwise runnable model.
  - Optional `configure_route_wire_api(route_key, endpoint)`: set the catalog
    row's served `wire.api` fact to `Some(endpoint)`, or remove it with `None`,
    independently of `wire.family` and the fact's provenance. The default
    returns `Ok(false)`. `runner_requires_wire_api` is not applicable and names
    the missing hook if either this hook or `configure_runner_route_facts` is
    unsupported. Otherwise it checks select and, when served, head choices
    exclude the row without an endpoint, then serves that endpoint and checks
    the row becomes selectable. Head choices may remain an admission subset.
    Declare `requires_wire_api` only after the router runs routing 0.7.13 or
    later; older strict parsers drop the runner;
  - `pin_for_project(project, consumer, pin)`: an operator pin that applies
    to the consumer's selects within the project;
  - `cool_down(route_key, until_ms)`: the route serves nothing before then.

A provider that declares `select_hold_control` in `role.describe` also
supplies these hooks for the hold checks. `block_account` is additionally
required by `account_continue` and `account_status`, even without `select_hold_control`.
The default hooks return an unsupported error, which fails the checks that
need them; providers declaring none of those groups are never asked for these hooks.

  - `configure_consumer_mode(consumer, mode)`: route the consumer by
    `ConsumerMode::Demand` or by `ConsumerMode::Ladder { entries }`, route
    keys in ladder order;
  - `configure_hold_limits(provider, limits)`: the operator's `max_hold`
    and `hold_when_reset_within`, in seconds, for that model provider;
  - `block_account(account_id, reset_at_ms)`: block the account by a window
    ending then, or at an unknown time for `None`; a later call replaces
    the block.

The hold checks configure `fixture::hold_catalog()` instead of the usual
catalog.

Before every check the kit resets the provider and configures the fixture
in `src/fixture.rs`. The provider's choice must be stable: the same request
against the same state answers the same route and runner.

Declarations cannot set `iq`: every request supplies it, and only operator
configuration may override it. Declaration checks use `max_output_tokens`
for defaults; the operator-precedence check also overrides the request's `iq`.

No check's expected answer depends on how a provider reads `iq`. The contract
leaves the mapping of `iq` onto model facts to the provider, so a provider may
read it on an absolute scale or relative to its strongest routable model, for
which `iq` 100 is always met. Every fixture model has the same `iq`, and every
`iq` a check's routing decision reads, after operator overrides, is met by all
of them under either reading; a check that needs a request nothing meets
raises a `context_window` or `max_output_tokens` floor instead.

Routers declaring `select_panel` implement
`configure_panel_class(class, size, top, extra)`. Configure the class's quality
floor so every panel-fixture model meets it, independently of the router's
quality scale. Routers declaring `select_pin_slot` implement
`configure_pin_slot(slot, route_key)`. Both hooks default to errors naming the
missing hook. `CatalogModel.family: Option<String>` is a breaking Rust minor
addition to an exhaustive fixture type; adapters must supply it.

## Checks

### Media engine selection (routing 0.7.7)

All `select_engine_*` checks are not applicable when the router does not
declare `select_engine`. Engine slots are installed through
`configure_catalog_declarations` under `media-provider/v1`; the adapter
must replace that fixture's slots without discarding decisions or admissions.
The kit opens each media module's routes as that module, under the principal
the daemon attests for a supervised module (`reserved:<module id>`), because
routing learns engines only from a module whose declaration the daemon verified.

Two additional hooks are required by the cases that need them:

- `configure_engine_facts(facts)` replaces the cost measurements
  for each `(kind, provider, model)`, including caller-local engines. Each
  `EngineFacts::new(kind, provider, model)` starts with both measurements absent;
  `with_cost(n)` supplies integer fixture units. Smaller cost is better, and
  absence sorts below a known value. These are relative ranking costs, not
  monetary amounts. The legacy `accuracy` member is retained for compatibility
  but ignored; accuracy is supplied only through `route.engine_eval`.
- `advance_clock(ms)` advances the provider's clock, firing every timer due in
  the interval before returning, while retaining decisions and admission replay
  state. The expiry cases advance ten minutes without sleeping.

The defaults return errors naming the missing hook. A router declaring the group
fails the affected check until its adapter implements the hook; it never silently
passes or skips an unsupported fixture.

`select_engine_slot_*` checks atomic rejection of malformed, duplicate and
empty-format slots and ignores nonce-unverified slots. `select_engine_request`,
`select_engine_requirements_valid` and `select_engine_caller_local` check
ownership, unknown/malformed members, refusals whose `field` names the failing
list entry by index (for example `caller_local[0].local`), and that a
`priced_only` consumer (one answered only with routes priced per token, §4)
still gets local and caller-local engines, because no consumer policy
restricts engine selection (§13). `select_engine_filters`, `select_engine_privacy`,
`select_engine_reachability` and `select_engine_match_requirements` check every
filter and all eleven requirements, including absent optional facts and exact
limits. `select_engine_answer` checks module/caller-local members and ten-minute
expiry; `select_engine_ranking` uses explicit cost fixtures and stable
module/provider/model ties with mixed caller-local and module engines.

The `select_engine_eval_*` cases require `engine_evals`; the
`select_engine_speed_authorization` case requires `engine_reports`. Each is
inapplicable without its own group, even when `select_engine` is declared.
They submit reference evaluations as the declaring module, check measured over
unmeasured ranking, newest scores on the same set, unrelated sets, new model
identities, the `transcribe` default `wer` metric, explicit metric overrides,
newest-set selection, score bounds, and attested-caller admission. Scores are
normalized to `0..=1`, including `1 - wer` for WER. Adapters must retain accepted
evaluations when catalog declarations are replaced. No accuracy fixture hook
or production reference text is needed.

Account admission checks match identifiers, not the router's open-string log
state. A router reporting `reserved` instead of `admitted` is conforming.

`select_engine_report` joins a report using only `decision_id` and `outcome` and
rejects another principal; `select_engine_explain` requires `unknown_decision`
when `explain` is also declared. `select_engine_media_admission` checks undecided
admission for a remote pair in an attested slot and refusal outside that slot.
The `select_engine_admission_*` cases check selected-module and pair binding,
caller-local refusal, pre-expiry admission, expiry for a new admission and the
unchanged replay of an admission accepted before expiry.

`rule-map.json` maps every pinned `select_engine` id, including the admission
and `no_engine` additions outside §13 of routing's `CONTRACT.md`, to one or more
registered cases. Every
rule is kit-checked; there are no `not_kit_checked` entries. The meta-tests run
each mapped case against the conforming fake and a deliberately incorrect fake,
and assert that the runner reports failure under that case's name. Separate
faults that ignore each requirement must also be rejected.

Health diagnostics for malformed slots, internal decision-to-consumer storage,
and route scope isolation are not observable through these media fixtures.
Remote reachability uses a cooldown rather than separately simulating backoff,
quota exhaustion and recorded failure. No test inspects real billing or engine
accuracy. The exported `NARROWINGS` repeats these coverage limits.

### Routing 0.7.0 and 0.7.1 additions

Each serving check below is not applicable without its group. Each
`undeclared_*` refusal check is not applicable **with** its group; whole-op
groups have no undeclared-op refusal case.

| Group | Serving checks | Undeclared-member checks |
|---|---|---|
| `select_panel` | `panel_full`, `panel_shortfall`, `panel_family_and_route_unique`, `panel_temperatures_by_slot`, `panel_same_seed`, `panel_replacement_preserves_slot`, `panel_foreign_replacement_unknown`, `panel_replacement_invalid_slot` | none |
| `budget_quote` | `budget_quote_matches_helper`, `budget_quote_foreign_unknown` | none |
| `explain` | `explain_matches_selection`, `explain_foreign_unknown` | none |
| `account_status` | `account_status_blocked_limited`, `account_status_unknown` | none |
| `head_choices` | `head_choices_valid`, `head_choices_runner_routes`, `head_choices_non_starting_empty`, `head_choices_billing_valid` | none |
| `consumer_models` | `consumer_models_valid`, `consumer_models_unknown_consumer` | none |
| `report_usage_run_id` | `report_usage_run_id_repeat_accepted`, `report_usage_run_id_overlong_refused` | none |
| `select_pin_slot` | `pin_slot_configured_pinned`, `pin_slot_unknown_unavailable` | `undeclared_pin_slot_refused` |
| `select_available_routes` | `available_routes_honoured`, `available_routes_empty_refused` | `undeclared_available_routes_refused` |
| `select_prefer_prompt_cache` | `prefer_prompt_cache_accepted` | `undeclared_prefer_prompt_cache_refused` |
| `select_settings` | `settings_unresolved_selects`, `settings_unknown_member_refused` | `undeclared_settings_refused` |
| `observe_backoff` | `backoff_with_admission_accepted`, `backoff_missing_retry_refused` | `undeclared_observe_decision_id_refused`, `undeclared_observe_retry_at_ms_refused`, `undeclared_observe_provider_code_refused` |
| `observe_credential_refused` | `observe_credential_refused_requires_backoff`, `credential_refused_blocks_credential`, `credential_refused_retry_at_ms_refused` | none |
| `report_outcomes` | none | none |

The panel fixture has repeated known families and two models with unknown
family; both family and route uniqueness are checked with `check_panel_answer`.
Replacements use `check_panel_replacement`. Same-seed selection resets and
reconfigures the subject between calls. It compares policy `size`, `top`,
`extra`, the shortfall code, and each seat's `slot`, `phase`, route,
`runner.module`, `family` and `temperature`. It ignores opaque `panel_id`,
`decision_id`, `expires_at_ms`, `explanation`, `detail` and informative policy
members. Budget checks compare the complete answer to `budget::quote` over
the selected decisions' answered capacities. Head choices accept either
provider-listing policy and never require completeness of the list.

Consumer-model answers pass `check_consumer_models`, including finite scores,
echoes and sorted unique pairs; decoding rejects an invalid billing value.
An unknown consumer must receive `unknown_consumer` naming it. Head-choice
decoding likewise rejects billing outside `subscription` and `per_token`.
Run-attributed reports exercise two run ids on one decision, then retry the
first pair with changed usage, requiring acceptance rather than a conflict.
The kit cannot observe internal usage counts through its current hooks, so it
does not assert deduplicated totals. A 129-character id must be refused naming
`usage.run_id`. Each new check has a faulty-router test and is not applicable
without its group.

The exported `NARROWINGS` coverage-limit notes state what the kit's hooks
can't observe: the availability and affinity effects of the four outcomes
`report_outcomes` adds, the fence a `transient_backoff` observation sets, and
the decision-owner authorization arm. Contract vectors and unit tests cover
those outcome effects, and the kit does check affinity after an ordinary
`failed` report and the account fence a report's future reset sets.
Prompt-cache tie-breaking is not inspected. Fixture models have no variants,
so head-choice variant rules are covered by `check_head_choices` unit tests;
panel modalities are not visible in an answer.

The new faulty-router matrix requires every named check to reject its faulty
implementation, while legacy checks remain unaffected. Several panel checks
share validation helpers, so a panel fault can also fail another panel check;
head-choice checks similarly share decoding and validation.

### Existing checks

The kit runs the checks listed below, including six declaration checks, eight optional-member
checks, four additional program checks, nine hold checks, eight checks
for pin fallback, reset fences, prefer and usage, and three for provider
exclusion and two for task references. The hold checks cover the optional
`select_hold_control` group: against a provider whose `role.describe` does
not declare it they report not applicable (`CheckOutcome::NotApplicable`,
printed `N/A`), and the verdict stays `Passed`. The two prefer routing checks
cover the optional `select_prefer` group the same way.

The requested-program check is likewise not applicable without `select_harness`;
the undeclared-group refusal check runs only without that group. The prefer
routing checks are likewise not applicable without `select_prefer`; the
undeclared-prefer refusal check runs only without that group. The two
`exclude_providers` checks are likewise not applicable without
`select_exclude_providers`, and the undeclared-group refusal check for it runs
only without that group. The two task-reference checks are not applicable
without `select_task_ref`: adding the reference must never change the selected
route, and malformed references must be refused by field name. Default-program
and non-starting-runner checks run regardless of the group. `harness_catalog()`
adds a runner with `["alpha", "beta"]` and default `"alpha"`, a singleton
`["solo"]` with no default, and an attach-only gateway with
`starts_sessions: false`. The default check also exercises multiple programs
without a default and an undeclared program list.

**Migration (breaking Rust minor):** `CatalogRunner` is exhaustive. Replace the
old route-key harness map with `harnesses: Option<Vec<String>>`,
`default_harness: Option<String>` and `starts_sessions: bool`. `None` means
undeclared; a declared list is non-empty. Feed these as the runner's actual
declaration, not labels assigned per route. **Readers first:** a 0.4.0 provider
refuses declarations with these members, so runners declare them only after
their routing provider serves 0.4.1. Callers send `harness` only when the
provider declares `select_harness`.

| Check | Contract rule |
|---|---|
| `parent_account_preference` | §9.1: prefer an eligible account other than the parent's |
| `parent_account_fallback` | §9.1: fall back to the parent when it is the only eligible account |
| `eligibility_never_holds_or_exhausts` | §8: model ineligibility never holds/exhausts the account for another model |
| `unknown_pool_accepted` | §2: unseen well-formed pools are accepted on all three ops |
| `malformed_pool_refused_by_field` | §2: malformed pools are `invalid_request {field: "pool"}` on all three ops |
| `pool_body_admission_conflict` | §9.1: pool changes under a used admission id are conflicts |
| `parent_body_admission_conflict` | §9.1: parent key changes under a used admission id are conflicts |
| `new_report_members_conflict` | §8: pool, eligibility, input-size and usage changes under a used decision id are conflicts |
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
| `harness_only_when_declared` | §5.2: `runner.harness` is the runner's declared harness, and absent when it declared none |
| `requested_harness_is_hosted_and_echoed` | §5.1–5.3: match a declared program and echo it; unhosted is `no_route` / `harness_unavailable` |
| `unrequested_harness_uses_declaration` | §5.2: declared default, else singleton, else absent, without filtering candidates |
| `non_starting_runner_never_selected` | §1.2: never select a `starts_sessions: false` runner, with or without harness |
| `undeclared_harness_group_refuses_member` | §5.1: without `select_harness`, refuse `harness` by field name |
| `hold_answers_blocked_entry_within_limits` | §5.4: a blocked entry whose reset is within both limits is answered, and `account_for` holds |
| `hold_moves_when_reset_beyond_window` | §5.4: a reset further away than `hold_when_reset_within` moves |
| `hold_moves_when_reset_unknown` | §5.4: an unknown reset moves |
| `hold_moves_beyond_caller_cap` | §5.4: a wait beyond the request's smaller `hold_wait_max_s` moves |
| `hold_ok_false_moves_to_usable_entry` | §5.4: `hold_ok: false` moves to an entry usable now |
| `hold_ok_false_falls_back_to_soonest_entry` | §5.4: with nothing usable now, the soonest reset within the limit is answered |
| `hold_nothing_within_limit_accounts_blocked` | §5.4: nothing within the limit is `no_route` `accounts_blocked` with the earliest known reset, or none |
| `hold_demand_mode_skips_blocked_model` | §5.4: a demand-mode consumer skips a blocked model |
| `hold_account_for_applies_recorded_cap` | §9.1: `account_for` naming a decision applies the cap its select recorded |
| `pin_fallback_routes_and_labels` | §5: an unservable pin with `pin_fallback: route` routes with `pin_outcome fell_back` and `detail pin_reason`; a servable pin is `pinned` |
| `report_reset_fences_only_decision_route_and_account` | §8: a failed report with a future reset fences only that decision's route on that account |
| `report_past_reset_fences_nothing_and_is_recorded` | §8: a past reset fences nothing and is recorded, never refused |
| `report_reset_capped_at_24h` | §8: a reset beyond 24 hours is capped at 24 hours |
| `prefer_prefers_listed_adequate_route` | §5.1: `select_prefer` answers a listed adequate route in list order |
| `prefer_never_admits_inadequate_route` | §5.1: `select_prefer` never admits a route the request would not get |
| `undeclared_prefer_group_refuses_member` | §5.1: without `select_prefer`, refuse `prefer` by field name |
| `report_usage_strict` | §8: bad usage counts or unknown members are `invalid_request {field: usage}` |
| `exclude_providers_answers_another_provider` | §5.1: `select_exclude_providers` never answers a route from a listed provider |
| `exclude_providers_every_provider_no_route` | §5.1: excluding every provider is the `no_route` an `exclude` of every route gets |
| `undeclared_exclude_providers_group_refuses_member` | §5.1: without `select_exclude_providers`, refuse `exclude_providers` by field name |
| `task_ref_never_changes_selection` | §5.1: `select_task_ref` answers the same route with or without `task_ref` |
| `invalid_task_ref_refused_by_name` | §5.1: malformed `task_ref` is `invalid_request` naming the field |

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
red, for a reason the test names. A break that rightly fails a second check
pins both: never holding (the recorded-cap check needs a held answer to
start from), reading `hold_ok: false` as `true` (the fallback check too),
and dropping `reset_at_ms` (a timed cause on two refusals). `FakeSubject::without_hold_control` shows the hold
checks not applicable and the verdict a pass. `FakeSubject::without_harness_control`
exercises undeclared-program-group refusals. Program defects cover membership,
echoing, the no-route reason, default and singleton fallback, invented/missing
programs and gateway selection. Missing or invented programs rightly fail
multiple checks, and the tests pin those names explicitly. When the fake ignores
the request's `runners` list, both `runners_honoured` and
`non_starting_runner_never_selected` fail: the gateway-only runner-list case
exercises the latter. `answering_a_runner_outside_runners_fails` asserts both
failures.

## Service-tier checks

A router declaring `service_tier` is checked by
`service_tier_eligible_account`, `service_tier_fallback_order`,
`service_tier_absent_is_standard` and `service_tier_report_members_accepted`.
The plan-fixture hook `configure_service_tiers` supplies test-only account plan
inclusions; it does not prescribe a production plan source. The eligibility
case follows a decision through account admission, and fallback records its
chosen level. `service_tier_absent_is_standard` expects standard for a select
without `service_tier`, so run it with no service-tier settings configured for
the kit's consumer: a router's tier settings may choose another level then.
Report cases send applied-only, served-only and both members.
Non-declaring routers are not applicable by name, and deliberately faulty
routers prove each check rejects its named violation.

## Account-continuation checks

Routers declaring `account_continue` implement
`configure_continue_credentials(&[ContinueCredential])`: install the named
credentials with the fixture's provider, auth method, account and exact model
eligibility, preserving existing decisions/admissions and enabling selection and
failover. `block_account` is also required, even if `select_hold_control` is not
declared. The checks admit a run before adding spares, without pinning an account,
then send the exhausted account's evidence as the runner.

| Check | Rule (CONTRACT §9.1.1) |
|---|---|
| `continue_offer_excludes_exhausted_account` | Never offer the exhausted account |
| `continue_offer_preserves_provider` | The credential's provider is unchanged |
| `continue_offer_preserves_auth_method` | The credential's auth method is unchanged |
| `continue_offer_preserves_model` | The credential can serve the original model |
| `continue_offer_excludes_known_hold` | Never offer an account under a known hold for the relevant window or pool; this check blocks a whole account |
| `continue_same_id_returns_identical_offer` | Compare the complete offer on a same-id retry |
| `continue_changed_question_refused_by_selection_id` | Refuse a changed parsed question under a reused id as `invalid_request {field: "selection_id"}`; preserve the first answer |
| `continue_hold_unknown_until_stays_unknown` | With no known reset, return hold without `until_ms`, never now |
| `continue_undeclared_group_refuses_mode` | Without the group, refuse `invalid_request` naming `mode` |

The first eight checks are not applicable without the group; the last runs only
without it. Tests pair each check with a deliberately faulty router and assert
its failing name. The kit's exported `NARROWINGS` list, printed in every suite
report, records the limits: credential facts and whole-account
holds are observable, but internal reservation counts, expiry release, ancestry,
pricing, exhaustion persistence, owner isolation and runner durability are not.
The kit does not inspect frozen session plans, tool sets or renderers, nor exercise terminal
refusal policy, transport retries or pool/window-specific holds.
