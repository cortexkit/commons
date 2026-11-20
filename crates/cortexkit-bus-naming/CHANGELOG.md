# Changelog

## 0.5.8

- Include both vault KV buckets in the machine leaf grant while keeping writes
  scoped to that computer's keys. A new test ensures the leaf grant remains a
  superset of every module's hub grant.

## 0.5.7

- Add two federation hub KV buckets for sealed shared logins and active vault
  manifests. Each vault writes only its computer's records and can read both
  buckets; the connectors module can read only active manifests. Shared-login
  confidentiality comes from sealing each list to its recipient, not from the
  hub read grant.
- Let the identity-log module read the residence epoch from the stream leader
  so sealed writes use the current epoch, even when a replica is behind.

## 0.5.6

- Let the message-bus module pull and acknowledge outgoing hand-off frames on
  `FEDSEND` through its durable `c_ckbus_fedsend` (`federation::FEDSEND_DURABLE`).

## 0.5.5

- Allow the memory and rooms modules to reply to flow-control requests from
  ordered consumers on `HUB_MEMORY` and `HUB_ROOMS`, the account-wide streams
  on the federation hub that every paired computer shares.
- Declare a durable event reader for device removals in the identity-log module.

## 0.5.4

- Add per-module user grants for direct hub access from the federation account.
  Each user can publish only to its own shared streams and their required APIs,
  subscribe only to its own delivery prefix, and cannot use per-machine
  federation subjects. The room grant accepts both `prefrontal` and
  `prefrontal-core` during the rename, with a separate delivery prefix for each ID.

## 0.5.3

- `cerebellum` reads Broca's run started, paused, and ended events through
  core NATS subscriptions.

## 0.5.2

- Add account-wide `HUB_MEMORY` and typed fleet memory subjects for replicating
  agent memories. The stream retains changes without expiry, refuses new
  writes at 1 GiB, and caps messages at 64 KiB. Leaf users can publish and read
  without purge or deletion access. Unlike `HUB_ROOMS`, the stream binding and
  leaf publish grant accept exactly two routing tokens (`kind.owner`).
- Fix ROOM, WAKE and PEER streams that NATS refused to create: 0.5.1 bound
  each to its local subject plus an overlapping `fed.>` catch-all. They now
  bind exact federated subjects for room posts, wake and peer handoffs, and
  peer custody replies. Publish subjects and consumer filters are unchanged.

## 0.5.1

- Add box-account FEDSEND and bus-only FEDSEND_DEAD, federation-account FEDOUT,
  validated outbound/inbound handoff subjects, and explicit file-storage stream
  descriptors. Frames are capped at 1 MiB and retained for seven days, with new
  writes refused when full. Both `prefrontal-core` and `prefrontal` may publish
  outbound PEER and WAKE handoffs.
- Add universal box-account publish denies for `ck.{acct}.*.fed.>`: every local
  user, including bus and system users, must receive these denies alongside its
  allow list. Only the federation import writes authenticated origins.
- Reserve the room/agent id `fed` for inbound frames, so locally addressed
  messages cannot collide with the authenticated-origin namespace.
- ROOM, WAKE and PEER now discard new and retain seven days instead of discarding
  old with a one-day age. The bus module applies these settings in place on its
  next start; a full stream now refuses publishes instead of dropping the oldest.
  Their bindings also accept inbound federation subjects.
- Add separate federation-only PEER and WAKE module readers for both delivery
  authority ids, with unlimited delivery attempts so delayed naks remain possible
  throughout retention. Existing participant grants remain unchanged: handoffs on
  a computer have the same participant visibility as its local PEER/WAKE messages.
- Add account-wide HUB_ROOMS, with 100,000 messages per room, oldest-of-subject
  eviction, no age limit and a 256 KiB message cap. Extend the leaf grant with room
  publishes and the same read/consumer APIs as HUB_IDLOG, never purge or delete.
  Box-account import/export rows belong to the bus module, not this crate.
- Refresh the checksummed hub and permission fixtures, and pin the new local
  federation names, limits, reader policies and denies in a checksummed fixture.

## 0.5.0

- **Breaking:** rename the module's bus names from `prefrontal-core` to
  `prefrontal`: its EVENT reader and reader-qualified filters, delivery authority,
  INBOX / CORE_WORK / ROOM durables, and session-liveness bucket. Its reads of
  other modules' buckets also use the new identity. Limits and other bucket
  readers are unchanged.
- A reader using this release must be deployed together with the module rename.
  Do not adopt it early: the new EVENT durable would collect a separate backlog,
  and INBOX / CORE_WORK cannot admit both delivery readers. Use 0.4.8 for
  readers-first deployment; other modules retain both publisher subject spellings
  in 0.5.0 until a later cleanup release.

## 0.4.8

