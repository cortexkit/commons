# cortexkit-role-step-transform-provider

Wire types and vectors for `step-transform-provider/v2`. Its four hooks
transform user and assistant text, tool calls before execution, and tool-result
text. Hooks use one route per session, opened under its scope and bound to
`(harness, session)`. The daemon attests the caller module; project context
comes from the route's scope stamp. Hook bodies carry no caller identity.

```toml
[dependencies]
cortexkit-role-step-transform-provider = "0.5.0"
```

The [contract](CONTRACT.md) is normative. `role.describe` and
`transform.declare` remain answerable on module-level or pre-session scoped
routes, so the plan composer can declare `{preset, params: {}}` before the
session or its hook route exists. A hook on such a route is refused per call
with `route_unbound`, never at bind. Declarations retain per-preset budgets,
availability policy and the four-hook subscription model.

## Focused conformance probes

The non-default `conformance` feature exposes:

- `conversation_state_ignores_project_root`: with fixed harness/session,
  folder aliases reach the same state and another attested runner reaches
  different state;
- `hook_on_non_session_route`: both non-session route kinds bind successfully,
  hooks return error code `route_unbound`, and a subsequent discovery RPC
  succeeds; this probe does not check refusal wording or discovery completeness;
- `declare_on_module_route`: declaration and discovery work before a session
  exists, on module-level and scoped routes.

These are focused probes, not a full provider suite. Implement their adapters
using real routes and a fresh test store, reading opaque markers from actual
provider state rather than reconstructing them from inputs. The state probe
expects `/var/probe-project` and `/private/var/probe-project` to be two spellings
of one folder. The shared runner helper takes
`&SessionIdentity { runner, harness, session }`, using bare runner ids without
normalizing them.

Enable `test-vectors` to access the packaged `vectors` module. Its
`step-transform-provider-v2` corpus and `SHA256SUMS` cover retained hook,
declaration, grant and admission behavior plus the v2 route boundary.
