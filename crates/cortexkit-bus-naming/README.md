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

In 0.4, EVENT readers may own several durables. Each name is `m_{module}` or
`m_{module}_{suffix}`, with suffix `[a-z0-9_]+`. Grants include all of a module's
declared durables, and no other module's. Bootstrap must use each row's explicit
name, not synthesize one name per module. The existing `m_prefrontal-core` and
`m_striatum` gain filters in place; mutable filter updates preserve their cursors.

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
