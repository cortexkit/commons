# cortexkit-role-routing

Version 0.7.26 adds readers-first `account_observe_success` for reporting a
successful retest without a select decision. Send `attempt_outcome` on an
`attempt` observation only after the router advertises the group. It also adds
`select_media`'s synthesize-only `input_characters` Unicode-scalar count for
per-character price estimates; routers list it in `requirement_fields` when
read and otherwise refuse it by name (CONTRACT.md §§9.2 and 16.4.1).

Version 0.7.22 clarifies `decision_harness`: decisions name a program only
when the selected runner declares programs, and omit the harness otherwise.
Use `check_answer_serving_with_harnesses` with that runner's declared list to
validate presence and membership. The old `check_answer_serving` signature is
retained for one release but no longer checks harnesses without declarations.

Version 0.7.21 pins `caller_runner` placement to a catalog runner attested as
the caller, or otherwise to the single runner in the select request (§16).

Version 0.7.20 adds `account_continue_signed_out` for continuing a run paused
for sign-in onto a compatible account (CONTRACT.md §9.1.1).

Version 0.7.18 adds reader-first groups `consumer_declarations`,
`runner_harness_facts`, `decision_harness`, `select_kind`,
`select_eligible_routes`, `select_price`, and `report_attempt_outcomes`.
See CONTRACT.md §16 for their independent discovery and wire rules. Modules
declare new members only after the live router advertises their group.
Use the declaration builders, `parse_consumer_declarations_serving`,
`parse_select_request_with_defaults` and `check_answer_serving_with_harnesses`
when adopting them; existing parsers keep their baseline request behavior.
For typed declarations use `validate()` without the group and
`validate_serving(capabilities)` for a declaring reader. The latter accepts
declared iq defaults and treats `iq: null` as absent; the former preserves
the strict legacy unknown-member refusal even after typed decoding.

Select prices estimate value, **not money spent**. Callers multiply usage by
the returned rates (already divided by the stated subscription multiplier),
and match lifetime cache writes only to their exact `cache_write_ttl` entry.
Reasoning tokens are already in output. A caller holding a dollar cap refuses
to send without a price; this is the caller's rule, not a routing refusal.
Runners still report usage only. `attempt_outcome` is authoritative when its
group is declared; rejected and unknown statuses never count against a route.

Since 0.6.0, `route.account_for` accepts optional `service_tier` only from
callers that discovered the `service_tier` group, using select's existing plan
eligibility and descending fallback. Its answer records top-level
`service_tier_chosen`; legacy wire bytes are unchanged when the optional
members are absent. `parse_account_for_request_serving` enables tier parsing
for declared groups; the legacy parser ignores that member. Never interpret
a non-declaring router's missing chosen tier as confirmation of a requested
tier. Construct both non-exhaustive types, `AccountForAnswer` and `AccountPin`,
with `new`, not Rust struct literals. Only `AccountForAnswer` has the optional
`with_service_tier_chosen` setter.

