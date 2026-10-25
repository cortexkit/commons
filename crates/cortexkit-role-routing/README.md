# cortexkit-role-routing

Wire types, validation, refusal codes and vectors for `routing/v1`: the role
a routing provider plays when it decides which model a piece of model work
runs on, and on which runner.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-routing = "0.4.0"
```

To include the shared vector bytes in your build, enable `test-vectors`:

```toml
[dev-dependencies]
cortexkit-role-routing = { version = "0.4.0", features = ["test-vectors"] }
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
  (`select_hold_control`);
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

**Readers first:** a 0.3.x routing provider rejects a slot containing `runner`.
Runner modules must not declare it until the routing provider adopts 0.4.0.

The [contract](CONTRACT.md) is normative. See the
[conformance kit](../cortexkit-role-routing-conformance/README.md) and the
[role/conformance guide](../../docs/role-contracts.md) for implementation
testing.
