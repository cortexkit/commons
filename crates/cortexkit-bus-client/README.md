# cortexkit-bus-client

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

## Credential lifecycle

The client calls `ckbus.credential` over a subc route whose caller identity the
daemon authenticates using the module's launch nonce, and connects to the
returned plaintext `nats://127.0.0.1:<port>` endpoint. It checks
the returned `_INBOX.{credential_public}` prefix and lets `cortexkit-bus-nats`
set it before connecting. Nonces are signed through `ckbus.nonce_sign`; the
module never receives a seed.

JWTs last fifteen minutes. A background task renews roughly five minutes before
`exp`, with zero to six seconds of process jitter. The dynamic NATS auth callback
reads the latest same-key JWT on each reconnect. Renewal does not replace the
connection or emit a change notification.

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

Re-issue creates a **new connection with the new key's inbox prefix**.
`connection()` returns a cloned snapshot, not a permanently updating handle.
Watch `connection_changes()` and reopen raw subscriptions and durable pulls on
the new snapshot whenever it changes. `EventReader` handles this rebinding for
you. A durable resumes at its server-side cursor.
The watch value advances once the replacement is ready; a closed watch means
the maintenance task stopped (including on a terminal renewal refusal). Dropping
the client stops its maintenance task. Already held snapshots should not be used
after re-issue.

After any unseen change is consumed, `changed()` returns `Err` when the
maintenance task ends and drops its sender. A terminal renewal refusal also
logs one warning containing only its refusal code, not the credential. Reconnect
with `BusClient::connect` or report the module as degraded when this happens.

## Publishing and reading rules

- Acknowledge a delivery **after your own transaction commits**, not before.
- `msg_id` is the fact's own stable id, not a delivery attempt id. Distinct facts
  need distinct ids, including across subjects in the same stream. Inspect the
  returned receipt's `duplicate()` flag.
- Bodies carry **no secrets and no tool output**. Publish a safe fact or pointer,
  not raw execution output. Event bodies carry ids, counters and enums and are
  limited to **4 KiB**; retrieve large content through its owner's own call.
- `publish_event(event, version, qualifier, msg_id, body)` uses naming constructors
  for `ck.{acct}.event.{module}.{event}.v{version}[.{reader}]`. The optional reader
  qualifier is a validated module token. This crate relies on commons' naming
  0.4.0 path dependency and its `event_subject_with_reader` constructor.
- `live_hint(kind, version, body)` is a plain-NATS, versioned publish on
  `ck.{acct}.live.{module}.{kind}.v{version}`; it is not a stored fact.

### Reading event bodies

`bus.event_reader(durable).await?` attaches to the module's existing EVENT
durable; ck-bus provisions it, and the client never creates or changes one.
Names must follow bus-naming's owner rule: `m_{module}` or
`m_{module}_{suffix}`, with a nonempty lowercase-letter, digit or underscore
suffix. Ownership does not grant new permissions: the module must already be
an authorized event reader.

`reader.next().await?` waits for one record and returns `EventItem::Delivery`
or `EventItem::Malformed`. A valid delivery has a naming-parsed `subject`
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
subject) carry a raw subject, a reason, and a stream sequence (absent only for
invalid JetStream metadata). They expose only `term()`: log the reason and
terminate the record rather than retrying it. They are not terminated
automatically. `next()` returns `Err` for transport or bind failures, including
the credential-maintenance change stream closing.

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
