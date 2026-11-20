# Changelog

## 0.1.12

- Bucket watches warn once and retry with backoff while the bucket is missing.
  A replacement bucket yields `Error::BucketReset`, then its current entries
  and a fresh `CaughtUp` marker instead of resuming from the old revision.
- Add `HubClient::watch_bucket` for named buckets in the hub JetStream domain.
  It uses the same snapshot, reset, and delivery checks as module bucket watches.

## 0.1.11

- Report a NATS connection's retirement once at INFO and log the bus client's
  own remaining diagnostics for the retired connection at DEBUG. The INFO
  line explains the brief, expected errors that async-nats 0.50 can still log
  while the old connection stops retrying. Live connection log levels are unchanged.

## 0.1.10

- Drop forged JetStream push deliveries before live hints or bucket watches
  process their subjects or sequence metadata. Live hints require their exact
  subscribed subject; bucket watches require their own stream and consumer and
  a valid key in the watched bucket. The shared `forged_deliveries_dropped`
  counter reports rejected deliveries without exposing their payloads.

## 0.1.9

- Replace a connection promptly when its credential is re-issued or its NATS
  handler gives up reconnecting, even if renewal is still pending. Waiting
  event readers and bucket watches resume on the replacement.
- A connection with a stale credential skips reconnect delays. It may make a
  burst of short failing attempts before it closes. The client waits briefly
  for it to close before opening a replacement; the retry window for a current
  credential is unchanged.

## 0.1.8

- Add `HubClient` for a separate, on-demand federation-account connection.
  It uses the issued module inbox and the `hub` JetStream domain, and renews
  and replaces its credentials independently of the box connection.
- Report `HubUnavailable` (retry when needed) and `HubNotGranted` (terminal)
  without retrying either refusal in the client.

## 0.1.7

- Accept a credential's `ckdel.{box}.{module}` inbox prefix for hub replies and
  pushed records, while retaining per-credential inboxes for older replies.
  Renewals may omit the prefix, but cannot change it on an existing connection.
- Expose the bus module's box id and the current connection's inbox prefix.
  Per-credential prefixes change on re-issue; module deliver prefixes stay stable.
- Require bus-nats 0.5.4. The bus module must not serve the new prefix until
  readers run bus-client 0.1.7 or later with bus-nats 0.5.4 or later.

## 0.1.6

- `publish_event` sends a non-empty body with its SHA-256 as `Ck-Content-Digest`,
  the same header claim checks carry. A reader can check the bytes it
  received, and a publisher whose event format requires the body, the message
  id and the digest together can use this helper. An empty body is still sent
  without a digest, because an empty payload with a digest is the claim-check
  shape.

## 0.1.5

- Accepts cortexkit-bus-naming 0.5.

## 0.1.4

- Fix a connection leak after a bus restart. A replaced connection used to keep
  reconnecting for the life of the process, opening a socket and logging an
  auth callback error every 4 seconds, because its old credential is refused
  forever. Every connection now gives up after 20 reconnect attempts (about
  36 seconds), which ends a replaced connection and frees its task and socket.
- A bus restart of about 30 seconds still keeps the same connection. If a
  connection stays down longer and ends, the client replaces it the same way as
  a credential re-issue and notifies `connection_changes()`; event readers and
  bucket watches rebind to the replacement.
- Event readers and bucket watches wait for that replacement rather than
  returning errors from the ended connection. If credential maintenance has
  stopped, they return the error instead of waiting.
- Each connection holds one extra subscription under its own inbox, used only
  to notice when the connection has ended.

## 0.1.3

- Add `publish_claim_check` for flow-trigger events: an empty payload with a
  content digest and codec-protected headers. Publishers store the body first
  and serve it through their own `events_get` query under the reader's scope.
- Add declared module KV buckets: owners can put and delete values; owners and
  declared readers can get values and watch current state followed by live changes.
  Undeclared buckets, unauthorized access, oversized values and exhausted watch
  consumer slots have named errors. Watches use only unnamed ephemeral consumers.
- Watches share the event reader's credential rebinding and terminal transport
  error. After reconnecting they resume after the last delivered revision without
  repeating it. A key's latest retained value or delete is never missed, but
  intermediate values overwritten during disconnection are not delivered: readers
  needing every change must use an event stream, not a history-1 bucket.
  Delete markers remain the key's latest entry; owners must use delete, not purge.
- New watches fail immediately when consumer slots are full. Existing watches
  rebinding after credential reissue log once and retry capacity with 1-to-5-second
  backoff for up to 60 seconds, then return the named capacity error if still full.

## 0.1.2

- Accept `subc-client-rs` 0.41 as well as 0.40, so a module on the 0.41 SDK can
  use this crate with one SDK version in its lock. No API change.

## 0.1.1

- Add a body-bearing EVENT reader that only attaches to an existing module-owned
  durable and rebinds it after credential re-issue, leaving the cursor on the
  server and acknowledgements to the caller after their own commit.
- Expose parsed subjects, message ids, bodies, delivery metadata and read-only
  headers (first value wins), with explicit ack, delayed nak and term operations.
- Return malformed records separately with a reason and a term-only handle.
- Refuse event bodies larger than 4 KiB; events carry ids, counters and enums,
  while large content is retrieved through its owner's own call.

## 0.1.0

- Add a shared credential client for connecting Rust modules to the local bus
  through ck-bus without holding a signing seed.
- Renew same-key JWTs and single-flight revoked or superseded re-issues, with
  replacement connection snapshots and a change signal for reopening readers.
- Add stored-event and versioned live-hint helpers using the shared naming rules.
- Warn with the refusal code when terminal renewal failure stops maintenance,
  and document the closed change stream as a reconnect/degraded signal.

### Verification notes

- Linux tests, the opt-in real-bus durable/re-issue test, workspace Clippy and
  Windows GNU Clippy pass.
- ARM64 Windows MSVC is blocked by missing Windows CRT headers on the Linux cross
  host (`ring` cannot find `assert.h`). The native Windows job was refused for
  capacity; native Windows compilation is still owed.
