# Changelog

## 0.3.0

- **Breaking:** `Stream::publish` and `terminally_dispose` now return the
  non-exhaustive, must-use `PublishReceipt` instead of `PublishAck`. Replace
  `ack.stream_seq` with `receipt.stream_sequence()` and check
  `receipt.duplicate()` before recording a new message as published.
- A duplicate receipt means no new message was stored: its sequence identifies
  the original publish in the stream's deduplication window. Accept a duplicate
  for an idempotent retry, or report an id collision if a new message was intended.
  Message ids must be distinct across subjects within a stream.
- `terminally_dispose` returns the same receipt semantics; a duplicate retry is
  still followed by termination of the exhausted work item. Keep dead letters in
  a separate stream to avoid collisions with work-item ids.
- Released together with `cortexkit-bus-inmemory` and `cortexkit-bus-nats` 0.3.0.
