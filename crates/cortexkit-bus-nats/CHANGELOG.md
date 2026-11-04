# Changelog

## 0.3.1

- Add `NatsConnection::close(deadline)`, which requests a connection-wide
  drain and waits for the connection to close. `CloseOutcome::Closed` means
  the connection's handler has exited, so the shared connection accepts no new
  NATS operations from any clone or handle built from it. `CloseOutcome::Pending`
  means closure wasn't confirmed within the deadline plus a final probe of up
  to 50 ms, usually because the connection is reconnecting. A background task
  keeps trying to queue the drain, which runs if the connection reconnects.
  async-nats processes no commands while reconnecting, so a caller replacing a
  connection after a credential change keeps the old one able to authenticate
  through its auth options or callback; `close` doesn't refresh credentials.

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
