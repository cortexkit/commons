# Changelog

## 0.3.0

- **Breaking:** publishing now returns `cortexkit_bus_trait::PublishReceipt`.
  Callers must distinguish a stored message from an idempotent retry or an id
  collision using `duplicate()`; `stream_sequence()` replaces `ack.stream_seq`.
- Emulate JetStream message-id deduplication per stream (one `InMemoryBus`,
  shared by clones). A duplicate returns the original sequence and creates no
  delivery, queue entry, or published event, even when its subject or digest differs.
- Default the deduplication window to two minutes. Configure it with
  `InMemoryConfig::with_deduplication_window` to match the production stream.
  Duplicates do not extend the original window, and expired ids can be stored again.
- Add `InMemoryBus::new_with_deduplication_clock` for deterministic expiry tests
  without sleeps; delivery leases and queue delays retain their real-time clock.
- Released together with `cortexkit-bus-trait` and `cortexkit-bus-nats` 0.3.0.
