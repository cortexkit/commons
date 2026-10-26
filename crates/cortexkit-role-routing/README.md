# cortexkit-role-routing

Wire types, validation, refusal codes and vectors for `routing/v1`: the role
a routing provider plays when it decides which model a piece of model work
runs on, and on which runner.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-routing = "0.4.1"
```

To include the shared vector bytes in your build, enable `test-vectors`:

```toml
[dev-dependencies]
cortexkit-role-routing = { version = "0.4.1", features = ["test-vectors"] }
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
  (`select_hold_control`, `select_harness`);
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

The [contract](CONTRACT.md) is normative. See the
[conformance kit](../cortexkit-role-routing-conformance/README.md) and the
[role/conformance guide](../../docs/role-contracts.md) for implementation
testing.
