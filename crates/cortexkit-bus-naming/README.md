# cortexkit-bus-naming

Validation of message-plane subject, stream, bucket, consumer, and grant names, along with topology limits.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-bus-naming = "0.3"
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

In 0.3.2, `prefrontal-routing`'s durable is narrowed back to Insula
(`event.insula.>`); its two runner step hints remain live-only readers.
Bootstrap should narrow `m_prefrontal-routing` in place and leave the removed,
now-unlisted `m_thalamus-pi` durable alone. `thalamus-pi` no longer appears in
either reader table or the generated permission fixture.