- Other modules' EVENT filters and derived live-hint subscriptions now accept
  both `prefrontal-core` and `prefrontal` publisher subjects, so they keep
  receiving across the module's rename. Nothing prefrontal-core itself owns
  changes yet: its own EVENT reader and the reader suffixes addressed to it,
  its INBOX / CORE_WORK / ROOM delivery durables, its role as the only module
  that publishes agent wakes, and its session-liveness KV bucket. Those move
  in 0.5.0.

## 0.4.7

- The `aft` reader's filter for the second runner's send-state notices now
  names its reader token (`...send_state.v1.aft`), like every other
  send-state row; the bare subject it named never matched a per-reader notice.
- Remove a reader row whose publisher was retired.

## 0.4.6

- HUB_IDLOG discards new messages when its storage is full: an append is refused
  and no stored entry is ever dropped, because a reader that falls behind reads
  from the first entry. Clarify the federation fixture README.

## 0.4.5

- Add hub-only federation names, strict canonical box ids, stream and bucket
  descriptors, and the ordered leaf-user grant, with a checksummed JSON fixture
  for implementations in other languages. Existing local stream rows are unchanged.
- Allow the system user to read leaf connection state for relay-bridge fallback
  when direct connections fail, and for reconnecting after credential renewal.

## 0.4.4

- Raise the `astrocyte.multiplier` bucket's value limit from 8 KiB to 64 KiB
  (8 MiB for its 128 keys). One plan's report entry carries per-model rate
  detail and already reaches 16 KiB.
- Correct the documented `views` bucket key: it is the lowercase hex encoding of
  the whole schema pin string, not the pin's digest without `sha256:`. This is
  what every writer and reader already uses; only the text was wrong.

## 0.4.3

- Remove the retired `prefrontal-routing` module id from the event-consumer and
  live-hint reader tables and the generated permission fixture.
- Rename the `m_occipital` filter to `event.cerebellum.display_record_ready.v1`.

## 0.4.2

- Add `m_occipital` with `event.thalamus-harness.send_state.v1.occipital`,
  `event.broca.send_state.v1.occipital`, `event.cerebellum.tool_display.v1`,
  `event.aft.display_record_ready.v1`, and `event.fusiform.catalog_changed.v1`.
- Extend the existing `m_broca` compaction reader with
  `event.aft.late_result_ready.v1.broca`,
  `event.cerebellum.late_result_ready.v1.broca`,
  `event.prefrontal-core.late_result_ready.v1.broca`,
  `event.basal.late_result_ready.v1.broca`, `event.fusiform.catalog_changed.v1`,
  and `event.cingulate.card_settled.v1.broca`, preserving its cursor.
- Extend `m_thalamus-harness_late_results` with
  `event.prefrontal-core.late_result_ready.v1.thalamus-harness`.
- Add `m_magic-context_runs` with `event.thalamus-harness.turn_ended.v1` and
  `event.broca.run.ended.v1`, separate from the existing compaction cursor.
- Extend `m_colliculus_events` with `event.broca.run.ended.v1`; its existing
  `event.magic-context.memory_changed.v1` filter is retained unchanged.
- Extend `m_aft_sendstate` with `event.broca.send_state.v1.aft`, retaining
  `event.thalamus-harness.send_state.v1`, and add the separate `m_aft_cards`
  durable for `event.cingulate.card_settled.v1.aft`.
- Extend `m_plexus` with `event.cingulate.card_settled.v1.plexus` and
  `m_prefrontal-core` with `event.cingulate.card_settled.v1.prefrontal-core`.
  Basal needs no table row: `m_basal` already filters `event.>`, including
  `event.cingulate.card_settled.v1.basal`. Keep `prefrontal-routing` unchanged.
- Add the separate `m_astrocyte_attribution` durable with
  `event.prefrontal-core.attribution_fact.v1`,
  `event.magic-context.attribution_fact.v1`,
  `event.entorhinal.placement_fact.v1`, and `event.basal.attribution_fact.v1`;
  existing Astrocyte cursors are unchanged.
- Declare owner-written `aft.views` and `cerebellum.views` buckets with reader
  `occipital`, 256 KiB values, history 1, no TTL, and 32 MiB each (128 schema
  pins, allowing multiple tools and schema revisions within a bounded budget).
  Keys are schema pins' lowercase hex digests without `sha256:`; values are
  exactly what `tool.views` serves for each pin.
- Clarify that `LiveHintReader` and `LIVE_HINT_READERS` name live subjects using
  relative `event.{module}.{name}.v{N}` spelling and grant subscribe only on
  `ck.{acct}.live.{module}.{name}.v{N}`, with no durable or stream read.

## 0.4.1

- Accept declared bucket owners and readers and live-hint readers as module
  principals in permission files, while continuing to reject unknown principals.

## 0.4.0

- Add optional reader-module event subjects and a shared account-scoped parser
  for both forms, including hierarchical event names.
