# Changelog

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
