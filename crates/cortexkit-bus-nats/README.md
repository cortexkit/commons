# cortexkit-bus-nats

NATS and JetStream implementation of the [bus traits](../cortexkit-bus-trait/README.md), using `async-nats`.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-bus-nats = "0.5.8"
```

See the [crate source](src/lib.rs) for `ConnectConfig` and `NatsConnection`.

`NatsStreamCursor::next_or_undecodable()` keeps a message from the cursor's
own durable in flight even if its bus headers cannot be decoded. It returns
`Pulled::Undecodable` with the stream sequence, delivery count, subject, and
decode error. Settle either `Pulled` variant with `StreamCursor::ack`, `nak`, or
`term` before pulling again. Use `term(Some(reason))` when you know the message
cannot be processed. `StreamCursor::next()` still returns the decode error
without settling the message, so JetStream can redeliver it.

Use `NatsStreamCursor::with_pull_expires(duration)` to override the default
fetch expiry, which is the connection's request timeout. A zero duration is
clamped to one nanosecond to keep the fetch bounded.

Use `NatsRegisterWatch::next_change()` to distinguish a bucket reset from a
transient outage. On `NatsRegisterWatchChange::BucketReset`, clear the old
entries and revisions, then continue reading the same watch. The next updates
begin with the current entries of the replacement bucket. The `RegisterWatch`
trait's `next()` reports the reset as `BusError::Unavailable` with the reason
`register watch bucket {bucket} reset`.
