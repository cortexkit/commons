# cortexkit-role-compaction-provider

Wire types and vectors for `compaction-provider/v1`: Setup, advance notices, views and `view.get`, `compaction.ride`, and the ceiling policy.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-compaction-provider = "0.7.0"
```

The [contract](CONTRACT.md) is normative. See the [role/conformance guide](../../docs/role-contracts.md) for the available suites and vector locations.

## Focused state-identity probe

The non-default `conformance` feature exposes `conformance::StateProbe` and
`conformance::conversation_state_ignores_project_root`. This probes one rule,
not a full provider conformance kit. An implementation adapter delivers Setup
through real daemon-attested runner routes and reads an opaque marker from the
provider's actual per-session state, without reconstructing it from inputs.
Use a fresh test store with `/var/probe-project` and
`/private/var/probe-project` as two spellings of one folder. The same
`(bare runner module id, harness, session)` must reach the same state through either
spelling; a different runner with the same harness and session must not.
The adapter supplies `runner` as the daemon-attested principal's bare module
id (for example `runner-a`), never `reserved:runner-a`, another prefix or
Setup's body. Calls to the shared reference helper take
`&SessionIdentity { runner, harness, session }`, so old positional calls cannot
silently use the wrong fields.
