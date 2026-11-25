# Changelog

## 0.5.9

- Add `NatsConnection::connect_observed` to observe NATS events and initial
  connection failures without changing the state and permission watchers.

## 0.5.8

- Add `NatsStreamCursor::next_or_undecodable` to let callers settle messages
  from their own durable whose bus headers cannot be decoded. `next` is
  unchanged, and both methods still discard foreign deliveries without replying.
- Add `NatsStreamCursor::with_pull_expires` to set the server expiry for each
  fetch without changing the connection's request timeout. Zero is clamped to
  one nanosecond to keep the fetch bounded.

## 0.5.7

- Stream cursors and work queues drop forged messages sent to the reply inbox
  for a module's pull durable. Before decoding, they check the JetStream reply
  metadata against the bound stream and durable and check the message subject
  against the durable's filters (or the stream's subjects if it has none).
  Drops never reply on the message's reply subject, which could belong to
  another consumer. Genuine deliveries are returned unchanged, and a genuine
  delivery whose headers fail to decode still returns the decode error with
  nothing settled, so the server redelivers it.
  An unfiltered durable loads the stream's subjects at bind and fails closed if
  `$JS.API.STREAM.INFO.<stream>` is unavailable; filtered durables never make
  that request.
- `NatsConnection::forged_deliveries_dropped` also counts pull drops. The first
  one per cursor or work queue is logged at WARN with its subject.

## 0.5.6

- Register watches warn once while a bucket is missing and retry with backoff.
  If the bucket was replaced, they read its current entries instead of resuming
  at the old revision. `NatsRegisterWatch::next_change` reports a typed bucket
  reset; the `RegisterWatch` trait reports it as unavailable.

## 0.5.5

- Register watches no longer panic on a delivery from outside the watched
  bucket. Anyone allowed to create a JetStream consumer can point a push
  consumer at a watch's delivery inbox; async-nats's KV watch panicked on such
  a message's subject, and its ordered consumer could take the message's
  sequence as the resume point and skip genuine updates. Watches now drive
  their ordered push consumer directly and drop, without parsing, any message
  that does not come from their own consumer on the watched bucket with a
  valid key inside the watch's filter. Snapshot, update, tombstone and
  revision semantics are unchanged.
- Add `NatsConnection::forged_deliveries_dropped`, counting those dropped
  messages. The first one each watch drops is logged at WARN with its subject.

## 0.5.4

- Add `NatsConnection::connect_with_inbox_prefix` for validated custom inbox
  subjects without changing `ConnectConfig` or its per-credential default.
  The bus module must not serve the new hub deliver prefix until readers run
  bus-client 0.1.7 or later with bus-nats 0.5.4 or later.

## 0.5.3

- Expose a connection-state watch for connection loss, automatic reconnection
  and final closure without polling the client.

## 0.5.2

- Accepts cortexkit-bus-naming 0.5.

## 0.5.1

- A subscribe no longer fails on another operation's permission violation on
  the same connection. Only a denial for that exact subscription subject or a
  connection-wide authorization failure rejects an otherwise accepted subscribe.

## 0.5.0

- Adopt bus-naming 0.4 for reader-token event subjects and declared module KV
  buckets. Callers sharing naming types must update their direct dependency.

## 0.4.0

- Require a suitable server for every live test when `CK_REQUIRE_NATS_SERVER=1`;
  otherwise all live tests share one explicit skip path if no server is available.
  Linux CI installs checksum-verified nats-server 2.15.0 and requires those tests;
  macOS and Windows retain optional-server behavior.
- Verify Stream cursors on WorkQueue-retained streams with unlimited server
  delivery, including ack, delayed nak, progress, term and delivery counts.
- Exercise family-routed cursor terminal disposal through repeated crashes
  after persistent dead-letter publication and before term; finite WorkQueue
  binding behavior is unchanged.
- Verify the 120-second EFFECT_DEAD / DELIVERY_DEAD duplicate window from
  bus-naming's `DEAD_LETTER_DUPLICATE_WINDOW`. A longer crash gap may duplicate
  a diagnostic, not the work; unlimited delivery keeps terminalization retryable.
  Command relays retain the server default window; durable command-id acceptance
  records checked before work prevent duplicate execution.
- Adopt bus-naming 0.3 and bus-trait 0.3.1. No NATS method signature changes;
  callers sharing naming types must update their direct bus-naming dependency
  to 0.3 (construct specs through its constructors and add wildcard match arms).

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