Wire types, validation, refusal codes and vectors for `routing/v1`: the role
a routing provider plays when it decides which model a piece of model work
runs on, and on which runner.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-routing = "0.7.21"
```

To include the shared vector bytes in your build, enable `test-vectors`:

```toml
[dev-dependencies]
cortexkit-role-routing = { version = "0.7.21", features = ["test-vectors"] }
```

The feature exposes `cortexkit_role_routing::vectors::FILES` and a constant
for each file. The vectors live in `test-vectors/routing-v1/` and ship
inside the crate package, pinned by `SHA256SUMS`. The crate's own vector
tests need the feature: `cargo test -p cortexkit-role-routing --features
test-vectors`.

What the crate holds:

- `declarations`: non-exhaustive `ConsumerDeclarations`, consumer demand defaults,
  optional `RunnerDeclaration` matching inputs and a typed
  validator for the catalog's `role_declarations[ROLE_DECLARATIONS_KEY]`;
- `engine_declarations`: atomic `parse_engine_declarations` for the separate
  `media-provider/v1` slot, using the media crate's `Engine` type;
- `select_engine`: media request/answer types, strict request parsing,
  `meets_requirements`, ordered filtering/ranking with `select_engine`, and
  answer location checks; attestation, consumer admission and retained decision
  state remain the provider's responsibility;
- `engine_measurements`: per-job speed reports, reference-set accuracy evals,
  strict parsing, attested engine-owner validation, realtime-factor derivation,
  and comparable evaluation scores with `apply_evaluations`;
- `describe`: the `role.describe` answer, its optional `capabilities`
  groups, and `check_describe`;
- `capabilities`: the optional groups this version defines
  (`select_hold_control`, `select_harness`, `select_prefer`,
  `select_exclude_providers`, `select_task_ref`, `service_tier`,
  `account_continue`, `select_panel`, `budget_quote`, `explain`,
  `account_status`, `head_choices`, `select_pin_slot`, `select_available_routes`,
  `select_prefer_prompt_cache`, `select_settings`, `observe_backoff`,
  `observe_credential_refused`,
   `report_outcomes`, `select_engine`, `engine_reports`, `engine_evals`);
- `select`: the `route.select` request, `parse_select_request` and
  `parse_select_request_serving` (which reads the members of the groups a
  provider declares), the answer and `check_answer`;
- `report`: the `route.report` request, `parse_report_request` and the
  answer;
- `account`: the `route.account_for` request, `parse_account_for_request`
  and the answer;
- `observe`: the `route.account_observe` request,
  `parse_account_observe_request` and the answer;
- `errors`: the refusal body, every refusal code, and each code's detail;
- `wire`: the `{"result": <answer>}` reply envelope (`reply_body`,
  `read_reply`).

## Optional operations and checks in 0.7.3

Since 0.7.7, `engine_reports` and `engine_evals` each require `select_engine`.
Call `route.engine_report` once after a module job ends; speed feeds a router's
rolling latency/realtime window, never accuracy ranking. Accuracy comes only
from `route.engine_eval` on a reference set; scores are in `0..=1`, with WER
reported as `1 - wer`. Both operations accept only the engine's attested
declaring principal. They do not release an account lease.

Apply evaluations after filtering candidates, before `select_engine` ranks
them. `requirements.metric` overrides the per-kind default (`transcribe` →
`wer`; no default for other kinds). Choose the set whose newest evaluation
has the latest measurement time, with bytewise ascending set-id ties, and
compare only that set's scores. A new model id and every caller-local engine
stay unmeasured. The engine-declaring media provider reporting evaluations keeps
one canonical set per kind and
re-runs every engine when the set changes. Cost remains a catalog fact.

Each new operation is listed in `ops` exactly when its same-named group
(without `route.`) is declared. `check_describe` names a mismatch as
`DescribeProblem::OpGroupMismatch { group }`; `REQUIRED_OPS` is unchanged, so
0.6.0-shaped routers remain valid. Call new operations only after discovering
their group.

| Module / operation | Pure checks and caller helpers |
|---|---|
| `select_engine` / `route.select_engine` | `parse_select_engine_request`, `meets_requirements`, `select_engine`, `check_answer`; media facts, filter order and catalog ranking; module-served answers carry `module` without `index`, while in-process caller-local answers carry `index` without `module` |
| `panel` / `route.select_panel` | `check_panel_answer`, `check_panel_replacement`, `SelectPanelRequest::seat_request`, `PanelState::new` and `apply`; slot policy, temperature, diversity, window/package floors and historical route/decision freshness |
| `budget` / `route.budget_quote` | `quote`, `check_budget_quote`; retained capacities, saturating ceilings and deterministic limiting ties |
| `explain` / `route.explain` | `check_explain`; decision, route, runner and serving-provider consistency |
| `account_status` / `route.account_status` | `check_account_status`; echoed account identity and reset applicability |
| `head_choices` / `route.head_choices` | `check_head_choices`; runner echo, sorted unique choices and valid default variants; optional `pin_only` and closed `billing::Billing` metadata |
| `consumer_models` / `route.consumer_models` | `check_consumer_models`; consumer echo, sorted unique pairs, finite `f64` scores and closed billing; select's consumer admission |
| `select` additions | `check_answer` now checks `available_routes`, slot-pin outcomes and `project_settings` presence; `SelectSettings::check_project_stamp` rejects mismatched trusted project stamps |
| `report` additions | `parse_report_request_serving`, `Outcome::effect`; four group-gated outcomes with no availability penalty and the prior route preference (affinity) kept; optional `usage.run_id` with `report_usage_run_id` |
| `observe` additions | `parse_account_observe_request_serving`, `check_observe_authorization` and `ObserveArm`; account-local backoff and admission-bound decision-owner wall observations |

New parsers take declared groups; legacy report/observe parsers supply none.
Without `report_outcomes`, the four new outcome strings remain unknown failures;
send `cancelled` instead to those routers. Without `observe_backoff`, new observe
members are refused by name and `transient_backoff` remains advisory. Select's
`pin_slot`, `available_routes`, `prefer_prompt_cache` and `settings` each have
their own group and undeclared-member refusal. Unknown open-set values are kept,
and absent optional fields preserve old bytes.

`observe_credential_refused` requires `observe_backoff`; `check_describe` names
the missing prerequisite with `DescribeProblem::MissingCapabilityDependency`.
With the group, `credential_refused` blocks a credential across runners rather
than recording a resettable limit. It names the credential or its admission,
allows verbatim `provider_code`, and refuses `retry_at_ms`. Without the group,
the kind remains advisory, including in the legacy parser.

`usage.run_id` is an opaque 1–128-character id without controls, refused by
`usage.run_id` when invalid or undeclared. Distinct run ids on one decision can
each contribute usage; repeated decision/run pairs are accepted and ignored
after the first. Without it, reports retain decision-level idempotency and
different bodies conflict. Consumer models carry only the router's own stored
scores and exclusions: prices, context windows, modalities and lifecycle still
come from the model catalog. Billing is a closed enum (`subscription` or
`per_token`); unknown wire values are malformed answers, while absence means
unknown. Pin-only models are chosen only when explicitly pinned.

`settings` selects only project and service-tier settings, never route scope;
an unresolved project still selects without settings. Panels use router-owned
class policies and can replace seats without returning to a previously seated
route or decision. Quote and explain accept only select/panel decisions, not
account-allocation or continuation-offer ids. A decision may still be quoted,
explained, or replaced through its panel after it expires, while it remains
retained by the router.

Pooled API keys use the existing account lease: the runner acquires/renews via
`account_for` and releases with `account_observe` `run_finished`. The runner's
`routed_accounts` declaration chooses which sign-in methods it handles itself;
listed methods require `follows_account_choice: true` and use the credential
named by account admission rather than a select hint. See CONTRACT §§5.2, 9.3
and 11 for ownership, rollout, request/answer tables and refusal rules.

The vector directory has one file per new group, including both op/group
mismatches, member refusals without declaration and unknown open-set strings.
Conformance checks and adapter hooks are documented by the
[routing conformance kit](../cortexkit-role-routing-conformance/README.md), not
implemented in this pure wire crate.

Declarations may default `eq`, `context_window`, `max_output_tokens`, `tools`,
`tool_capabilities`, `modalities` and `structured_output`, but cannot set `iq`.
A declaration carrying `iq` is invalid as an unknown member. Requests always
carry their required `iq`; an operator may override it in configuration.

`ConsumerDeclarations::new()` and its `with_consumers` / `with_runner` builders
construct catalog slots without exhaustive struct literals. Either member may
be omitted. A runner module declares the open-set `(wire_api, auth_method)`
pairs it can drive, whether it calls `route.account_for` and uses its answer,
and its supported tool capabilities. Optional `standing_limits.explicit_providers`
restricts it to credentials for those providers; absence (including
`standing_limits: {}`) means no provider limit. Routing intersects these
declarations with vault credentials and catalog routes; declarations grant no
credential access. See contract §1.2 for validation and matching rules.

`RunnerDeclaration::with_requires_wire_api(true)` sets the default requirement
to open rows only through their served `wire.api`.
`RunnerPair::with_requires_wire_api(bool)` overrides that default for one pair;
omission or null inherits it. Routing applies the effective requirement to
select, panels, head choices and the `runner_requires_wire_api` explanation.
`requires_wire_api()` reads only the runner default;
`requires_wire_api_for_pair(&pair)` resolves the matching runner pair's override,
then the default, then false. `admits_pair_row(&pair, bool)` checks endpoint
availability with that effective value, without reading the catalog. These
helpers resolve the runner pair by wire API and auth method, so a requirement
written in a harness-limit pair has no effect. Pair membership and other routing
constraints still need separate checks.

`admits_row(bool)` retains its 0.7.13 runner-default behavior for callers without
pair overrides, but is deprecated in documentation for pair matching: use
`admits_pair_row` instead. Upgrade the router to 0.7.13 or later before declaring
the runner-level member, and to 0.7.14 live before declaring it on any pair:
older strict parsers drop the whole runner declaration. Non-boolean, non-null
values refuse the declaration with the runner or pair field path.

Runners optionally declare `harnesses` (1–16 unique program tokens), a
`default_harness` listed there, and `starts_sessions` (true by default and
omitted when true). Use `RunnerDeclaration::with_harnesses`,
`with_default_harness` and `with_starts_sessions`, with corresponding getters.
An empty program list is refused, not read as "no program". Programs are
independent of wire/auth pairs: a runner that launches an agent program may
host several programs through one pair. A gateway runner sets
`starts_sessions: false` and never answers select, but account allocation for
its own turns, observations, reports and describe are unchanged.

`with_harness_limits` attaches a map of `HarnessLimits` constructed with
`with_pairs` and/or `with_explicit_providers`. Restrictions only narrow the
runner-level values; omitted entries and fields inherit them. The declaration
retains this member raw: `typed_harness_limits` validates it independently,
and `limits_for_harness` resolves inheritance. An error means warn with the
runner name and use runner-level values for every harness, not drop the runner.
Upgrade routing to 0.7.11 or later before declaring the member: older strict
declaration parsers reject it. `HeadChoicesRequest::with_harness` requests
harness-specific choices; without it head choices keep the runner-level union.

With the `select_harness` describe group, a caller may request `harness` using
`SelectRequest::with_harness`. Only a runner listing that program is a
candidate, and `runner.harness` echoes it. No hosting runner is `no_route`
with reason `harness_unavailable` and the requested program in the message.
Without a requested program, candidates are not filtered by harness and the
answer carries the declared default, else the sole listed program, else no
harness. No route or pair is ever relabeled as a program.

**Readers first:** a 0.3.x routing provider rejects a slot containing `runner`.
Runner modules must not declare it until the routing provider adopts 0.4.0.
Likewise, a 0.4.0 provider refuses declarations carrying the new program
members: runners declare them only after their provider serves 0.4.1.
A caller sends `harness` only when `role.describe` declares `select_harness`;
a provider without the group refuses that member by name.

With the `select_prefer` describe group, a caller may send `prefer` using
`SelectRequest::with_prefer`: up to 16 route keys with no duplicates. The
provider answers a listed adequate route in list order, never admitting one
the request would not otherwise get. A provider without the group refuses
`prefer` by name, like `hold_ok`.

With the `select_exclude_providers` describe group, a caller may send
`exclude_providers` using `SelectRequest::with_exclude_providers`: provider
ids, each the part of a route key before the `/`. No route from a listed
provider is answered, and `check_answer` refuses one that is. When every
adequate route is from a listed provider the select is the same `no_route`
an `exclude` listing those routes gets. `exclude` keeps its meaning. A
provider without the group refuses `exclude_providers` by name, so a caller
sends it only after `role.describe` declares the group.

`route.account_for` may refuse with `invalid_request`, `unknown_decision`,
`account_not_authorized`, `auth_required`, `account_hold`,
`account_quota_unknown`, `account_pool_exhausted`, `account_pool_reserve_limited`,
`account_selection_disabled`, `account_selection_unavailable`,
`admission_conflict` or `pin_generation_conflict`. `account_selection_disabled`
means account selection is off on this router; use your own credential. It is
module-wide, never per provider or per pool, and untimed: it carries no
`reset_at_ms`. A caller must not treat it as exhaustion or a hold, or retry
against another provider expecting a different answer.

With `pin_fallback` (`SelectRequest::with_pin_fallback`) beside a pin, an
unservable pin routes normally instead of refusing `pin_unavailable`. The
answer carries `pin_outcome` (`SelectAnswer::with_pin_outcome`), and on
`fell_back` its `detail` carries `pin_reason`. Without the field pins still
refuse.

Reports carry `usage` built with `Usage::new` and its `with_*` setters: token
counts matching the runner's names plus `duration_ms` and `first_token_ms`,
all bounded integers, and the caller's optional `cost_usd` claim, which the
provider may ignore. A failed report with a future `reset_at_ms` fences
only that decision's route on that account until the reset (capped at 24
hours; a past reset fences nothing). `run_finished` observations release the
runner's account pin.

The [contract](CONTRACT.md) is normative. See the
[conformance kit](../cortexkit-role-routing-conformance/README.md) and the
[role/conformance guide](../../docs/role-contracts.md) for implementation
testing.

## Provider service tiers

A caller sends `route.select.service_tier` and either
`usage.service_tier_applied` or `usage.service_tier_served` on a report only to
a router whose `role.describe.capabilities` declares `service_tier`. This gate
protects older strict report readers and prevents an older router silently
ignoring a faster-tier request. Unknown report usage members remain refused.

The requested levels are `standard`, `fast` and `ultrafast`, with absence meaning
standard. Only accounts whose plans include the tier are eligible; plan sources
are the router's knowledge, not a new configuration schema. Fallback proceeds
ultrafast to fast to standard, recording `service_tier_chosen`. The runner applies
the chosen level. A reported wire is a runner discovery entry with
`served: "reported"`, which permits served metadata only when that response
states it (runner CONTRACT §9.3.2). Prefer a provider-stated served tier; its
price stays unconfirmed if that tier or its rate is unrecognised. Without a
served statement, applied gives only an unconfirmed estimate; absent applied
means standard. Keep unfamiliar strings rather than inventing a rate. Never
infer served from applied. See CONTRACT §5.5.

Request levels use a small local enum, with no runtime runner dependency.
Unknown select levels receive `unsupported_tier` naming `service_tier`.
Record levels are strings; decoding retains unknown applied, served and chosen
values, with unfamiliar prices unconfirmed. Producer checks reject unknown
applied levels. Dev-only drift tests keep enum spellings and wire API lists
equal to the runner contract.
`select::check_answer` is stricter than record decoding: it refuses an
unrecognised chosen tier, a missing chosen level for an explicit request or an
upgrade, with `AnswerProblem::ServiceTier`.

## Continuing a paused run

A router declares `account_continue` in `role.describe.capabilities` to opt in
(see CONTRACT §9.1.1). Only the runner of a durably paused run sends
`route.account_for` with `mode: "continue"`, the original admission and currently
used decision,
exhaustion evidence and a retry-stable `selection_id`. The additive
`account_continue` module supplies `ContinueRequest`, `ExhaustedAccount`,
`parse_continue_request(params, describe)` and `ContinueAnswer`. The old account
request, answer and parser are unchanged; a non-declaring router refuses the mode.

For a run admitted without `route.select`, `previous_decision_id` initially
names the account decision in its admission answer's `reason.decision_id`.
After a switch it names the account decision the continuation offered. The
router resolves both select and account decisions; an account decision must
belong to the same `admission_id`, otherwise the continuation refuses
`unknown_decision`, as for an unknown id. Only the account may change: provider,
model, auth method, owner scope and harness stay fixed, and the offered decision
links to its predecessor in either space. No wire member or capability is added.
A router without account-decision resolution refuses `unknown_decision` as
before. The runner keeps the run paused on its original account until its own
quota reset, or waiting for a person to renew sign-in for `signed_out`, and does
not ask again for that pause. A later pause may ask with a new selection id.

With the additional `account_continue_signed_out` capability, a runner may send
`limit_window: "signed_out"` after a definite needs-sign-in signal. It omits
`pool`, `resets_at_ms` and `retry_after_s`. The router records the evidence and
excludes the credential from selections, offers and fresh admissions until the
vault reports it usable again. An unreachable vault, transient error or timeout
never triggers sign-in continuation; credit walls still do not trigger it.

The reply's `kind` is `offer`, `hold` or `refuse`. Offers switch only to the same
provider, auth method, model, owner scope and harness, never the exhausted account or an
account under a known hold for the relevant window or pool. The kit's hold check
blocks a whole account. The runner records a validated switch before sending and never uses an
expired offer. Unknown hold times remain unknown; a terminal refusal leaves the
run on its original account until its own reset, or waiting for a person to
renew sign-in for `signed_out`. A sign-in hold waits for another compatible
account, never the signed-out one. Transport retries keep the same
selection id; asking again after a hold or for another fresh eligibility check
uses a new one. Shared samples are in
`route-account-continue.json`, including identical-offer retries and refusals.
