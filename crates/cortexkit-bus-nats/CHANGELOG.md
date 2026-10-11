# Changelog

## 0.3.0

- **Breaking:** `NatsStream::publish` now returns
  `cortexkit_bus_trait::PublishReceipt`, preserving JetStream's `duplicate` flag
  instead of reporting a deduplicated publish as an indistinguishable success.
  Use `receipt.stream_sequence()` instead of `ack.stream_seq` and inspect
  `receipt.duplicate()` to accept an idempotent retry or report an id collision.
- Continue to send the supplied id verbatim as `Nats-Msg-Id`: distinct messages
  need distinct ids across subjects in a stream. A duplicate receipt refers to
  the original stored sequence, not a newly stored message.
- Add a real-server duplicate-publish regression test with a named skip message
  when `nats-server` is unavailable.
- Released together with `cortexkit-bus-trait` and `cortexkit-bus-inmemory` 0.3.0.
