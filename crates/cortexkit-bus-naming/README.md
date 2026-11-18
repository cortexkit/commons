# cortexkit-bus-naming

Validation of message-plane subject, stream, bucket, consumer, and grant names, along with topology limits.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-bus-naming = "0.4"
```

See the [crate source](src/lib.rs) for `validate_token`, `TokenKind`, and topology validators.

## Event durables and live hints

`EVENT_CONSUMERS` is the closed, reviewed durable-reader table.
`LIVE_HINT_READERS` separately grants exact live-hint subscriptions without an
EVENT durable, EVENT-stream read, or consumer create/info permission. Who may
observe another module's events or hints is a security decision reviewed in
code; runtime configuration cannot extend either table.

The bus module should derive live permissions from
`live_hint_subscriptions(&account, module_id)`, which returns the sorted,
deduplicated union of both tables under `ck.{account}.live.`. Do not derive live
permissions by rewriting only `EVENT_CONSUMERS` filters.

Bootstrap should leave removed, now-unlisted durables alone. `thalamus-pi` no
longer appears in either reader table or the generated permission fixture.

EVENT readers may own several durables. Each name is `m_{module}` or
`m_{module}_{suffix}`, with suffix `[a-z0-9_]+`. Grants include all of a module's
declared durables, and no other module's. Bootstrap must use each row's explicit
name, not synthesize one name per module. Mutable filter updates preserve each
durable's cursor.

Version 0.5 renames the `prefrontal-core` module's bus identity to `prefrontal`:
its EVENT durable and delivery durables are `m_prefrontal`, reader-qualified
filters end in `.v1.prefrontal`, and its bucket is `prefrontal.session-liveness`.
A reader using 0.5 must be deployed together with the module rename. Use 0.4.8
for readers-first deployment: it adds other modules' `event.prefrontal.*` filters
beside `event.prefrontal-core.*` and subscribes to both publishers' corresponding
ephemeral notifications under `ck.{account}.live.*`, without changing delivery
ownership. Both publisher spellings remain readable in 0.5. Bootstrap must not
create the renamed EVENT durable before the rename, since it would accumulate
a separate backlog; INBOX and CORE_WORK cannot host both delivery readers.

`AccountNames::event_subject_with_reader` appends the intended reader module
after the version. `AccountNames::parse_event_subject` parses both forms for
the account and returns the publisher, event path, version and optional reader.
Readers must not assume the version is the last token.

## Declared module KV buckets

`MODULE_BUCKETS` declares owner-only writers, explicit readers, value caps,
history, optional TTL in seconds, and total byte caps. Derive each name with
`AccountNames::module_bucket_name`: `CK_{ACCT}_{OWNER}_{NAME}`, uppercased with
hyphens and dots changed to underscores. `validate_module_buckets` checks the
declarations, including normalized-name collisions.

The caps budget maximum-sized values for 1,024 AFT status keys (2 MiB), 128
Astrocyte multipliers (1 MiB), 65,536 read marks (16 MiB), 65,536 Entorhinal
changes (32 MiB), and 16,384 session-liveness keys (16 MiB). All have history 1;
only status expires, after 10 seconds. The bus service must provision the
buckets and derive writer/read permissions from these declarations; this crate
does not provision them or add KV grants by itself.

`aft.views` and `cerebellum.views` are owner-written and readable by `occipital`.
Each key is the lowercase hex encoding of the whole schema pin string's UTF-8
bytes; the value is exactly what `tool.views` serves for that pin. Each bucket has a
256 KiB value cap, history 1, no TTL, and a 32 MiB total byte cap. This budgets
128 maximum-sized schema pins per owner, allowing multiple tools and schema
revisions without unbounded storage. The byte cap is a budget, not a key-count
limit.

## Cross-computer handoffs, rooms, and memories

`federation` describes hub-only streams, the federation account's `FEDOUT`,
canonical `BoxId`s and the per-computer leaf grant. Local streams use the account
token (`box_{box}`); remote destinations and authenticated origins use bare box
ids. `AccountNames::fedsend` and `AccountNames::inbound_federated` validate a
family from `room`, `wake`, `peer`, `sync` and a nonempty list of destination
tokens. Stream descriptors require file storage.

Every box-account user must receive `box_account_denies(&account)` alongside its
allow list, including bus and system users. These explicit denies protect the
`ck.{acct}.*.fed.>` namespace from forged origins even if an allow list is broad.
The older permission text fixture remains allow-only; the universal deny is
pinned separately in the local federation fixture. FEDSEND_DEAD may be published
only by the bus user.
Room and agent ids cannot be `fed`, because their local address position is
reserved for frames delivered from other computers. Session and destination
tokens do not occupy that position and retain their existing lexicon.

`federation_consumer_specs` supplies federation-only PEER and WAKE module readers
for `prefrontal-core` and `prefrontal` while deployments move to the latter id,
without changing agent durables. Use the reader for the deployed identity; these readers route handoffs
and custody replies to agents. They have unlimited delivery attempts so delayed
naks can defer a frame throughout the seven-day retention.

`HUB_ROOMS` stores `ck.fleet.room.{room_id}` with a per-room history cap, not a
box-scoped subject (`fleet` cannot be a canonical box id). Its leaf APIs match
HUB_IDLOG's read and consumer APIs and permit neither stream purge nor delete.

`HUB_MEMORY` stores memory changes on `ck.fleet.memory.{kind}.{owner}`. Publishers
encrypt each change before sending it so the hub cannot read its contents.
Use `federation::hub_memory_subject` to validate the kind and owner. `project` and
`primer` use a project ID, `workspace` uses a workspace ID, and `global` and
`observation` use `all`. The owner is one token and cannot be `fed`. The hub
binding and leaf publish grant accept exactly those two token positions.

Each computer reads changes in stream order with its own durable consumer.
To let new computers replay from the beginning, the stream has no expiry and
refuses new writes once it reaches 1 GiB. It caps each message at 64 KiB. Leaf
users have the same read and consumer APIs as for `HUB_ROOMS`, not purge or
deletion access to stored changes.

The bus module owns the box-account import/export rows; they are not described
by this crate.
