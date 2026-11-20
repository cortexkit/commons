# cortexkit-bus-nats

NATS and JetStream implementation of the [bus traits](../cortexkit-bus-trait/README.md), using `async-nats`.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-bus-nats = "0.3.0"
```

See the [crate source](src/lib.rs) for `ConnectConfig` and `NatsConnection`.

Use `NatsRegisterWatch::next_change()` to distinguish a bucket reset from a
transient outage. On `NatsRegisterWatchChange::BucketReset`, clear the old
entries and revisions, then continue reading the same watch. The next updates
begin with the current entries of the replacement bucket. The `RegisterWatch`
trait's `next()` reports the reset as `BusError::Unavailable` with the reason
`register watch bucket {bucket} reset`.