- Declare five owner-written KV buckets, their readers and retention/size caps,
  with normalized bucket names and validation.
- Add late-result, route-fact, payer, media, catalog, send-state and agent/event
  reader rows, plus the versioned Astrocyte steps-committed live hint.
- Allow multiple reviewed EVENT durables per module, named `m_{module}` or
  `m_{module}_{suffix}`, and grant every declared durable, never another owner's.
  The bootstrap planner updates filters without resetting cursors, so extend
  `m_prefrontal-core` and `m_striatum` in place. Other existing filters are unchanged.

## 0.3.4

- Add `striatum` (the routing module's new id) with the same reads as
  `prefrontal-routing`: Insula's quota events through its own durable
  `m_striatum`, and the runners' content-free `step.finished.v1` live hints, plus
  the standard participant grants for its own id. The old id is unchanged.

## 0.3.3

- `m_prefrontal-core` also reads `event.basal.flow_instance_rescheduled.v1`.
  At bootstrap, the ck-bus service adds the subject to the existing durable's
  filter in place, so the durable keeps its position in the stream.

## 0.3.2

- Separate live-hint readers from event durables with the closed
  `LIVE_HINT_READERS` table and `live_hint_subscriptions` API. The bus module
  should derive live permissions from this function, not rewrite EVENT filters.
- Narrow `prefrontal-routing`'s durable back to exactly `event.insula.>` while
  retaining its two runner step hints as live-only subscriptions. Bootstrap
  should narrow `m_prefrontal-routing` in place.
- Remove `thalamus-pi` and its filters from the reviewed readers. Bootstrap
  should leave the now-unlisted `m_thalamus-pi` durable alone.

## 0.3.1

- Add two exact runner `step.finished.v1` reader filters for routing's model
  speed statistics, retaining its existing event-family filter. ck-bus derives
  the subscription grants for non-stored live hints from these filters; this
  crate's checked-in permission snapshot remains unchanged.

## 0.3.0

- Add validated INBOX and closed-family CORE_WORK command subjects, streams and
  core-only module durable grants, plus DELIVERY_DEAD and diagnostic consumers
  on both dead-letter streams.
- Commands use WorkQueue retention, discard-new, no age eviction, 256 MiB and
  a 16 KiB message cap. EFFECT keeps its retention and accepts an in-place age
  update; ROOM, WAKE, PEER, EVENT and EFFECT_DEAD policies are unchanged.
- Require body pins until terminal business disposition for no-age queues;
  finite retention validation now refuses those queues. Add bootstrap consumer
  policy and cross-consumer filter/ownership validation.
- Make the existing server-default dead-letter duplicate window explicit at
  120 seconds (`DEAD_LETTER_DUPLICATE_WINDOW`). `StreamSpec::duplicate_window`
  is `Some` only for EFFECT_DEAD and DELIVERY_DEAD; `None` means omit the wire
  property and use the server default, never send zero. Command relays use that
  default window and require durable command-id acceptance records checked before
  work for lifetime protection against duplicate execution.
- Breaking: `StreamKind`, `TokenKind` and `LimitError` gain variants and become
  non-exhaustive; add a wildcard arm to downstream matches.
- Breaking: `StreamNames` gains fields and becomes non-exhaustive; use
  `StreamNames::new(account)` or `AccountNames::derive(account)?.streams()`.
  `StreamNames::all()` now returns `Vec<&str>`; consume its iterator rather than
  assuming a six-element array.
- Breaking: `StreamSpec` gains `max_message_size` and `duplicate_window` and becomes non-exhaustive;
  replace struct literals with `StreamSpec::new(kind, name, subjects)` followed
  by explicit policy fields, or use `shipped_streams` for normative policies.

## 0.2.10

- Add `cerebellum` as a reader of the runners' turn events through
  `m_cerebellum`, with four exact filters: `turn_started.v1` and
  `turn_ended.v1` from `thalamus-harness` and from `thalamus-pi`. Its grant
  keeps the ordinary participant permissions and adds only the EVENT-stream
  pull, acknowledgement and consumer-info permissions for `m_cerebellum`.
  This crate adds no live-hint permissions; the bus module that issues
  credentials derives its live-hint subscriptions from these filters.

## 0.2.9

- Add `thalamus-pi` (the Pi runner) as a reader of Magic Context's compaction
  notices through `m_thalamus-pi`, and widen `magic-context`'s filter to also
  read `event.thalamus-pi.compaction.>`. Its grant keeps the ordinary
  participant permissions and adds only EVENT-stream pull, acknowledgement
  and consumer-info permissions for `m_thalamus-pi`, matching the other
  runner readers.
- Add the exact filter `event.thalamus-pi.step.finished.v1` to `astrocyte`'s
  step-history reader (`m_astrocyte`), beside its other runner step-finished
  filters.

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
