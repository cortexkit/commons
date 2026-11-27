# cortexkit-bus-client

> [!WARNING]
> **Alpha.** cortexkit-bus-client is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

The shared Rust connection to the local ck-bus service. The module passes its
existing `Arc<subc_client_rs::SubcConsumer>`. Call
`subc_client_rs::launch_nonce()` before the module spawns children, as in any
supervised subc module.

```rust,no_run
# async fn example(subc: std::sync::Arc<subc_client_rs::SubcConsumer>) -> cortexkit_bus_client::Result<()> {
use cortexkit_bus_client::{BusClient, BusClientOptions};
let bus = BusClient::connect(subc, BusClientOptions::new("my_module")?).await?;
let connection = bus.connection();
let mut changes = bus.connection_changes();
bus.publish_event("fact.created", 1, None, "fact-42", "public fact").await?;
# let _ = (connection, &mut changes);
# Ok(()) }
```

## Logging

An outage logs one WARN when the connection is lost and one INFO when it is
restored, with the elapsed time and the number of reported failed retries. An
outage still open after 60 seconds logs an ERROR, then another every 10 minutes.
If NATS closes the connection because its credential expired, the disconnect and
a recovery within 60 seconds log at debug instead. The client normally redials
with the renewed credential right away; long-outage reports still apply.

Add `ASYNC_NATS_LOG_DIRECTIVE` to the module's `EnvFilter` to suppress the
dependency's reconnect and handshake logs while keeping its other WARN and
ERROR logs. The bus client still reports non-transient connection failures by
kind, but async-nats does not pass the server's handshake details to the
reconnect callback. Set `async_nats::connector=debug` in place of
`async_nats::connector=off` when investigating those failures.

```rust
use cortexkit_bus_client::ASYNC_NATS_LOG_DIRECTIVE;
use tracing_subscriber::EnvFilter;

let mut filter = EnvFilter::from_default_env();
for directive in ASYNC_NATS_LOG_DIRECTIVE.split(',') {
    filter = filter.add_directive(directive.parse().expect("valid bus log directive"));
}
```

With `cortexkit-log` 0.3.7 or later, pass a `Crates` policy to
`cortexkit_log::init_with_crates` instead. List your module's own crates so they
keep their existing logger names:

```rust
use cortexkit_log::{Crates, LevelFilter};

let crates = Crates::new()
    .own(["my_module_core"])
    .ceiling("async_nats", LevelFilter::WARN)
    .ceiling("async_nats::connector", LevelFilter::OFF);
```

## Credential lifecycle

The client calls `ckbus.credential` over a subc route whose caller identity the
daemon authenticates using the module's launch nonce, and connects to the
returned plaintext `nats://127.0.0.1:<port>` endpoint. It checks
the returned inbox prefix and lets `cortexkit-bus-nats` set it before connecting.
The prefix is either `_INBOX.{credential_public}` or, for a module that reads
hub records, `ckdel.{box}.{module}`. The bus module must not serve the new prefix
until readers run bus-client 0.1.7 or later with bus-nats 0.5.4 or later.
Nonces are signed through `ckbus.nonce_sign`; the module never receives a seed.

If the daemon refuses to open the credential route on initial connect,
`Error::RouteRefused { code, expected_back }` preserves its refusal code and
verdict. `Some(false)` means ck-bus needs operator action; watch the daemon's
module catalog for ck-bus to become available instead of polling the route.
`Some(true)` means ck-bus is restarting, and `None` means its return is unknown.
Internal retry behavior is unchanged.

`inbox_prefix()` returns an owned copy of the current connection's prefix. It
changes after credential re-issue for the per-credential form and stays stable
for the `ckdel` form. `box_id()` returns the computer's box id from the
credential's `box_{machine_id}` account, or `None` for another account form. Use
the id supplied by the bus module in that account; a module must never derive it
itself.

