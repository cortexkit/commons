# cortexkit-bus-trait

NATS-shaped message-plane traits for registers, streams, and work queues, with a shared backend conformance contract.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-bus-trait = "0.3.0"
```

See the [crate source](src/lib.rs) for the interfaces. Backends are available for [in-memory use](../cortexkit-bus-inmemory/README.md) and [NATS](../cortexkit-bus-nats/README.md).
