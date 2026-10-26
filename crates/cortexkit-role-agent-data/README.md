# cortexkit-role-agent-data

Wire types and vectors for `agent-data/v1`, the capability for moving an agent's data between Macs: participant operations, refusal codes, and transfer steps.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-agent-data = "0.2.4"
```

The [contract](CONTRACT.md) is normative. Enable the `test-vectors` feature to use the bundled [vectors](test-vectors/agent-data-v1/README.md); see the [role/conformance guide](../../docs/role-contracts.md) for context.