JWTs last fifteen minutes. A background task renews roughly five minutes before
`exp`, with zero to six seconds of process jitter. The dynamic NATS auth callback
reads the latest JWT for that connection's credential public key on each
reconnect. Renewal must keep the same inbox prefix; older replies that omit the
prefix leave it unchanged. Renewal does not replace the connection or emit a
change notification.

`ckbus_credential_superseded` and `ckbus_credential_revoked` from signing or
renewal cause one single-flight re-issue for the observed key. Managers are
shared by module id within the process, so concurrent refusals cannot repeatedly
revoke each other's replacements. `ckbus_generation_not_live` never re-issues.
Transient issuance refusals (`ckbus_census_unavailable`, `ckbus_not_ready`,
`ckbus_spawn_snapshot_unavailable`, `ckbus_signing_failed`) retain the current
credential and retry with exponential backoff from 250 ms to 30 seconds. The
[ck-bus contract](https://github.com/cortexkit/subconscious/blob/master/docs/ck-bus.md)
specifies backoff but no numeric schedule. Initial connect waits
through those refusals too; cancel its future to stop waiting.

Re-issue creates a **new connection with the new credential's inbox prefix**.
`connection()` returns a cloned snapshot, not a permanently updating handle.
Watch `connection_changes()` and reopen raw subscriptions and durable pulls on
the new snapshot whenever it changes. `EventReader` and `BucketWatch` handle this
rebinding for you. A durable resumes at its server-side cursor.
The watch value advances once the replacement is ready; a closed watch means
the maintenance task stopped (including on a terminal renewal refusal). Dropping
the client stops its maintenance task. Already held snapshots should not be used
after re-issue.

A dropped connection reconnects on its own for about 36 seconds (20 attempts),
so a bus restart of about 30 seconds keeps the same connection and emits no
change. After that the connection ends for good and the client replaces it the
same way as a re-issue, retrying with backoff from 250 ms to 30 seconds while the
bus is still down. `EventReader` and `BucketWatch` wait for the replacement
instead of returning errors from the ended connection. An in-flight renewal
does not hold up replacement. When the old credential is no longer current,
the connection skips reconnect delays and may make a burst of short failing
attempts before it closes. The client waits briefly for the old connection to
close before opening a replacement, but does not wait indefinitely.

After any unseen change is consumed, `changed()` returns `Err` when the
maintenance task ends and drops its sender. A terminal renewal refusal also
logs one warning containing only its refusal code, not the credential. Reconnect
with `BusClient::connect` or report the module as degraded when this happens.

### Troubleshooting connection replacement

The bus client logs `retiring bus connection for replacement` once at INFO,
with the server address and a process-local connection ID. After a credential
replacement, async-nats 0.50 may briefly log these errors for the old connection:

- `auth callback failed` with `connection credential was replaced`
- `max reconnection attempts reached`
- `expected INFO, got nothing`

The old connection can no longer sign with its credential. It skips reconnect
delays, but async-nats cannot cancel a reconnect already in progress and can
still log errors as the remaining attempts run out. These errors are harmless
for the retired connection; they do not mean the replacement failed. If
`connection_changes()` does not advance to a ready connection, or the live
connection reports authentication errors, investigate those separately.

## Hub connection

Use `HubClient::connect(subc, opts)` with the same daemon consumer and module
options when the module needs account-wide hub streams. It opens a separate
federation-account connection and does not require a `BusClient`. Opening a box
connection never requests a hub credential. The hub credential must name domain
`hub` and carry the issued box id, its `fed_{box}` account, and its
`ckdel.{box}.{module}` inbox prefix.

`HubClient::jetstream()` returns a JetStream context in domain `hub`;
`HubClient::client()` returns a raw NATS client for core publishes. Both are
snapshots; watch `connection_changes()` and get fresh handles after a replacement.
`hub.watch_bucket(name).await?` instead returns a `BucketWatch` for a named
bucket in that domain. It follows credential replacements, checks every delivery
against its own consumer and bucket, and reports bucket resets as described below.
The hub connection renews independently, with the same bounded NATS reconnect
behavior as the box connection.

If the first `HubClient::connect` returns `Error::HubUnavailable`, retry when the
module next needs the hub. An established client recovers from that refusal in
the background. While it recovers, `client()`, `jetstream()`, and `watch_bucket()`
return `HubUnavailable`. An open bucket watch that loses its connection waits
for the replacement and resumes on it. `Error::HubNotGranted` is terminal and
stops maintenance, closing the connection change watch.

## Publishing and reading rules

- Acknowledge a delivery **after your own transaction commits**, not before.
- `msg_id` is the fact's own stable id, not a delivery attempt id. Distinct facts
  need distinct ids, including across subjects in the same stream. Inspect the
  returned receipt's `duplicate()` flag.
- Bodies carry **no secrets and no tool output**. Publish a safe fact or pointer,
  not raw execution output. Event bodies carry ids, counters and enums and are
  limited to **4 KiB**; retrieve large content through its owner's own call.
  Inline bodies reach every reader whose durable matches the subject, with no
  per-reader visibility check; reserve them for pointers and facts every declared
  reader may see. Flow triggers must use
  [`BusClient::publish_claim_check`](https://docs.rs/cortexkit-bus-client/latest/cortexkit_bus_client/struct.BusClient.html#method.publish_claim_check),
  the `cortexkit_bus_trait::Stream::publish` claim-check path (empty payload plus content digest), with the publisher serving
  the body under the reading flow's own scope to check which agent may see it.
  The flow engine refuses inline trigger bodies.
- `publish_event(event, version, qualifier, msg_id, body)` uses naming constructors
  for `ck.{acct}.event.{module}.{event}.v{version}[.{reader}]`. The optional reader
  qualifier is a validated module token. This crate relies on commons' naming
  0.4 naming dependency and its `event_subject_with_reader` constructor.
- `publish_claim_check(event, version, qualifier, msg_id, digest, headers)` uses
  the same subject naming and the bus-nats codec, sending an empty payload with
  `Nats-Msg-Id` and `Ck-Content-Digest`. Store the body before publishing and
  serve it through the publisher's own `events_get` query under the reading
  flow's scope. Caller headers may not override either reserved codec header.
- `live_hint(kind, version, body)` is a plain-NATS, versioned publish on
  `ck.{acct}.live.{module}.{kind}.v{version}`; it is not a stored fact.

### Receiving

`live_hints(module, name, version)` yields only deliveries on that exact hint
subject, and `BucketWatch` accepts only its own consumer and stream with a valid
key in the watched bucket. Both drop other pushed deliveries before processing
their sequence metadata and count them in `forged_deliveries_dropped()`. Modules
that subscribe with a raw NATS client must perform the same subject and origin
checks themselves: the delivery inbox alone does not authenticate a message.

### Reading event bodies

`bus.event_reader(durable).await?` attaches to the module's existing EVENT
durable; ck-bus provisions it, and the client never creates or changes one.
Names must follow bus-naming's owner rule: `m_{module}` or
`m_{module}_{suffix}`, with a nonempty lowercase-letter, digit or underscore
suffix. Ownership does not grant new permissions: the module must already be
an authorized event reader. The durable must have at least one subject filter.

`reader.next().await?` waits for one record and returns `EventItem::Delivery`
or `EventItem::Malformed`. Only deliveries with parseable metadata identifying
the bound stream and durable and a subject matching a configured filter reach
the caller. Other deliveries are counted in `bus.forged_deliveries_dropped()`
and discarded without replying; the reader starts another fetch.

A valid delivery has a naming-parsed `subject`
(`module` is the publisher, plus `event`, `version`, optional `reader`),
`msg_id`, `body: Bytes`, `stream_sequence` and `delivered_count`. Its
`headers()` accessor returns a read-only `cortexkit_bus_trait::Headers` map;
`header(name)` looks up a stored header name. Repeated headers retain their
first value, including `Nats-Msg-Id`.

Call `delivery.ack().await?` **only after your own transaction commits**.
`nak(delay).await?` requests redelivery, and `term().await?` stops redelivery.
No read, drop or reconnect acknowledges anything automatically. After a
credential re-issue, the reader rebinds to the same durable on the new connection;
the server keeps its cursor and unacknowledged records may redeliver. Keep the
`BusClient` alive for credential maintenance.

Malformed records (for example, a missing `Nats-Msg-Id` or an invalid EVENT
subject that still matches a durable filter) carry a raw subject, a reason, and
a stream sequence. They expose only `term()`: log the reason and
terminate the record rather than retrying it. They are not terminated
automatically. `next()` returns `Err` for transport or bind failures, including
the credential-maintenance change stream closing.

### Module KV buckets

`bus.bucket(owner_module, bucket)?` resolves a declaration in bus-naming's
`MODULE_BUCKETS`, refusing undeclared buckets and modules that are neither the
owner nor a declared reader before making any NATS request. The bus service must
already have provisioned the bucket and grants; this client never provisions it.

- `bucket.get(key).await?` returns `Option<BucketEntry>` with `value: Bytes` and
  `revision: u64`. Deleted and absent keys return `None`.
- `bucket.put(key, value).await?` and `bucket.delete(key).await?` return their
  revisions. Only the owner may write, and put checks the declared value cap
  before sending. Delete writes a retained tombstone; it does not purge history.
- `bucket.watch().await?` returns a `BucketWatch`. Its `next().await?` yields
  `BucketItem::Put { key, revision, value }`, `Delete { key, revision }`, or
  `CaughtUp`. Each snapshot's current entries come first, followed by exactly
  one `CaughtUp` (also for an empty bucket), then live changes. No durable name
  is accepted.

After reconnecting to the same bucket, the watch resumes after its last delivered revision and
never repeats a delivered revision. With history 1, overwritten intermediate
values cannot be recovered; every key's latest retained value or delete is
delivered instead. Use an event stream if every intermediate change matters.
Owners must use delete, not purge: a removed tombstone cannot inform readers of
a deletion. Keep the `BusClient` alive; terminal credential maintenance failure
returns the same `Error::Transport` as the event reader.

An already-open watch logs once while its bucket is missing and retries with
backoff. If the bucket is replaced, `next()` returns `Error::BucketReset` before
delivering any entries from the replacement. Clear your cached entries and
revisions, then continue reading the same watch for a new snapshot and `CaughtUp`.

`Error::WatchCapacity` means the bucket's shared consumer slots are full. On
NATS 2.15.0 this maps code 400, JetStream error 10026, `maximum consumers limit
reached`. Watches are unnamed ephemeral ordered consumers. Their grants cannot
delete consumers; after a watch drops, its slot is reclaimed by the server's
30-second inactivity threshold, so rapid reopen attempts can temporarily fill
the bucket even with fewer live readers.
New watches fail immediately when capacity is exhausted. An already-open watch
rebinding after credential reissue logs once and retries with backoff from 1 to
5 seconds, allowing its old consumer to be reaped; after 60 seconds it returns
`WatchCapacity` if no slot is available. Terminal credential maintenance failure
interrupts the wait with `Error::Transport`.

## TLS and platforms

The local endpoint uses no TLS. However, async-nats 0.50 has unconditional
rustls dependencies, and the shared NATS backend selects its `ring` provider.
This crate uses the same explicit features with default features disabled; it
does not claim to remove `ring`. Native Windows builds can supply that provider
as existing Rust modules do.

## Tests

Unit tests use a fake subc transport and a loopback NATS protocol server for
issuance, nonce signing, renewal, refusal handling, backoff, single-flight, fresh
inboxes and publish helpers. The opt-in real-rig test uses the published testkit;
see its source for explicit binary paths and the rerun command.
The `bucket_live` tests reuse the testkit's isolated server with restricted
owner/reader JWTs matching the bus service's KV grants. Run them against the
checksummed NATS 2.15.0 binary with `CK_NATS_SERVER_BIN` and
`CK_REQUIRE_NATS_SERVER=1`; without a binary they report a skip.
