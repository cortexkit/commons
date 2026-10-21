# `compaction-provider/v1` — role contract (draft)

Stability: **alpha, draft**, unpublished. This document is the role
definition; the Rust types in this crate are its wire
shapes, `test-vectors/compaction-provider-v1/` at the repository root holds
its vectors, and a separate `-conformance` crate will hold its suite (§16).

`llm-runner/v1` states what the runner owes on the same interface; this
document states the provider's side, and the runner's side where the two
must agree byte for byte.

Every item is marked **[pinned]**: a settled, binding requirement, even before
the crate's first release. A provider must
do it, and a runner may rely on it.

Names and types this role shares with `llm-runner/v1` (the `compaction`
group, the session reference, the model view's `source`, the runner's codes)
are taken from that crate, which this crate depends on, so the two cannot
drift apart.

A compaction provider is any module that decides the shape of the history a
runner sends to the model. It works off the runner's hot path: the runner
announces what it wrote over ck-bus (§5); the provider reads it, decides on
its own when to reshape the history, and publishes a view (§6); the runner
applies the newest view it holds to every request (§7). The only synchronous
contacts after Setup are the ride request and the ceiling pause (§8).

## Terms

- **Plan composer**: the component that writes a session's plan and freezes
  it. It checks each provider's runner groups against the runner, places
  the providers in order, and fills the plan's user-tier fields.
- **Runner group**: a capability group a runner advertises in its
  `llm-runner/v1` `role.describe`, such as `compaction` or
  `transcript_reads`. A provider lists the groups it needs in its own
  `role.describe.runner_groups`.
- **Reduction owner**: the session's compaction provider, the only party
  that may remove or rewrite history already written. Every other step
  transform may only add to it.
- **View**: the provider's preferred shape for the prefix of a runner's
  transcript through one message (§6).
- **Signed thinking**: model reasoning blocks that carry a model provider's
  signature. Any change to them, even one byte, invalidates the signature,
  so nothing in these roles ever edits them.

## 1. Role identity and addressing

- [pinned] A provider lists `compaction-provider/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe`, the Setup op
  `compaction.setup` (§4), `view.get` (§6) and `compaction.ride` (§8). A
  provider of this role serves all four; there is no optional group of ops.
- [pinned] A runner refuses a module whose `role.describe` lacks a required
  op, by name, before routing anything to it.
- [pinned] Compaction routes are module-level and unscoped. Setup names the
  opaque session handle; `view.get` and `compaction.ride` name the session
  reference. Neither is an authorization credential. The daemon-attested
  principal on the route identifies the runner; the caller-chosen `harness`
  string is not attested and never identifies or authorizes a runner.
- [pinned] Setup carries required strings `harness` and `session`, the
  session's caller-chosen identity as reported by the runner. Setup does not
  carry `project_root`; the provider takes it from the route that delivered
  Setup. A Setup without `harness` or `session` does not decode; neither is
  defaulted.
- [pinned] A runner reports its session's own `harness` and `session` in
  Setup, and opens the provider route with that session's `project_root`.
  A route bound to a single session carries that same session. A route shared
  at project scope may instead have an empty bind session and a generic bind
  harness such as `runner`.
- [pinned] At Setup, the provider records the authoritative reverse-call
  identity: Setup's `harness` and `session`, together with the incoming
  route's `project_root`.
- [pinned] Calls about the conversation—`session.read`, `session.head`,
  `session.resolve`, sends into it, and reminders—open the provider's own
  route to the runner with exactly that identity. They never use a constant or
  the Setup route's harness or session. A reverse `session.read` can find
  messages and lineage only under the identity carried by its route.
- [pinned] A child run the provider creates is a separate session owned by
  that provider. Its route binds the Setup route's `project_root`, Setup's
  `harness`, and the child's own new session. It never binds the conversation's
  session.
- [pinned] If the incoming route has a non-empty bind session and that value
  differs from Setup's session, the provider may refuse Setup with
  `setup_identity_mismatch` (§4, §10, §13.3). It does not compare harnesses;
  the harness is caller-chosen and unattested. The provider recognizes a
  runner only by the daemon-attested principal on the route.
