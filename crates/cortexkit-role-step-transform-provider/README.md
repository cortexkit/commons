# cortexkit-role-step-transform-provider

Wire types and vectors for `step-transform-provider/v1`, whose hooks transform user and assistant text, tool calls before execution, and tool-result text.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-step-transform-provider = "0.4.1"
```

The [contract](CONTRACT.md) is normative. See the [role/conformance guide](../../docs/role-contracts.md) for the available suites and vector locations.

## Focused state-identity probe

The non-default `conformance` feature exposes `conformance::StateProbe` and
`conformance::conversation_state_ignores_project_root`. This probes one rule,
not a full provider conformance kit. An implementation adapter establishes a
session through an initial hook on a real daemon-attested runner route (this
role has no Setup wire op) and reads an opaque marker from actual provider
state, without reconstructing it from inputs. Use a fresh test store with
`/var/probe-project` and `/private/var/probe-project` as two spellings of one
folder. The same `(bare runner module id, harness, session)` must reach the same
state through either spelling; a different runner must not. The adapter
supplies the daemon-attested runner principal's bare module id, never its
`reserved:<id>` display form, another prefix or a body field. The shared
reference helper takes `&SessionIdentity { runner, harness, session }`, not
positional arguments.
