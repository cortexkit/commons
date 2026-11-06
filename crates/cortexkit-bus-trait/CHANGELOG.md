# Changelog

## 0.3.1

- Add typed `DeliveryFamily` routing and `terminally_dispose_cursor`: publish a
  persistent diagnostic before term, routing effects to EFFECT_DEAD and other
  families to DELIVERY_DEAD. Subject-qualified deterministic ids avoid collisions
  across families. Business terminal receipts remain responsible for lifetime
  replay protection beyond the broker's duplicate window.
- Unlimited server delivery and terminal receipts checked before work prevent
  stranded unterminated messages and work re-execution during terminal retries.
  EFFECT_DEAD and DELIVERY_DEAD suppress duplicates for 120 seconds, as specified
  by bus-naming's `DEAD_LETTER_DUPLICATE_WINDOW`; a longer crash gap may append a
  second diagnostic with the same deterministic id. Command-stream relay
  republishes use the server default window, with durable command-id acceptance
  records checked before work as the authoritative guard against duplicate execution.
- No breaking public items. Existing finite-cap WorkQueue and effect-only
  `terminally_dispose` signatures and behavior are unchanged.

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