- [pinned] A provider keys a runner conversation, and every piece of
  per-session state it keeps for one, on `(project_root, session,
  harness)`: the project root it serves the request for, the session
  handle and the harness. Two requests that differ in any of the three
  belong to different conversations. The session reference
  (`llm-runner/v1`'s `session_ref`, its §14.1) is the digest of the same
  three values; notices, views, `view.get` and `compaction.ride` name the
  session by it.
- [pinned] Every request in this role is `{method, params}` (`OpRequest`):
  `method` is the op's name and `params` its request. `role.describe` takes
  empty params (`DescribeRequest`).
- [pinned] The two ck-bus events of this role are the runner's advance
  notice (§5) and the provider's view-ready event (§6). Every event this role
  defines puts its event name in its bus message id: the event stream
  deduplicates message ids across the whole stream, not per subject, so two
  events of different kinds with the same number would otherwise collide,
  and the broker would drop the second as a duplicate.

## 2. `role.describe`

- [pinned] The answer is `{majors: [{version, ops, stability}],
  implementation_version, capabilities, runner_groups?,
  setup_descends_from?}` (`RoleDescribe`), the shape `tool-provider/v1` and
  `llm-runner/v1` use. `version` is spelled as in the manifest,
  `compaction-provider/v1`.
- [pinned] Stability is data, never part of a name: `alpha`, `beta` or
  `stable`, read as `alpha` when absent. This draft is `alpha`.
- [pinned] Decoded leniently: unknown fields, majors, ops and capabilities
  are ignored, and an unknown `stability` decodes. Only the
  `compaction-provider/v1` major and its required ops are checked strictly
  (`check_describe`, `missing_op`).
- [pinned] The answer describes the provider build, never a session, so a
  consumer may cache it for as long as it talks to the same module
  incarnation.
- [pinned] This role defines no identifiers for the `capabilities` list;
  optional support is advertised by named describe members.
- [pinned] Optional boolean `role.describe.setup_descends_from: true`
  declares support for Setup descent (§4). Absent or false means
  unsupported; false is omitted when encoded.
- [pinned] The provider declares what it needs from the runner in
  `role.describe.runner_groups`: the runner groups it needs besides
  `compaction`, such as `transcript_reads` for reading new messages. A list
  of strings, omitted when empty.
- [pinned] The plan composer checks `runner_groups` against the runner
  groups the runner advertises when it composes the session. An unmet group
  fails the launch, and the failure names each unmet group
  (`unmet_runner_groups`). Runner requirements are carried nowhere else.

## 3. Pairing with a runner

Admission is the runner's check of a proposed session plan before it accepts
the session and freezes that plan for use.

- [pinned] A session has at most one compaction provider. Without one,
  history is sent as written.
- [pinned] Hosting a compaction provider is the runner group `compaction`
  (`RUNNER_GROUP`), all or nothing. A runner that declares it implements the
  runner's side of this document: it calls Setup (§4), publishes advance
  notices (§5), fetches and applies views (§6, §7), sends rides and applies
  the ceiling (§8), and writes native markers where its host has them (§9).
  A runner that does not declare it has no compaction provider: it refuses a
  plan naming a compaction item at admission with `invalid_params {field:
  "plan.compaction_item"}`.
- [pinned] The compaction provider is the session's one reduction owner:
  the only party that may remove or rewrite what was written. On the
  step-transform side, only it may `replace` on `pre_user` and
  `post_assistant`; every other step transform is preserving and runs in a
  separate ordered list (`step-transform-provider/v1` §5).
- [pinned] The plan names the provider in its `compaction_item`, with the
  item's `preset` and `params`. The runner passes the preset, the params
  and the session's composition to Setup verbatim.
- [pinned] The runner finds the session's compaction provider at
  `plan.compaction_item.provider` (`llm-runner/v1`'s
  `plan_compaction_provider`). The item is `{provider, preset, params}`;
  `provider` is a module id, `preset` a string, and `params` an opaque
  object.
- [pinned] New messages are read with `llm-runner/v1`'s `session.read`.
  This role defines no delta op.

## 4. `compaction.setup`

- [pinned] Called once per session, before its first model call. Its answer
  is recorded before the first model call; once recorded, Setup never runs
  again for the session.
- [pinned] If Setup fails or times out with no answer, no model call is
  made and the run ends `error` with `provider_code`
  `compaction_unavailable`, `llm-runner/v1`'s code, which this crate
  re-exports (`errors::runner_codes::COMPACTION_UNAVAILABLE`). It is not
  retryable (§13.3). A Setup answered `refuse` ends it with the provider's
  own `code` instead (§10). Either way the next send calls Setup again,
  because no initial view was recorded. So a provider answers Setup for a
  session it has seen before, and its answer is well formed whatever it
  answered last time (kill point `SetupRecorded`).
- [pinned] The request (`SetupRequest`) is `{session, harness, request_id,
  preset?, params, composition, model, variant?, context_window?,
  output_limit?, newest?, lineage_id?, descends_from?, now}`. `model` is a
  string; `variant`, `context_window` and `output_limit` are sibling fields,
  not a nested model object. `params` defaults to `{}`; an absent `preset`
  selects the provider's default. `composition` is an opaque object passed
  verbatim. `newest` is `{ordinal, mid}` when a message is already written;
  `lineage_id` is absent when no lineage exists. `now` is milliseconds since
  the Unix epoch.
- [pinned] Before accepting Setup, the provider checks whether the incoming
  route is bound to a single session. If its bind session is non-empty and
  differs from Setup's session, the provider may refuse with the named,
  non-retryable code `setup_identity_mismatch` (§10, §13.3). It does not
  compare Setup's harness with the route's bind harness: routes shared across
  sessions may carry a generic harness. For reverse calls, it combines
  Setup's `harness` and `session` with the incoming route's `project_root`.
- [pinned] For calls about the conversation—`session.read`, `session.head`,
  `session.resolve`, sends into it, and reminders—the provider opens its own
  route to the runner using exactly the identity above. It never uses a
  constant or the Setup route's bind harness or session. A reverse
  `session.read` can find messages and lineage only under the identity its
  route carries.
- [pinned] A child run the provider creates is a separate session owned by
  that provider. Its route binds the Setup route's `project_root`, Setup's
  `harness`, and the child's own new session, never the conversation's session.
- [pinned] Optional `descends_from: {lineage_id, through_ordinal}`
  (`lineage::DescendsFrom`) declares that the session's first lineage
  continues another lineage of the same conversation, through the inclusive
  `u64` cut. The inherited prefix keeps its message ids and ordinals; no
  later history is inherited. Before accepting it, the provider validates
  that the parent and cut exist and that it holds every retained message
  through the cut. An unknown parent, a cut it does not hold, or a gap in the
  retained prefix answers `refuse` with `history_unreadable`; it never
  guesses missing history or saves a link from a refused declaration.
- [pinned] The provider durably saves an accepted descent link before
  replying. A repeated Setup (after a failure) with the same parent and cut
  is an idempotent no-op, not a second inheritance. One with a different
  parent or cut conflicts with the saved link and is refused with
  `invalid_params`, field `lineage_id` (§13); the saved link remains
  unchanged. The reference check is `lineage::DescentLink`; providers still
  validate and persist their own retained history and link. A Setup without
  `descends_from` makes the session's first lineage a root.
- [pinned] A runner sends `descends_from` only when the provider's
  `role.describe.setup_descends_from` is true (§2). Setup decodes leniently
  (§14), so a provider without that support would ignore the member; decoding
  is not a declaration of support. Later forks within the session are
  announced on advance notices instead (§5).
- [pinned] The answer (`SetupAnswer`) is `{answer: "ready", request_id,
  initial, ceiling?}` or `{answer: "refuse", request_id, code, reason,
  provider_code?}` (§10).
  - `initial` is a CompactionMessage (`message::CompactionMessage`,
    `{compaction_id, version, range: {from, to}, replacement}`): the
    provider's finished messages, in the runner's message schema, standing
    for the half-open ordinal range `[from, to)`. Usually the range is empty
    at the start of the lineage, `{from: 0, to: 0}`: the provider's head is
    inserted before every message, and the model view shows it as the
    insertion `[0, 0)` (§11). A provider with no head returns an empty
    replacement. Only `from > to` is malformed (`range_inverted`); a runner
    treats an answer whose `initial` is malformed as a failed Setup.
  - The runner renders `initial` until it applies the session's first view
    (§7). A view replaces it entirely and is never a delta over it.
  - `ceiling` (`ceiling::CeilingPolicy`) says when the runner pauses for a
    view and, failing one, refuses the run (§8). It is omitted when absent.
  - A `refuse` ends the run `error` with the refusal's role `code`, and the
    next send runs Setup again (§10).
- [pinned] Setup freezes nothing else. The stability ranks of the provider's
  own messages and the `call_when` conditions that earlier drafts froze are
  gone: they described the per-step call, and a view's cache behaviour is the
  runner's to manage. A `ready` answer that still carries `stability` or
  `call_when` decodes, and both are ignored.

## 5. Advance notices: runner to provider

The bus carries no message content: the broker transports ids, digests and
headers, never bodies. A notice is a claim check that only wakes the
provider.

- [pinned] After each durable write to a session (a message, or a runner
  event below), the runner publishes an advance notice
  (`notice::AdvanceNotice`) on
  `ck.{acct}.event.<runner_module_id>.compaction.advanced.v1`
  (`notice::event_subject` with `notice::ADVANCED_EVENT`). The bus message
  id is `<session_ref>:advanced:<seq>` (`AdvanceNotice::message_id`), which
  names its event for the reason in §1. The bus digest is the notice digest
  below. `session_ref` is `llm-runner/v1`'s session reference: 32 lowercase
  hexadecimal characters.
- [pinned] `seq` is the session's notice sequence: 1 for the first notice,
  one higher for each later one. The runner assigns it when it records the
  notice durably, before publishing. A notice republished after a crash
  reuses its `seq` and carries identical headers, so the bus discards the
  duplicate by id.
- [pinned] Headers (`notice::headers`). Every value is ASCII. Every number
  is a canonical decimal `u64`: digits only, no sign, no leading zero except
  `0` itself. A header this contract does not define is ignored and is not
  part of the digest.

| Header | Required | Value |
|---|---|---|
| `session_ref` | yes | the session reference |
| `seq` | yes | the notice sequence, at least 1 |
| `lineage_id` | yes | the session's current lineage, the one `through_ordinal` counts in: 1 to 256 bytes of printable ASCII without space; it may contain `:` |
| `through_ordinal` | yes | the newest ordinal written in the current lineage when the notice was recorded |
| `through_digest` | yes | the content digest (below) of the message at `through_ordinal` |
| `descends_from` | no | on a notice that moves the session to a new lineage only: `<lineage_id>:<through_ordinal>:<through_digest>` (fork descent, below) |
| `applied_view_version` | no | the version of the view the runner renders now; absent before its first view, and after a discard leaves it rendering none (§7) |
| `events` | no | event tokens, comma-separated with no spaces, in the order they happened; omitted when there are none, never empty |
| `est_tokens` | yes | the runner's estimate, in tokens, of its next request under its current view plus the messages after it |
| `window` | no | the current model's context window in tokens; omitted when unknown |
| `model` | yes | the current model id: 1 to 256 bytes, printable ASCII, no space |
| `last_usage` | no | the last response's measured usage, `<input>:<cache_read>:<cache_write>:<output>`; omitted before the first response |

- [pinned] Event tokens (`notice::Event`). Every token follows one grammar:
  a name `[a-z][a-z0-9_]*`, then zero or more `:<argument>` segments of
  `[A-Za-z0-9._-]+`, at most 128 bytes in all. A notice carries at most 32
  tokens and never repeats a token. The known tokens:

| Token | Meaning |
|---|---|
| `model_change` | the session's model changed |
| `surface_change` | the frozen system text or tool surface changed |
| `ttl_expired` | the prompt cache expired |
| `revert_below:<ordinal>` | the transcript was truncated; every message at `ordinal` or above is gone |
| `marker_outcome:<applied\|deferred\|unknown>:<cut_id>` | the host's native compaction marker for the cut of the view named by `applied_view_version` in the same notice (§9). `cut_id` is 1 to 64 bytes of `[A-Za-z0-9._-]` |
| `view_discarded:<view_version>` | the runner discarded that view because its coverage is no longer a prefix of the transcript; always with `revert_below` in the same notice |
| `view_refused:<view_version>:<reason>` | the runner will not render that view and keeps its current one (§7). `reason` is 1 to 32 bytes of `[a-z0-9_]`: `tool_pair_split`, `role_order`, `unknown_part`, `provider_constraint`, or another reason a provider treats the same way (`notice::refusal_reasons`) |
| `view_deferred:<view_version>:<reason>` | the runner holds that view until a step boundary where applying it is safe for the current model provider, and keeps rendering its current one meanwhile (§7). `reason` has the same form: `signed_reasoning`, or another (`notice::defer_reasons`) |
| `view_companion:<view_version>:<edit>` | the runner applied that view together with a companion edit the model provider requires, and nothing more (§7). `edit` has the same form: `strip_signed_reasoning`, or another (`notice::companion_edits`) |

  A known name with arguments it does not take is malformed. An unknown
  name that follows the grammar decodes (`Event::Unknown`) and the provider
  ignores it.
- [pinned] `applied_view_version` is the runner's durable acknowledgement
  of views. Every notice repeats it, so a lost notice loses no
  acknowledgement, and there is no separate `view_applied` event. The
  provider builds its next view against the view the runner renders now.
- [pinned] `ttl_expired` may be declared, as an event or a ride cause,
  **only** by a runner that knows the cache lifetime it actually sent. A host
  whose model SDK places its own cache markers declares `model_change` and
  `surface_change` only.
- [pinned] A notice is refused (`AdvanceNotice::from_headers`,
  `NoticeProblem`) when a required header is missing (`missing:<header>`), a
  value breaks its encoding (`malformed:<header>`: `seq` or
  `applied_view_version` of 0, a digest not lowercase hexadecimal, a
  non-canonical number, a lineage id outside its grammar, a malformed token,
  a `descends_from` that does not parse or names the notice's own lineage, a
  value over 1,024 bytes), a token repeats (`duplicate_event`), there are
  more than 32 tokens (`too_many_events`), `view_discarded` comes without
  `revert_below` (`view_discarded_without_revert_below`), or
  `marker_outcome` comes without `applied_view_version`
  (`marker_outcome_without_applied_view_version`). The provider drops a
  refused notice; the next well-formed notice wakes it.
- [pinned] Size bound: the headers this contract defines total at most
  4,096 bytes (`MAX_NOTICE_HEADER_BYTES`), counting every name's and value's
  length as the bus counts a message's headers. The per-value bounds keep
  every well-formed notice at or under 2,280 bytes; a runner with more events
  than fit splits them across consecutive notices.
- [pinned] Notice digest (`AdvanceNotice::digest`, `notice::notice_digest`):
  lowercase hexadecimal SHA-256 over a sequence of frames, each an 8-byte
  unsigned big-endian length followed by that many bytes: first the domain
  `ck.compaction.advanced.v1`, then, for each header of the table the notice
  carries, in ascending byte order of its name, the name's frame followed by
  the value's frame.
- [pinned] Content digest (`digest::content_digest`): lowercase
  hexadecimal SHA-256 over the RFC 8785 (JCS) encoding of a message, or a
  message part, as `session.read` returns it. `through_digest` is the content
  digest of the message at `through_ordinal`. The provider computes part
  digests itself, over the parts the runner's tags address.

### 5.1 Fork descent

- [pinned] A notice whose `lineage_id` differs from the lineage the runner
  last announced for the session may carry `descends_from`
  (`notice::NoticeDescent`): `<lineage_id>:<through_ordinal>:<through_digest>`
  naming the old lineage, the last ordinal whose entry is byte-identical in
  both lineages, and that entry's content digest, defined as for
  `through_digest`. `llm-runner/v1` pins no grammar for lineage ids, so a
  lineage id may contain `:`; the value is parsed from the right (the digest
  after the last `:`, the ordinal before it, the old lineage id before that).
  A lineage id announced in a notice is 1 to 256 bytes of printable ASCII
  without space.
- [pinned] The runner claims only entries that are byte-identical in both
  lineages. A wrong claim is a runner defect, and the provider's digest
  check below catches it.
- [pinned] When a notice moves the session to a new lineage, the provider
  keeps state derived from entries through the declared `through_ordinal`
  (tags, queues, folds, coverage) only if `descends_from` names the lineage
  it holds and the content digest it recorded for that lineage at that
  ordinal equals the declared digest (`notice::descent_outcome`). It then
  reads the new lineage from the ordinal after the cut. In every other case
  (no `descends_from`, another old lineage, an ordinal it never recorded, a
  different digest) it resyncs as below. The header is an optimisation,
  never a requirement: a runner that omits it gets the full resync.

### 5.2 What the provider does with a notice

- [pinned] A notice only wakes the provider. The provider then reads every
  message after its own cursor over its `llm-runner/v1` route with
  `session.read` (range reads from the ordinal after its cursor, following
  `next_from_ordinal`), with each message's per-part tags. That read is
  ordered and replayable, so a lost or duplicate notice loses no content.
  The cursor is the provider's own, kept durably per session, and advances
  only once the messages it covers are durable in the provider's store.
- [pinned] A notice with a `seq` at or below the highest the provider has
  processed is a duplicate: it is a wake and nothing more. A gap in `seq`
  loses no content and no acknowledgement (`applied_view_version` repeats).
  A gap may hide events. A hidden revert shows up in the read as one of the
  resync triggers listed next. A hidden refusal or discard shows up as
  `applied_view_version` never naming that view. A hidden marker outcome is
  taken as `unknown`.
- [pinned] The provider computes the content digest of every message it
  reads. **Resync** is required when any of these happens:
  - `through_digest` differs from the digest the provider computes for
    `through_ordinal` once it has read that far;
  - a re-read returns different bytes for an ordinal or a tag it already
    holds;
  - the session moves to a new lineage without a fork descent the provider
    accepts (§5.1);
  - the transcript ends below an ordinal the provider holds.
- [pinned] Resync, in order:
  1. The provider publishes no `view_ready` for the session and answers
     every `compaction.ride` with `nothing` until step 4. Views it already
     published stay immutable and `view.get` keeps serving them.
  2. It discards its held copy of the session's messages, its per-part
     index, and every unpublished view built from them. It resets its cursor
     to none. Derived state (summaries, say) may be kept only if its sources
     re-validate by content digest after the re-read, as §5.3 states for
     work anchored on message ids and part keys.
  3. It re-reads the whole current lineage from its first message, through
     at least the newest `through_ordinal` it has seen.
  4. It publishes a new view, at a higher version, built only from re-read
     content.
  If the re-read itself returns different bytes for an ordinal already
  re-read in the same resync, the runner's reads are unstable. The provider
  then publishes no views for the session until a later notice's
  `through_digest` matches its read. Meanwhile the runner keeps rendering its
  current view, and the ceiling still applies.
- [pinned] The provider never repairs or rewrites the runner's transcript.
  Tags are the runner's, so nothing needs repair after an outage.

### 5.3 Runner state lost

- [pinned] A runner that loses or scrubs its own tag or view state (for
  example after restoring an older store) MUST continue the session under a
  new `lineage_id`, announced on its next notice without `descends_from`. It
  claims no truncation: `revert_below` means truncation only, and the runner
  does not send it for lost state. Having lost its view, it renders Setup's
  initial CompactionMessage and omits `applied_view_version` until a newer
  view applies (§7).
- [pinned] Tag numbers after the loss follow three rules. Within a session
  the runner MUST NOT give a tag number to a different part than the one it
  had before. New tag assignments go above the floor: every tag number the
  session may have used before the loss, as for a fork in `llm-runner/v1`
  (`tags::next_after`). A runner whose restored state cannot tell it the
  highest number it handed out jumps past any number it could have used. A
  part whose restored state still records its tag MAY keep that number.
- [pinned] A provider never relies on tag numbers across lineages. It
  re-anchors by message id, part key and content digest.
- [pinned] A provider that sees a new lineage without descent drops its
  runner-side copy of the session: its tag index, its cursor, and its record
  of which view the runner renders. It then re-reads the whole new lineage
  with `session.read` (the resync of §5.2). View versions still only rise
  (§6): the next view carries a version above every version the provider has
  published.
- [pinned] The provider may keep derived work anchored on a message id, or
  on a message id plus part key, when the re-read transcript still has that
  message or part with a matching content digest. Summaries, memories and
  queued drops are examples. A queued drop is re-addressed by the part's new
  tag, never by its old tag number. Work whose message or part is gone, or
  whose digest differs, is discarded. Tag numbers never carry across
  lineages.

## 6. Views: provider to runner

- [pinned] A view (`view::CompactionView`) is `{session_ref, view_version,
  lineage_id, covers_through: {ordinal, mid}, ranges, cut?, digest}`. It is
  the provider's preferred shape for the prefix of the transcript through
  message `covers_through`. Every message after it goes out unchanged. A view
  never replaces or rewrites the runner's transcript or the records in its
  write-ahead log, so a session can later switch to another compaction
  provider, or to none, with its history intact.
- [pinned] `ranges` (`view::ViewRange`) are half-open over ordinals of
  `lineage_id`, `from` included and `to` excluded, the convention of a
  CompactionMessage (§4). Together they tile `[0, covers_through.ordinal +
  1)` in order, with no gap and no overlap:
  - `{kind: "keep", from, to, edits?}` sends the transcript's messages in the
    range as written, except for the parts `edits` changes. Never empty.
  - `edits: [{ordinal, mid, part, bytes?, drop?}]` (`view::PartEdit`) edits
    single parts of messages that otherwise stay: a dropped tool output, a
    stripped image, compressed text. So a view never re-sends a whole message
    to change one part. `part` is `llm-runner/v1`'s opaque part key, the same
    key as its `tags: [{part, tag}]` rows and `session.resolve`; it is 1 to
    256 bytes, compared byte for byte. Each edit gives exactly one of `bytes`,
    a string holding the replacement part as UTF-8 JSON in the runner's
    message schema, or `drop: true`, which removes the part outright (`drop`
    is omitted when false). Edits are sorted by `ordinal` and, within one
    message, by the part's position in that message; there is at most one
    edit per part; every `ordinal` lies inside its keep range; and every
    `mid` is the id of the message the runner holds at that ordinal, or the
    view is discarded as not a prefix (§7). `edits` is omitted when
    empty.
  - `{kind: "replace", from, to, bytes}` sends `bytes` in their place.
    `bytes` is a string holding a UTF-8 JSON array of messages in the
    runner's message schema: a history head or a fold of whole messages. It
    is carried as a string so its exact bytes enter the digest. `from == to`
    inserts the messages before `to` and replaces nothing.
  `kind` is strict: a runner cannot apply a range it does not understand.
- [pinned] **Replaced bytes are final.** A replace range's `bytes` and an
  edit's `bytes` are sent exactly as given. When the run renders tags
  (`render_tags`), the runner renders them only on transcript parts it sends
  as written, and adds nothing inside replaced bytes. A drop placeholder that
  should carry a tag carries it in its own bytes, written in `llm-runner/v1`'s
  pinned tag render format (its exact prefix and separator, and the part
  kinds that carry a tag, pinned in `cortexkit-role-llm-runner` 0.3.0), so a
  provider produces the same bytes the runner would.
- [pinned] Optional `cut: {cut_id, through_ordinal}` (`view::ViewCut`) names
  the point through which the provider's replacements stand for the whole
  transcript. It lies on a range boundary within the coverage, with only
  replace ranges before it (§9).
- [pinned] `view_version` is a `u64` from 1, raised for every view of the
  session, including ride views. It is never reused across a provider
  restart: the provider resumes above both the highest version it published
  and the highest `applied_view_version` it has seen. The provider never
  publishes two contents under one version.
- [pinned] View digest (`CompactionView::compute_digest`): lowercase
  hexadecimal SHA-256 over frames as in §5: the domain
  `ck.compaction.view.v1`; `session_ref`; `view_version` in decimal;
  `lineage_id`; `covers_through.ordinal` in decimal; `covers_through.mid`;
  the number of ranges in decimal; for each range in order, its `kind`,
  `from` and `to` in decimal and, for `replace`, its `bytes`, or, for
  `keep`, the number of its edits in decimal followed, for each edit in
  order, by its `ordinal` in decimal, its `mid`, its `part`, and either
  `bytes` and the edit's bytes or `drop`; then, only when a cut is present,
  `cut`, its `cut_id` and its `through_ordinal` in decimal. `digest` itself
  is not an input.
- [pinned] A view is well formed (`CompactionView::check`) when, checked in
  this order: `session_ref` is a session reference (`session_ref`);
  `view_version` is not 0 (`version_zero`); `digest` is lowercase hexadecimal
  (`digest_malformed`); `ranges` is not empty (`no_ranges`); each range starts
  where the previous ended, the first at 0 (`gap`); no range is inverted and
  no keep range is empty (`inverted`); the last range ends at
  `covers_through.ordinal + 1` (`coverage_end`); every replace range's bytes
  are a JSON array (`replacement_not_array`); every keep range's edits lie
  inside it (`edit_outside_range`), are sorted by ordinal (`edit_order`), name
  each part once (`duplicate_edit`), have a valid part key (`edit_part`),
  give exactly one of `bytes` and `drop` (`edit_action`), and give JSON bytes
  (`edit_not_json`); the cut is valid (`cut`); the
  compact JSON encoding is at most 3,145,728 bytes (`MAX_VIEW_BYTES`,
  `too_large`); and `digest` matches the content (`digest_mismatch`).
- [pinned] When a view is durable in its own store, the provider publishes
  `ck.{acct}.event.<provider_module_id>.compaction.view_ready.v1`
  (`notice::ViewReady`, `notice::VIEW_READY_EVENT`) with headers
  `session_ref`, `view_version`, `covers_through` (the ordinal) and `digest`
  (the view digest). The bus id is
  `<session_ref>:view_ready:<view_version>` (`ViewReady::message_id`), which
  names its event for the reason in §1, and the bus digest is the view
  digest, so the event is a claim check for exactly one immutable view. It
  publishes one for every view, including ride views.
- [pinned] `view.get {session_ref, view_version}` (`view::ViewGetRequest`)
  is served by the provider on the compaction route. Its answer
  (`view::ViewGetAnswer`, strict on `answer`) is `{answer: "view", view}`,
  byte for byte the view first published under that version, or
  `{answer: "refuse", code, reason}` with `code` `unknown_session` or
  `unknown_version` (`view::view_get_codes`). The provider never answers a
  different version. It keeps at least its newest published view for each
  session, plus every view newer than the latest `applied_view_version` it
  has seen. A runner treats an unknown refusal code like `unknown_version`:
  it keeps its current view and waits for the next `view_ready`.
- [pinned] `ck-bus`'s module-event consumer table (`cortexkit-bus-naming`)
  must admit the provider as a reader of the runner's events and the runner
  as a reader of the provider's. This role does not change that table.

## 7. Applying a view: the runner's side

- [pinned] On a `view_ready` whose version is above the view it renders,
  the runner fetches that view with `view.get` and decides in a fixed order
  (`view::dispose_fetched`):
  1. the view names the announced `session_ref` and version, or it is
     `rejected:not_requested`;
  2. it is well formed (§6), including its own digest, or it is
     `rejected:<problem>`;
  3. its digest equals the announced digest, or it is
     `rejected:digest_mismatch`;
  4. its version is above the one the runner renders, or it is `stale` and
     ignored;
  5. its coverage is a prefix of the transcript: same `lineage_id`, the
     runner's message at `covers_through.ordinal` has id
     `covers_through.mid`, and the message at each edit's `ordinal` has the
     edit's `mid`. If not, it is `discard`ed;
  6. every edited part key exists in its message, or the view is
     `refused:unknown_part` and the runner reports
     `view_refused:<version>:unknown_part`;
  7. each message's edits follow its part order, or it is
     `rejected:edit_order`.
  Only then does it `apply`. A rejected view is not persisted, not applied
  and not reported; the runner keeps its current view and may fetch again.
- [pinned] **A view applies only at a step boundary.** The runner writes it
  durably, as its own record in its write-ahead log, before the step that
  first renders it. So a resumed run renders byte for byte what it rendered
  before. A view that arrives mid-step waits for the next boundary. From that
  boundary on, the runner renders it for every request until a newer view
  applies, including across runner restarts and while the provider is down.
  Every later notice names it in `applied_view_version`.
- [pinned] Under a view, a request's history is, for each range in order,
  the transcript's messages inside a keep range, with their part edits
  applied (a replaced part becomes its bytes, a dropped part is left out,
  every other part is sent as written), or a replace range's messages,
  followed by every message after the coverage (`CompactionView::assemble`
  is the reference, over a model in which a message's parts are the elements
  of its `content` array). Messages a runner carries
  outside the transcript (reminders, appends) are the runner's to place.
- [pinned] Model provider quirks are **the runner's** duty. The view is the
  provider's preference, and the provider carries no model-provider quirks
  for runners. When the current model provider's rules forbid
  applying a view as it stands, the runner has two options:
  - **Defer:** hold the view until a step boundary where applying it is safe
    for the model provider, keep rendering its current view, and report
    `view_deferred:<version>:<reason>`. `applied_view_version` names the view
    only once it applies.
  - **Companion edits:** apply it together with only the edits the model
    provider requires and nothing else, and report
    `view_companion:<version>:<edit>` alongside `applied_view_version` naming
    the view. Today the one such edit is `strip_signed_reasoning`: removing
    the signed reasoning produced after the view's first changed byte, which
    a model that binds signed reasoning to its prefix requires.
  A runner that can do neither refuses the view with
  `view_refused:<version>:provider_constraint`.
- [pinned] A deferral lasts no later than the next step that starts from a
  user message: at that step the runner applies the view (with companion
  edits if the model provider requires them) or refuses it with
  `view_refused:<version>:provider_constraint`; it never keeps deferring.
- [pinned] The runner also refuses a view, reporting
  `view_refused:<version>:<reason>` on its next notice and keeping its
  current view, when applying it would separate a tool call from its result
  (`tool_pair_split`), including through a dropped part, or break role order
  for the current model provider (`role_order`), and when an edit names a
  part key the message does not have (`unknown_part`, step 6 above).
- [pinned] A view whose coverage is no longer a prefix of the transcript,
  because a revert truncated below `covers_through`, is discarded, whether
  just fetched or currently applied. The runner reports
  `view_discarded:<version>` with `revert_below` on its next notice. With
  its current view discarded, it renders Setup's initial CompactionMessage
  and omits `applied_view_version` until a newer view applies.

## 8. Ride request and ceiling

- [pinned] `compaction.ride {session_ref, cause, deadline_ms, overflow?}`
  (`ride::RideRequest`) is a synchronous call on the compaction route. The
  runner sends it only when its prefix is being rewritten anyway (`cause`
  `ttl_expired`, `model_change` or `surface_change`), or when a request
  reached the ceiling (`cause` `ceiling`, below). `cause` decodes open
  (`RideCause::Unknown`); a provider answers `nothing` to a cause it does not
  know. `deadline_ms` is how long the runner waits for the answer, counted
  from sending, and is above 0. For the three rewrite causes it is the
  runner's own short deadline, at most 10,000
  (`ride::MAX_REWRITE_RIDE_DEADLINE_MS`), because the ride sits in front of a
  request the user is waiting for. Only `ceiling` uses the ceiling policy's
  `wait_ms`: its `deadline_ms` is the time left of that wait, at most 300,000
  (`ceiling::MAX_CEILING_WAIT_MS`). `RideRequest::check` refuses a deadline
  out of range (`deadline_out_of_range`).
- [pinned] Optional `overflow: {reported_limit_tokens?}` (`ride::Overflow`)
  is allowed only with `cause: "ceiling"`. Present, it says the ride follows a
  model provider error stating that the request exceeded the model's context
  window. `reported_limit_tokens` (`u64`) is the limit that error stated, set
  only when the runner parsed one from it and never guessed or filled from
  the model catalog; `overflow: {}` is valid. Absent, the ceiling came from
  the runner's own count, and a provider never reads absence as "no overflow
  happened". On any other cause a provider ignores `overflow`; it is not a
  refusal, and `RideRequest::check` does not fail on it.
  `RideRequest::ceiling_overflow` returns it for a `ceiling` ride only.
  `overflow` and `reported_limit_tokens` are omitted when absent, never
  encoded as `null`.
- [pinned] A rewrite-cause ride never ends a run: whatever its outcome, the
  runner sends the request, under the ride's view or its current one. Only
  a `ceiling` ride can end in the run being refused (`context_ceiling`,
  below).
- [pinned] The answer (`ride::RideAnswer`, strict on `answer`) is
  `{answer: "view", view}` or `{answer: "nothing"}`. A view is checked as in
  §7, against its own digest, and applies at this step: the ride happens
  at the step boundary before the step. A missed deadline, an `ERROR`, or an
  answer that does not decode is treated as `nothing`, and the runner
  proceeds with its current view. A late answer is dropped. The provider
  also publishes `view_ready` for every ride view, so a dropped one can still
  apply at a later boundary.
- [pinned] Setup's `ready` answer carries optional `ceiling`
  (`ceiling::CeilingPolicy`): `{at_share, on_overflow?, wait_ms}`. It is
  frozen with the session (§12). The runner pauses
  when its estimate of the next request reaches `at_share` of the window
  (`CeilingPolicy::reached`: `size >= at_share * window`), or, when
  `on_overflow` is true, when the model provider refused the request as too
  large. `at_share` is above 0 and at most 1. `wait_ms` is above 0 and at most
  300,000. `on_overflow` is omitted when false (`CeilingPolicy::check`,
  `share_out_of_range`, `wait_out_of_range`).
- [pinned] At the ceiling the runner sends `compaction.ride` with `cause:
  "ceiling"` and `deadline_ms` set to the time left of `wait_ms`, carrying
  `overflow` when the pause follows a model provider's overflow error. If that call
  brings no view that fits, it keeps waiting for a `view_ready` until
  `wait_ms` has passed in total. Then it decides (`CeilingPolicy::decide`),
  in order:
  1. a request larger than the window is refused `over_window`: the runner
     never sends over the window;
  2. a request that no longer reaches the ceiling is sent. A pause caused by
     an overflow error resends only under a newer view than the one that
     overflowed;
  3. anything else is refused `provider_unavailable` when the provider
     could not be reached, and `view_timeout` otherwise.
  A refusal ends the run with `llm-runner/v1`'s `context_ceiling` code
  (`ceiling::CONTEXT_CEILING`, added in `cortexkit-role-llm-runner`
  0.3.0) and detail `{window, size, cause}`, with `cause` from
  `ceiling::context_ceiling_causes`.
- [pinned] Without `ceiling` the runner never pauses for the provider, and
  still never sends over the window: such a request is refused
  `context_ceiling` with `cause: "over_window"`.

## 9. Native compaction marker

- [pinned] Once a view with a `cut` has applied (§7), the runner writes
  its host's native compaction marker at the cut (for example OpenCode's
  compaction message or Pi's compaction entry). On its next notice it
  reports `marker_outcome:<applied|deferred|unknown>:<cut_id>`, alongside
  `applied_view_version` naming that view. `deferred` promises a later notice
  with the final outcome. A runner whose host has no native marker reports
  nothing.
- [pinned] The marker is a host record. It never changes what the runner
  sends while a view applies, and never rewrites the runner's transcript.

## 10. REFUSE

- [pinned] A Setup `refuse {code, reason, provider_code?}` ends the run
  `error`. The runner records the role `code` as the refusal's code, with
  the provider and `reason`, and, when present, the provider's
  `provider_code` as a separate member beside it, never in place of `code`.
  Nothing is added to the model's history, and the session stays usable: the
  next send calls Setup again.
  In llm-runner run errors, this answer's role `code` maps to `provider_code`
  and its optional finer `provider_code` maps to `provider_detail_code`,
  diagnostics only, never replacing the role code or deciding retryability.
- [pinned] `code` is one of the four typed codes below (`RefuseCode`). Each
  has a fixed retryability, so an owner knows whether retrying without acting
  can help: a busy provider or unreadable history may clear, while a window or
  configuration refusal needs correction. The table also lists the named wire
  code `setup_identity_mismatch`, which is not a typed variant in 0.3.x.
  The answer carries no retryability of its own: a `retryable` member is ignored
  on decode and never decides retry.
- [pinned] `provider_code` is an optional open string: the provider's own,
  finer reason, for logs and display. It never replaces `code` and never
  decides retry.
- [pinned] A `code` the runner does not know decodes rather than failing
  the answer (`RefuseCode::Unknown`). It still ends the run, is recorded as
  received, and is not retryable.
- [pinned] `setup_identity_mismatch` is a named wire refusal, but in 0.3.x it
  decodes as `RefuseCode::Unknown`. The runner therefore ends the current run
  as `error` after Setup and does not retry the refusal; the next send calls
  Setup again. The next breaking release will make it a typed variant and
  mark both public enums `#[non_exhaustive]`.
- [pinned] `reason` is user-facing text.

| Code | When | Retryable |
|---|---|---|
| `window_too_small` | the model's window cannot hold the smallest view the provider can build | no |
| `provider_busy` | the provider is busy with the session's history and cannot answer Setup now | yes |
| `misconfigured` | the configuration for this preset and params cannot be used | no |
| `history_unreadable` | required history is missing, such as a declared inherited prefix the provider does not hold | yes |
| `setup_identity_mismatch` | the incoming route is bound to one non-empty session that differs from Setup's session; in 0.3.x decoded as `Unknown`, ending the current run as `error` | no |

## 11. The model view

The model view is `llm-runner/v1`'s; this section only says where this
role's ids show up in it while the runner renders Setup's `initial`. How a
view (§6) appears in the model view is the runner role's to define.

- [pinned] A runner that declares `model_view` answers `session.read`
  with `view: "model"` as `ModelPage {messages: [ModelEntry {source,
  message}], lineage_id?, next_from_ordinal?, head?, compaction_id?,
  version?}`. Each replacement message has `source: {kind: "replacement",
  compaction_id, version, from_ordinal, to_ordinal}` (`EntrySource`),
  and each raw message `source: {kind: "message", ordinal, mid}`. This crate
  imports `EntrySource` from the runner's crate and defines no source kind
  of its own.
- [pinned] For a CompactionMessage with range `{from, to}`, each
  replacement message's source is `{kind: "replacement", compaction_id,
  version, from_ordinal: from, to_ordinal: to}`, the same half-open range
  (`CompactionMessage::model_view_source`, which builds the runner crate's
  `EntrySource`). A malformed CompactionMessage has no source.
- [pinned] An empty range is an insertion: `from_ordinal == to_ordinal`,
  placed before that ordinal and covering no transcript message. Setup's
  usual head is `[0, 0)`. There is no separate source kind for inserted
  messages.

## 12. Purity and identity

- [pinned] What the runner freezes from Setup, `ceiling`, derives only from
  the preset, the params, the provider's configuration and the composition.
  The same inputs give the same bytes.
- [pinned] No request in this role carries scope, agent or session identity
  beyond the opaque `session` handle, the caller's `harness` and the session
  reference derived from them (§1). A provider uses them only as keys for its
  own per-session state, never to choose a variant. A subagent session gets
  different behaviour through its preset or params, never through who is
  asking.
- [pinned] Replacement and edit bytes are history content, not fetched
  text: they may depend on the session's own messages and on the provider's
  state for the session (its memories and summaries). Project-level content
  such as project docs goes into user-role messages, never into system text.

## 13. Error codes

Codes ride as the `code` of an `ERROR` frame's body `{code, message,
detail?}` (`ErrorBody`). They are open strings: a party that meets one it
does not know treats it as a terminal refusal of that one request, and
never retries it.

`KnownCode` classifies the typed codes this role recognizes from §10 and §13,
with their fixed retryability (§13.3); it does not close the wire vocabulary.
A named wire refusal not represented there follows `RefuseCode::Unknown` (§10).
`ErrorBody.code` remains a string; an unrecognized Setup `refuse` code decodes
to `RefuseCode::Unknown`. `detail` is arbitrary JSON, omitted when absent;
`message` is a required string.

### 13.1 Provider refusals (`errors`)

| Code | When | Detail | Retry |
|---|---|---|---|
| `invalid_params` | a request field is malformed, or names a preset or params value the provider does not know | `field` | no |
| `transient` | a condition that may clear by itself | — | **yes**, within the call's budget |

- [pinned] The malformed-request code is `invalid_params`, as runners
  answer. `tool-provider/v1` spells it `invalid_request`; a consumer of
  both maps each by its own role.
- [pinned] An `ERROR` refusal of `compaction.ride` counts as `nothing`
  (§8). An `ERROR` refusal of `view.get` leaves the runner on its current
  view, as an unknown `view.get` refusal code does (§6).

### 13.2 Codes the runner answers or writes (`errors::runner_codes`)

| Code | When |
|---|---|
| `invalid_params {field: "plan.compaction_item"}` | admission, on a runner without the `compaction` group |
| `compaction_unavailable` | a run's `provider_code`, after Setup only: Setup failed or timed out with no answer, so no initial view exists and no model call is made (§4) |

The ceiling refusal is `llm-runner/v1`'s `context_ceiling` (§8); that role
defines its retryability.

### 13.3 Retryability and whether a code ends a run after Setup

`KnownCode` classifies the typed codes this role recognizes from §10 and §13.
This table also pins open wire codes that do not yet have a typed variant.
`setup_identity_mismatch` follows §10's `RefuseCode::Unknown` path in 0.3.x:
it is not retryable and ends the current run after Setup. No party chooses
retryability or terminal behavior per message.

| Code | Retryable | Ends a run after Setup |
|---|---|---|
| `invalid_params` | no | no |
| `transient` | yes | no |
| `window_too_small` | no | yes |
| `provider_busy` | yes | yes |
| `misconfigured` | no | yes |
| `history_unreadable` | yes | yes |
| `setup_identity_mismatch` | no | yes |
| `compaction_unavailable` | no | yes |

## 14. Decoding

- [pinned] **Lenient** (unknown fields ignored): `role.describe`, the Setup
  request, the fields of every answer, the fields of `view.get` and
  `compaction.ride` requests and of their `overflow`, of a view, its cut and
  its part edits, and of
  `ceiling`. Unknown notice and view-ready headers are ignored. A part
  edit's `ordinal`, `mid` and `part` are required.
- [pinned] **Strict on values**: `answer` in every answer, because a runner
  cannot act on an answer it does not understand, and each view range's
  `kind`. Values a runner may grow are decoded open: ride `cause`, unknown
  event tokens, the reasons and edits in `view_refused`, `view_deferred` and
  `view_companion`, `view.get` refusal codes, and a refusal's
  `provider_code`. So is a refusal's `code`, whose unknown values are never
  retried (§10).
- [pinned] Versions and ordinals are `u64`, as `llm-runner/v1` types them.
- [pinned] Public types with optional members are non-exhaustive where they
  are likely to grow, built with a constructor and `with_*` setters.
- [pinned] Setup's `descends_from` and the answer's `ceiling` default to
  absent and are omitted when absent. A present `descends_from` requires
  string `lineage_id` and `u64` `through_ordinal`.

## 15. Crash and durability guarantees

What a provider owes across its own crash, stated as properties the suite
checks (§16) by killing the provider at each point below and restarting it
on the same state root (`cortexkit-role-harness`). The runner's side is
`llm-runner/v1`'s.

1. [pinned] **View versions only rise.** No view version is used for two
   different contents, and every view after a restart carries a version
   above both the highest it published and the highest
   `applied_view_version` it has seen.
2. [pinned] **Messages are taken in once.** Messages the provider reads
   again after a kill are recognised by ordinal and lineage, never counted
   twice.
3. [pinned] **Setup is repeatable.** A Setup repeated for a session the
   provider recorded gets a well-formed answer.
4. [pinned] **No identity leaks into frozen bytes.** Setup's `ceiling` for
   the same inputs is byte-identical before and after a kill.

Kill points (`points.rs`). [pinned] The list below.

| Point | Durable state after the kill | Property |
|---|---|---|
| `SetupRecorded` | the provider's per-session Setup state; the answer not sent | 3, 4 |
| `MessagesIngested` | messages read with `session.read` in the provider's store; its cursor not moved past them | 2 |
| `ViewRecorded` | a view in the provider's store under its version; its `view_ready` not published | 1 |

A provider whose state lives in a store where truncation is not meaningful
(SQLite, say) reaches these points with a fault hook followed by a real
process kill, and declares that. A suite run fails unless at least one kill
was a real process kill.

## 16. Conformance suite (planned)

A separate `cortexkit-role-compaction-provider-conformance` crate, run by
each provider in its own CI against its real module over real routes, never
a double. It drives the provider with a scripted runner that publishes
advance notices, serves `session.read` from a scripted transcript, fetches
views and sends rides. It will check:

| Case | Checks |
|---|---|
| `role_describe_shape`, `role_describe_cacheable`, `extra_op_still_admitted` | §2 |
| `setup_once_initial_well_formed`, `setup_repeat_after_failure` | §4 |
| `setup_declarations_pure` | §12: two Setups with the same inputs and different `session` give the same `ceiling` |
| `setup_descent_parent_unknown`, `setup_descent_cut_not_held`, `setup_descent_prefix_gap` | §4: refuse missing retained history without saving a link |
| `setup_descent_conflict`, `setup_descent_repeat` | §4: fixed durable link, idempotent repeat |
| `notice_wakes_read`, `duplicate_notice_is_a_wake`, `seq_gap_loses_no_content` | §5.2: the provider reads with `session.read` from its own cursor |
| `resync_on_changed_bytes` | §5.2: a changed digest triggers the resync steps, and no view is published until the re-read completes |
| `fork_descent_kept`, `fork_descent_digest_mismatch_resyncs`, `fork_without_descent_resyncs` | §5.1 |
| `view_immutable`, `view_version_rises`, `view_get_never_another_version` | §6 |
| `part_edits_well_formed` | §6: a provider's part edits are sorted, one per part, inside their keep range, and name part keys the runner's `session.read` returned |
| `ride_answers_within_deadline`, `ride_unknown_cause_nothing`, `ride_overflow_ignored_off_ceiling` | §8 |
| `lineage_reset_drops_runner_copy` | §5.3: a new lineage without descent drops the tag index, cursor and rendered-view record, re-reads, and keeps only derived work whose message or part re-validates by digest |
| `crash_at_<point>` for each point in §15 | §15 |

Runner-side rules no live provider can exercise (a fetched view's
disposition, assembly under a view, the fork-descent decision, the ceiling
decision, the model-view source of `initial`) are tested against the vectors
in this crate.

Verdict, as for `tool-provider/v1`: a case whose requirements the subject
does not declare is skipped, never passed; a run fails if any case fails or
if no kill ended a real process.

## Not in v1

- A neutral, role-defined message schema. Messages read with
  `session.read`, replacement bytes and edit bytes are in the runner's
  schema (`llm-runner/v1` §4.2); a provider serving more than one runner
  decodes each runner's schema.
- More than one compaction provider per session, or a chain of them.
- A synchronous per-step call. Earlier drafts of this role had
  `compaction.step` with a request fence, `wait` and `compaction.ready`; this
  revision removed them.

## Differences from `llm-runner/v1` wording

The `llm-runner/v1` contract (`cortexkit-role-llm-runner/CONTRACT.md`)
states the runner's side of this interface. These passages there are out of
date against this contract and should be changed to match it:

- **The `compaction` group.** `llm-runner/v1` defines the group as serving
  `compaction.ready` and calling the provider on each step. Here the group is
  the runner's side of §3: Setup, advance notices, view application, rides
  and the ceiling. `compaction.ready` no longer exists.
- **The plan field naming the provider.** `llm-runner/v1` marks the
  `compaction_item` provider field provisional. The field is settled: the
  plan names the session's compaction provider at
  `plan.compaction_item.provider`, inside `compaction_item: {provider,
  preset, params}` (§3). The runner crate's `plan_compaction_provider`
  helper already reads it there.
- **When `compaction_unavailable` is written.** It is written after Setup
  only (§4).
- **What a refusal records.** A `refuse` carries a role `code` and,
  optionally, the provider's own finer `provider_code` (§10). The runner
  contract should name one member for the role code and a separate one for
  the provider's code, and never let the second replace the first.

## Interoperability

Versions and ordinals are unsigned 64-bit integers, not bounded to the
exact-integer range of JSON readers that use doubles. Such readers must
preserve integer precision rather than round ids, cursors or versions.
