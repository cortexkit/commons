# cortexkit-bus-trait

> [!WARNING]
> **Alpha.** cortexkit-bus-trait is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

NATS-shaped message-plane traits for registers, streams, and work queues, with a shared backend conformance contract.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-bus-trait = "0.3.0"
```

See the [crate source](src/lib.rs) for the interfaces. Backends are available for [in-memory use](../cortexkit-bus-inmemory/README.md) and [NATS](../cortexkit-bus-nats/README.md).
