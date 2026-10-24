# Changelog

## 0.2.8

- Additive patch: overlap validation uses the existing
  `EventConsumerError::InvalidFilters` refusal for the static reader table.
  No public enum variant or existing signature changes, and downstream
  dependency constraints stay unchanged.

- Give the step-history consumer three exact runner step-finished filters in
  addition to its existing event family, without an overlapping module wildcard.
  ck-bus updates the durable's filters in place while preserving its cursor.
- Refuse overlapping filters within one reviewed module's static filter list,
  including duplicates, before NATS can reject bootstrap. Keep disjoint filters
  on the same durable.

- Add validated `live_subject` and account-wide `live_subscription` builders
  for plain, non-stored hints on `ck.{acct}.live.<module>.<kind>`.

## 0.2.7

- Allow the bus module to publish only `event.subc.activity.v1`, so ck-bus can
  mirror the daemon's activity ledger; the daemon has no bus credential of its
  own. Keep the bus grant from publishing other modules' events.
- Let `prefrontal-core` read the four exact Basal flow-instance lifecycle events
  through its existing `m_prefrontal-core` durable, without widening its filter
  to other Basal events.

## 0.2.6

- Add `thalamus` (the Claude Code gateway) as a reader of Magic Context's
  compaction notices through `m_thalamus`, and widen `magic-context`'s filter to
  also read `event.thalamus.compaction.>`. Thalamus gets the same
  own-durable-only pull, acknowledgement and consumer-info grants as
  `thalamus-harness`.

## 0.2.5

- Add three readers to the event-consumer table for compaction between
  harness runners and Magic Context. `magic-context` reads runners' advance
  notices on `event.thalamus-harness.compaction.>` and `event.broca.compaction.>`
  through `m_magic-context`; `thalamus-harness` and `broca` each read Magic
  Context's view notices on `event.magic-context.compaction.>` through their
  own durable. As before, each reader's grant gains only pull, acknowledgement
  and consumer-info on its own durable.
- The golden's foreign-event refusals for a reader now skip that reader's own
  events, since broca, now a reader, may publish `event.broca.>` itself.

## 0.2.4

- Add `prefrontal-routing` as a third reader in the event-consumer table,
  consuming `ck.{acct}.event.insula.>` through its own durable
  `m_prefrontal-routing`, so model routing can follow Insula's quota samples
  instead of polling quota every 60 seconds. Its participant grant gains only
  pull, acknowledgement and consumer-info on that durable. ck-bus creates the
  durable at boot once it is built against this version.

## 0.2.3

- Add a closed, code-reviewed table of module-event consumers. The first reader,
  `prefrontal-core`, consumes `ck.{acct}.event.broca.>` through its own durable
  `m_prefrontal-core`. Its participant and delivery-authority grants gain only
  pull, acknowledgement, and consumer-info permissions for that event durable;
  the basal grant and grants for modules outside the table are unchanged.
- Add `astrocyte` as a second reviewed reader, consuming
  `ck.{acct}.event.insula.>` (including `quota_sampled.v1`) through `m_astrocyte`.
  Its participant grant gains only pull, acknowledgement, and consumer-info for
  that durable, never access to prefrontal-core's or basal's event durable.
- Expose `event_consumer_specs` and `flow_engine_consumer_spec` for ck-bus
  bootstrap. Both use the same `ModuleEventConsumerSpec` builder: pull, explicit
  acknowledgement, deliver-all, 30-second ack wait, and 1,000 max ack pending.
  The existing topology-only `ConsumerSpec` remains source-compatible.
- Permit hierarchical event names of one to four dot-separated tokens, each
  `[a-z0-9][a-z0-9_]{0,62}`. Empty levels, hyphens, wildcards, whitespace, and
  version-shaped levels (`v` followed by digits) are forbidden. Existing
  valid single-token names retain their subject bytes.
- **Reader migration:** readers who split event subjects by fixed position must
  now take the version as the last token, and the event name as everything
  between the module id and the version. For example,
  `event_subject("broca", "run.started", 1)` ends in
  `event.broca.run.started.v1`.
- Extend the permission golden and conformance tests with the filtered reader's
  durable allows and refusals for consumer creation/deletion, foreign durables,
  publishing Broca events, and direct stream reads.

## 0.2.2

- Raised the event stream's per-subject message cap to 100,000, extending replay history for busy event subjects while preserving the 1 GiB stream byte cap and seven-day age. Rebuild `ck-bus` to apply the updated limit to a running stream.
