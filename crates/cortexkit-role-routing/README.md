# cortexkit-role-routing

Wire types, validation, refusal codes and vectors for `routing/v1`: the role
a routing provider plays when it decides which model a piece of model work
runs on, and on which runner.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-routing = "0.5.6"
```

To include the shared vector bytes in your build, enable `test-vectors`:

```toml
[dev-dependencies]
cortexkit-role-routing = { version = "0.5.6", features = ["test-vectors"] }
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
- `describe`: the `role.describe` answer, its optional `capabilities`
  groups, and `check_describe`;
- `capabilities`: the optional groups this version defines
  (`select_hold_control`, `select_harness`, `select_prefer`,
  `select_exclude_providers`);
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

Runners optionally declare `harnesses` (1–16 unique program tokens), a
`default_harness` listed there, and `starts_sessions` (true by default and
omitted when true). Use `RunnerDeclaration::with_harnesses`,
`with_default_harness` and `with_starts_sessions`, with corresponding getters.
An empty program list is refused, not read as "no program". Programs are
independent of wire/auth pairs: a runner that launches an agent program may
host several programs through one pair. A gateway runner sets
`starts_sessions: false` and never answers select, but account allocation for
its own turns, observations, reports and describe are unchanged.

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
