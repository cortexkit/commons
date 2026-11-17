# `send_state.v1` bus notice samples

These samples were derived from a live runner's outbox rows with identifiers
replaced. Each file contains the subject, the `Nats-Msg-Id` value as
`nats_msg_id`, the other headers, and a `body` string containing the exact
UTF-8 bytes of the inline JSON payload. The bodies are serialized through
`SendStateNotice`; the original field order, value types, sequence numbers
and states are preserved.

All samples use a runner named `runner-a`. Files `01`–`04` address module
`sender-a`: pending, active, superseded by a newer send with the same dedupe
key, and expired. Files `05`–`08` address module `sender-b`: pending, active,
paused, and finished. Both senders are reserved module principals, not people
or apps. The account prefix `ck.box_test` belongs to the bus.

A runner publishes a notice whenever the status answer for a keyed send by
a reserved sender changes, including receipt-only changes. The change's
record is durable before the notice enters a durable outbox. Delivery is at
least once. A replay keeps the same `<session_ref>:<send_id>:<seq>` message id
so the bus can drop it as a duplicate. Sequence numbers increase within one
session, are not dense, and are not comparable across sessions.

The body is only a wake-up hint. Readers ignore unknown members, accept open
state strings, and re-read `session.send_status` through the runner owning
the send. The contract also defines `retracted` and `not_delivered`, which
are not represented in these eight samples.

`SHA256SUMS` pins every sample and this README. The `test-vectors` feature
exposes the same packaged bytes through `vectors::send_state`. The
`send_state` tests compare every serialized body byte for byte, check the
envelopes and verify the checksums.
