# Changelog

## 0.7.9

- Add the optional `history_edit_cost` notice header for suffix-only or
  full-input edit pricing. Unknown values keep their wire spelling but read
  as no hint; absence keeps existing pricing. Deploy readers first: every
  provider that receives a runner's notices must pin this release (or a later
  one defining the header) before that runner sends it, because older readers
  leave it out of notice digests.
- Add the `chain_reset` rewrite ride for full input already known to be
  needed before rendering, never for send-time discoveries. It uses the
  normal rewrite wait and answers, without requiring the `cache_ttl` refresh
  that `model_change` requires. Older providers read it as an unknown cause,
  and a rewrite ride never ends a run.
- Regenerate async vectors for both pricing modes, absent and unknown hints,
  their different digests, and chain-reset rides. The notice-size bound for
  the defined headers rises from 2,391 to 3,432 bytes, still under the
  4,096-byte cap.

## 0.7.8

- Extend the shared Setup checkpoint with optional paired `message_count`
  and certified `head_cut`, sent only to `message_checkpoint` runners.
  Document the permanent moved-session checkpoint, counting and timestamp
  units, adoption head equality checks, and separate message and tag anchors.
- Add terminal Setup refusals `message_checkpoint_unsupported`,
  `moved_session_reverted_below_import` and `moved_session_head_mismatch`,
  including typed codes, retryability tables and vectors. Setup answers
  remain lenient for older decoders. Depend on runner 0.7.17.
- Pin reverse `session.head` during an outstanding Setup for
  `message_checkpoint` runners: recover the head from a complete marker key
  or earlier stored Setup checkpoint, without waiting on Setup or its gates.
- Pin the same Setup-independent access for `session.read` at an adoption
  cut, serving retained rows from `first_ordinal` and refusing earlier rows.
  This lets the provider build its Setup answer after adoption-cut recovery;
  native-cut descent continues to use the provider's held parent-lineage rows.

## 0.7.7

- The `held_copies_follow_reasoning_pruned` probe's cut conversation
  starts its second lineage only at a cut the provider published. The probe
  used to cut on its own after ordinal 1 and then send a Setup descending
  from that cut. The provider never published it, so a conforming provider
  refuses `unknown_cut` (CONTRACT.md §4) and no conforming provider could
  pass. Now the probe asks the provider for a view with a cut through
  ordinal 1 through the new optional `PrunedReasoningProbe::publish_cut`.
  It checks that view as a runner checks a fetched view
  (`view::dispose_fetched`), starts the new lineage after the view's own
  `cut.through_ordinal`, and only then sends the descending Setup and
  notice naming that cut. A later cut is applied where it stands; a view
  without a cut, one a runner would not apply, or a cut below ordinal 1
  fails by name. The cut conversation runs on every serializer profile's
  transcript, as the conversation without a cut does.
- An adapter without `publish_cut` (the default returns `None`) cannot have
  the provider publish a cut, and the probe never cuts on its own. For it
  the cut conversation is not run. `HeldCopiesReport` gains
  `conversations_checked` and `conversations_not_applicable`, which names
  `cut` with the reason. Not run is not a pass.
- A Setup the provider refuses fails the probe with the refusal: the Setup
  it answered (its lineage and any `descends_from`), the refuse code (for
  example `unknown_cut`), any `provider_code`, and the reason. It used to
  report any refusal as "the provider's Setup answer does not accept
  pruned_reasoning". A ready answer without `pruned_reasoning` in
  `accepts` still fails with that message, now naming the Setup. An
  adapter error while delivering a Setup or a notice, or while building a
  view, now names that step.
- Kit tests guard these. A provider that refuses a Setup descending from
  a cut it never published passes on every transcript. One that refuses
  `unknown_cut` for a cut it did publish fails by name. A recording
  provider shows that every descending Setup names a cut a published view
  carried; that test failed on the 0.7.6 probe.

## 0.7.6

- The `held_copies_follow_reasoning_pruned` probe's no-second-reconcile
  check fails only on a read of an ordinal below the provider's newest held
  ordinal. A provider that reads from its newest held message onward on
  every notice, a one-entry overlap confirming the transcript still
  continues from what it holds, no longer fails as "re-read ordinal 4
  again" when the reconciled `through` is the newest message. A second
  reconcile, which re-reads held ordinals from the cut onward, still fails
  with the same message.

## 0.7.5

- The `held_copies_follow_reasoning_pruned` probe's profile transcripts
  are ones a real runner could serve, so a provider that strictly decodes
  the stored formats reaches the prune. On `opencode-aisdk` and `pi`, every
  `mid` the runner serves (page entries, `pruned_reasoning` rows, the head's
  `cut`) is the host's own message id, `info.id` or the journal entry `id`,
  as `llm-runner/v1` pins for a host-served entry. Every notice now ends on
  a complete tool call and result (through ordinal 4, and 6 after the cut)
  rather than on an `anthropic-messages`, `owned-broca` or `pi` answer whose
  tool call had no result yet, and the prune covers the same range; what
  the probe checks is unchanged.
- The probe's `owned-broca` transcript serves the shapes a runner's
  `session.read` serves. A stored text-with-state part is served as plain
  `text`, because its provider state never leaves the runner. A tool
  result's `output` is an object tagged on `kind` with `is_error` beside it,
  not an array, and the encrypted reasoning item names its real
  `provider_format`. So on `owned-broca` the report names the
  `text_with_state` part as not applicable. The `signature` kind stays
  checked through the tool call's `thought_signature`, which a page serves
  as stored.
- A kit test guards these: host mids, notices ending on complete tool
  calls, and the `owned-broca` served shapes. It failed on the 0.7.4
  fixtures.

## 0.7.4

- The probe's `owned-broca` transcript uses the real
  `provider_state {format, data}`. It alternates the signed formats
  `gemini` and `google_code_assist` and carries a Responses item state that
  no prune touches. Its signed `reasoning` part gives up only its signature
  under `opaque_reasoning`, and so does an `anthropic-messages` `thinking`
  block. The generic transcript's `text_with_state` also moves to the
  `gemini` shape, because llm-runner 0.7.11 corrected the `owned-broca`
  rule it is checked by; its results are unchanged.
- The probe's `pi` transcript also carries an OpenAI Responses text block
  whose `textSignature` is `TextSignatureV1` item state. No prune touches
  it, and it never counts as restored.
- The `held_copies_follow_reasoning_pruned` probe serves one transcript
  per serializer profile the runner contract names (`anthropic-messages`,
  `opencode-aisdk`, `owned-broca` and `pi`), each in that profile's native
  stored format with `tags` rows. It picks the transcript from the new
  `PrunedReasoningProbe::serializer_profile` (default `None`) and sends it
  in each Setup's `params.serializer_profile`. A provider with strict
  codecs for the real formats could not read the single generic shape,
  so for such a provider the probe proved nothing. Prunes follow each
  profile's own rules, and views are checked with the new
  `CompactionView::restored_pruned_part_in` (llm-runner 0.7.11's
  `restored_part_in` and `restored_by_edit_in`). Profile transcripts add a
  third conversation that prunes with `opaque_reasoning`. The new
  `held_copies_follow_reasoning_pruned_report` returns a `HeldCopiesReport`
  listing which logical parts (`probe_parts`) and kinds were checked, and
  naming those the profile cannot carry as not applicable (`NotApplicable`).
  Without a profile, the probe serves the generic transcript, with the
  same results as before.
  A kit test runs every profile transcript through llm-runner's own profile
  code (`check_descriptor_rows`, `top_level_part_count`, `display::project`,
  `strip_cache_control`, `carries_pruned_content_in`), so a fixture cannot
  drift from its profile. Every existing named fault fails by name on every
  profile transcript that can carry what it breaks, and a new
  `opaque_reasoning` fault proves the third conversation checks something.
  Track llm-runner 0.7.11.

## 0.7.3

- Add optional `reasoning_pruned` to the Setup request
  (`SetupRequest::with_reasoning_pruned`, `setup::ReasoningPruned`, the
  runner crate's type) and the optional advance-notice header
  `reasoning_pruned: <generation>:<through_ordinal>:<at_ms>`
  (`AdvanceNotice::with_reasoning_pruned`, `encode_reasoning_pruned`,
  `parse_reasoning_pruned`). A notice with a malformed value or generation 0
  is refused `malformed:reasoning_pruned`. Both are absent unless the runner
  has pruned the session, so existing requests, notices and digests keep
  their bytes. The widest well-formed notice grows from 2,313 to 2,391
  bytes, still under the 4,096-byte bound.
- Pin the post-prune Setup (§4), the provider's duty to reconcile its held
  copies once per prune generation from its cut and never below it before
  rendering any view from them (a provider must not accept
  `pruned_reasoning` otherwise), and the runner's
  `pruned_reasoning_restored` view refusal (§7,
  `notice::refusal_reasons::PRUNED_REASONING_RESTORED`), which covers every
  form a view sends bytes in: replace ranges and keep-range part edits.
  `CompactionView::restored_pruned_part` is the reference over a whole view.
  The text states that provider-authored bytes for a pruned message include a
  held part re-serialized inside an edit, not only new content.
- Add the `held_copies_follow_reasoning_pruned` probe to the non-default
  `conformance` feature, with a scripted runner (`ScriptedRunner`) and the
  `PrunedReasoningProbe` adapter trait. Named faulty providers prove it
  fails a provider that renders a held pre-prune copy, renders a copy held
  from before its cut, reads below its cut, re-reads without a generation
  change, or builds a keep-range part edit from a held pre-prune part: one
  trimming the output of a part holding a tool call and its result, which
  re-encodes the call's thought signature, and one editing
  `text_with_state` text, which re-encodes its provider-state signature.
- Add `test-vectors/compaction-provider-v1/reasoning-pruned.json` with
  independently computed notice digests. Track llm-runner 0.7.10.

## 0.7.2

- Add the `account_change` rewrite ride and notice event, with a required,
  validated opaque account label. Ignore the ride's optional `account` field
  on other causes. Providers may compare labels to avoid folding an already
  warm prompt cache.
- Specify pre-send account switches, including retries after refusals that
  wrote no cache, and next-notice attribution for every switch. Older providers
  answer unknown causes `nothing` and ignore unknown events; no rollout gate
  is needed. Add generated valid and malformed account vectors.

## 0.7.1

- Add optional, empty-omitted `accepts` to ready Setup answers, with a setter,
  accessor, `ACCEPTS_PRUNED_REASONING` and vectors. Providers explicitly opt
  into archive-pruned re-reads; older answers grant no pruning permission.
- Track llm-runner 0.7.4, including its clarification that an imported
  assistant display row may have no recorded model.

## 0.7.0

- Reference calls now use named `SessionIdentity` fields so legacy positional
  calls cannot silently compile. State keys, hashes and the focused probe use
  the attested runner principal's bare module id, never `reserved:<id>`.
- Breaking: provider conversation state and reverse-call identity now use the
  daemon-attested runner principal with Setup's harness and session. Project
  folders are route metadata only; existing session-mismatch rules and the
  deliberate lack of harness comparison are unchanged. No wire fields change.
- Add the non-default `conformance` feature's focused folder-alias/runner-isolation
  probe, with faithful and faulty provider tests; it is not a full provider kit.
- Session references in notices and views now use the shared llm-runner 0.7.0
  derivation from runner module id, harness and session, not project folders.
  Refresh independently generated async vectors and document v1 ref migration.
- Update the llm-runner dependency to 0.7.0. Public wire types are unchanged.

## 0.6.0

No behaviour change. The bump follows `cortexkit-role-llm-runner` 0.6.0:
this crate's public API re-exports `SetupCheckpoint`
(`cortexkit_role_llm_runner::tags`) and returns `EntrySource`
(`cortexkit_role_llm_runner::read`) from
`CompactionMessage::model_view_source`, and those types now come from
llm-runner 0.6, which a downstream crate sees as different types from
llm-runner 0.5's.

## 0.5.1

An additive release: the compaction provider sets how long a rewrite ride
may hold a step, and the runner shows the wait. Every new member is
optional and omitted when absent, and answers decode leniently, so an older
runner ignores them and keeps its own short ride deadline.

- `SetupRequest` gains optional `ride_wait_max_ms` (1 to 300,000,
  `SetupRequest::with_ride_wait_max_ms`): the longest this runner can hold a
  step for a rewrite ride (`ttl_expired`, `model_change`,
  `surface_change`). Absent means it does not honour a provider-set wait.
  CONTRACT §4 notes, as information, that the Claude Code gateway sends
  240,000, because Claude Code times a request out 360 s before its first
  byte; a runner sends its host's real limit.
- `SetupAnswer`'s `ready` answer gains optional `ride_wait_ms`
  (`SetupAnswer::with_ride_wait_ms`, `SetupAnswer::ride_wait_ms`): at least
  1 and at most the request's cap, frozen with the session like `ceiling`
  (§12). A provider sends it only when the request carried a cap.
  `SetupAnswer::check_for` now also fails an answer whose `ride_wait_ms` is
  0, above the request's `ride_wait_max_ms`, or present without one,
  naming `setup::RIDE_WAIT_MS`; a runner treats that as a failed Setup on
  the path of a malformed `initial`, and never clamps the value.
- §8: a rewrite ride's `deadline_ms` is Setup's `ride_wait_ms` when the
  answer carried one, and otherwise the runner's own short deadline, at
  most 10,000 (`ride::MAX_REWRITE_RIDE_DEADLINE_MS`, now that default's
  bound). New `ride::MAX_RIDE_WAIT_MS` (300,000). `RideRequest::check`
  accepts any cause's deadline up to 300,000, so a rewrite ride's deadline
  above 10,000 is no longer refused `deadline_out_of_range`; bounding a
  session's rides by its `ride_wait_ms` is the runner's job. The user sees
  the wait as `llm-runner/v1`'s `compaction_wait` (llm-runner 0.5.2).
- Vectors: `setup.json` gains a request carrying `ride_wait_max_ms`, an
  answer carrying `ride_wait_ms`, and a `ride_wait` section of request and
  answer pairs a runner accepts or fails naming `ride_wait_ms`;
  `async.json` (regenerated) moves the rewrite rides over 10,000 ms from
  refused to a new `long_deadline_requests` section and refuses a rewrite
  ride over 300,000 ms. `SHA256SUMS` and `ASYNC_SHA256SUMS` refreshed.

## 0.5.0

Requires `cortexkit-role-llm-runner` 0.5.0, whose lineage rules this release
follows: a lineage is the readable history from the latest cut, and a cut
starts a new lineage without renumbering anything.

- Breaking: a view's `ranges` tile from the lineage's `first_ordinal` through
  `covers_through.ordinal`, not from 0. Everything before `first_ordinal`
  rides only in the view's head. `CompactionView::check` no longer requires
  the first range to start at 0 (it still checks the ranges are contiguous),
  and `view::dispose_fetched` takes the lineage's `first_ordinal`: a view
  whose ranges start anywhere else is refused with the new reason
  `ranges_start` (`notice::refusal_reasons::RANGES_START`), reported as
  `view_refused:<version>:ranges_start`. New `CompactionView::ranges_start`.
- Breaking: Setup runs once per lineage, before the lineage's first model
  call, instead of once per session, a cut's lineage included; a runner may
  call a cut's Setup as soon as it prepares the view carrying the cut. The
  provider keeps its state across the call. A cut lineage's Setup carries
  `descends_from` naming the old lineage at the cut; one naming a cut the
  provider never published is refused `unknown_cut` (`errors::UNKNOWN_CUT`,
  decoded through `RefuseCode::Unknown`, terminal for that lineage). A
  provider sends `checkpoint` only on a Setup without `descends_from`; a
  runner fails a Setup answering one with a checkpoint
  (`setup::CHECKPOINT_ON_DESCENT`, `SetupAnswer::check_for`). The initial
  CompactionMessage's range is usually `{first_ordinal, first_ordinal}`.
- Breaking: the first advance notice of a lineage a cut started carries
  `descends_from` naming the previous lineage at the cut (the cut's
  `through_ordinal`, one below the new `first_ordinal`). The provider keeps
  its state across the cut and reads nothing before the new lineage.
- Breaking: a provider's sync and view building read only through
  `session.read` within the current lineage, never through
  `session.history`. Raw recovery of folded content reads on demand through
  `session.history` or the host's reader.
- Breaking: a view on a lineage whose `first_ordinal` is above 0 leads with
  a `replace` range carrying the history head (it may be zero-length);
  `dispose_fetched` refuses one leading with `keep` as `ranges_start`.
- Breaking: `SetupAnswer` and its `Ready` and `Refuse` variants, and
  `RideAnswer`'s `View` and `Nothing` variants, are `#[non_exhaustive]`, so
  a later optional member is not a breaking change. Build a Setup answer
  with `SetupAnswer::ready` or `SetupAnswer::refuse` and the `with_ceiling`,
  `with_checkpoint`, `with_cache_ttl` and `with_provider_code` setters, and
  read it with `is_ready`, `initial`, `ceiling`, `checkpoint`, `cache_ttl`,
  `refuse_code`, `refuse_reason` and `provider_code`. A `match` outside
  this crate keeps a `_` arm, and a variant pattern keeps `..`. The wire
  bytes don't change.
- Advance notices gain an optional `first_ordinal` header
  (`AdvanceNotice::first_ordinal`, `with_first_ordinal`), at most one above
  `through_ordinal`. The widest well-formed notice grows from 2,280 to
  2,313 bytes, still under the 4,096-byte bound.
- A revert that undoes the latest cut restarts at the newest cut at or below
  the revert point; its first notice descends from the lineage at that cut
  only when the runner had one there, and Setup runs again (§5.1).
- New `cache_ttl` module and optional `cache_ttl: {model, ms, configured}`
  (§8.1) on Setup's `ready` answer, on every `model_change` ride answer and
  on every view; a view's digest frames it only when present, so existing
  answers and views keep their bytes. `cache_ttl::lifetime` is the runner's
  precedence for declaring `ttl_expired`: a configured entry (`null` means
  never), then the lifetime observed in the request, then the provider's
  default, else nothing; only entries for the request's model count.
  Breaking: `RideAnswer::Nothing` and `RideAnswer::View` gain the member,
  and `RideAnswer::View` now holds its view boxed
  (build them with `nothing()`, `view()` and `with_cache_ttl`), and
  `SetupAnswer::Ready` gains it.
- Vectors: `async.json` regenerated. The view refused as "first range does
  not start at 0" (`gap`) is gone, because that view is now well formed;
  every disposition names its lineage's `first_ordinal`; new cases: a view
  on a cut lineage tiling from its first ordinal (accepted, assembled, and
  applied), views tiling from 0 on a cut lineage and from 1 on a lineage
  starting at 0 (`refused:ranges_start`), a cut lineage's first notice with
  its descent, a notice refusing a view `ranges_start`, a cut lineage's
  fork descent kept through the cut, a cut lineage's view leading with
  `keep` (`refused:ranges_start`), notices carrying `first_ordinal` and
  refused for a malformed one, revert notices with and without descent,
  ride answers and a Setup answer carrying `cache_ttl`, views with and
  without it, and the `cache_ttl` precedence decisions. `ASYNC_SHA256SUMS` and `SHA256SUMS`
  refreshed.

## 0.4.2

- CONTRACT §7 states when a deferral ends: at the latest at the first step
  of the session's next run. A steer, a delivered notice or a tool result
  continues the running run, whoever sent it, so a step that starts from one
  does not end a deferral, and a runner that can still defer a view does not
  refuse it. The text said "the next step that starts from a user message",
  which runners could read as including a steer.
- §4 states that Setup runs again when a runner no longer holds its
  recorded answer, as after a restore of its store. A provider already answers Setup
  for a session it has seen before.
- A view may carry an optional `apply` mark (`CompactionView::apply`,
  `view::apply_marks`). The one mark defined, `next_step`, asks the runner to
  apply the view at its next step boundary, with the companion edits the
  model provider accepts there, and to defer only when no such edit exists.
  The provider decides when holding a view back costs more than stripping
  signed reasoning; the runner still decides what its model provider
  accepts. An unmarked view behaves as before. An informative note in §7
  records the measured models on which a marked view can apply mid-run with
  `strip_signed_reasoning`.
- Setup gains an optional `view_apply` list (`SetupRequest::view_apply`,
  `with_view_apply`): the marks the runner honors for the session. A
  provider marks a view only with a listed mark, and a runner refuses a view
  whose mark it did not list with the new refusal reason
  `unsupported_apply` (`notice::refusal_reasons::UNSUPPORTED_APPLY`).
- When `apply` is present, the view digest frames `apply` and the mark after
  the cut. A view without it keeps its digest, so every existing view and
  vector keeps its bytes. `async.json` adds a `view_apply` section: a marked
  view, the same view unmarked with a different digest, a view with a mark
  this crate does not define, and a Setup request carrying `view_apply`.

## 0.4.1

- The contract states the reply shape: a `RESPONSE` body is the bare answer
  object, never wrapped in `result`, and a refusal's fields other than
  `code` and `message` live inside `detail`. New `reply-shape.json` vector.
  Nothing else changes.

## 0.4.0

- Move the complete `compaction-provider-v1` vector set
  from the workspace root into this crate. It now ships in the package,
  exposed through `vectors` with `test-vectors`, including a SHA256SUMS check
  run in CI. Old pinned commits retain their old workspace path;
  new consumers use the versioned crate's compiled-in vectors.

- Providers read using `media: "descriptors"`,
  recording the runner-served message and part content digests rather than
  hashing descriptor JSON. The existing `digest::content_digest` API is now
  a re-export of llm-runner's shared implementation; its bytes do not change.


Breaking. Requires llm-runner 0.4.0, whose tag numbers are computed from the
transcript; this crate's public API names llm-runner types (`EntrySource`,
and now `SetupCheckpoint`), so it moves with it in a minor bump.

- Setup's `ready` answer gains an optional `checkpoint`
  (`setup::SetupCheckpoint`, llm-runner's `tags::SetupCheckpoint`,
  `{tag_checkpoint, cut}`; `SetupAnswer::checkpoint`). A provider MUST send it,
  unchanged, on every Setup answer for a session whose history and context
  state it imported from standalone Magic Context, the plugin that manages
  context inside OpenCode and Pi, and only to a runner declaring llm-runner's
  `tag_checkpoint` group.
- Two new typed, terminal Setup refusal codes, `RefuseCode` and `KnownCode`
  variants: `standalone_session_not_migrated` (a session with standalone state
  and no completed move) and `tag_checkpoint_unsupported` (a moved session on
  a runner without `tag_checkpoint`; ends the run). For
  `standalone_session_not_migrated` the runner's duty depends on its host:
  where standalone Magic Context runs in-process, a runner that has never
  served the session hands it to standalone in the same step and sends no
  rides, while one that has served it fails the run by name; a host with no
  standalone (a module-launched private host, the Claude Code gateway) fails
  the run, or the request, by name and never forwards unmanaged history.
- `setup_identity_mismatch` is now typed, as 0.3.x promised:
  `RefuseCode::SetupIdentityMismatch` and `KnownCode::SetupIdentityMismatch`,
  not retryable and ending the run after Setup, the same behaviour it had
  while it decoded as `RefuseCode::Unknown`. Its wire spelling is unchanged.
- `RefuseCode` and `KnownCode` are now `#[non_exhaustive]`: a later code is
  additive, and a `match` on either outside this crate needs a `_` arm.
- §5.3: the tag-number floor rules are replaced. Nothing durable is keyed by
  a tag number, and renumbering a part a read exposed is a new lineage. The
  rule that a runner which lost its state continues on a fresh lineage
  without `descends_from` is unchanged.
- Vectors: `setup.json` and `errors.json` gain the checkpoint and the two
  codes; `errors.json` lists `setup_identity_mismatch` as a named code.

## 0.3.3

- Pin the runner/provider route identity contract: Setup reports the session's
  caller-chosen `harness` and session; the Setup route supplies `project_root`.
  Providers use that composite identity for conversation calls and use a new
  child session with the Setup harness and route root for provider-owned child
  runs. Shared project routes may carry a generic harness and empty session;
  only a non-empty route session that differs from the Setup-reported session
  may be refused. Runner authorization uses the daemon-attested principal, not
  the harness.
- Name `setup_identity_mismatch` as a Setup refusal wire code without adding
  public enum variants. In 0.3.x `RefuseCode` decodes it as `Unknown`, which is
  terminal and non-retryable; `RefuseCode::is_setup_identity_mismatch()`
   identifies it. The next breaking release will add a typed variant and mark
  `RefuseCode` and `KnownCode` `#[non_exhaustive]`.
- Add route-identity vectors for shared and single-session routes, provider
  reverse calls, child sessions, principal attestation, and named faults that
  reuse an empty session or constant harness. There is no provider
  conformance kit to execute transport calls, so the vectors pin expected
  identities; the wire tests validate their relationships.

## 0.3.2

- Pin a limit on provider-constraint deferral in §7: at the next step starting
  from a user message, apply the view with required companion edits or refuse
  `view_refused:<version>:provider_constraint`, never keep deferring. Contract
  text only: no view member, wire byte, digest or vector changes.

## 0.3.1

- `RideRequest` gains optional `overflow` (`ride::Overflow {reported_limit_tokens}`, both non-exhaustive, built with `Overflow::new`, `with_reported_limit_tokens` and `RideRequest::with_overflow`). Present on a `ceiling` ride, it says the ride follows a model provider error stating that the request exceeded the context window; `reported_limit_tokens` is the limit that error stated, set only when the runner parsed one; `overflow: {}` is valid. Absent means the ceiling came from the runner's own count, never "no overflow happened". On any other cause a provider ignores it: `RideRequest::check` does not fail on it, and `RideRequest::ceiling_overflow` returns it for a `ceiling` ride only. Both members are omitted when absent, never encoded as `null`.
- CONTRACT.md §5.3, runner state lost: a runner that loses or scrubs its own tag or view state continues the session under a new `lineage_id` with no `descends_from`, and claims no truncation (`revert_below` means truncation only). Within a session it never gives a tag number to a different part; new tags go above the floor (every tag number the session may have used before the loss; a runner that cannot tell jumps past any number it could have used), and a part whose restored state still records its tag may keep it. A provider seeing a new lineage without descent drops its tag index, cursor and rendered-view record and re-reads; it may keep derived work (summaries, memories, queued drops) anchored on a message id, or a message id plus part key, whose message or part the re-read still has with a matching content digest, and discards the rest. A queued drop is re-addressed by the part's new tag. Tag numbers never carry across lineages.
- Vectors: `async.json` gains `ride.overflow_requests` (with a limit, `{}`, absent, and ignored on a rewrite cause) and `state_loss` (a notice sequence with the same session, a new lineage, no descent and no revert); `ASYNC_SHA256SUMS` is refreshed. No existing vector changes meaning.
- Patch release: `RideRequest` was already non-exhaustive and the new member is optional, so no Rust caller or wire byte breaks.


## 0.3.0

Compaction moves off the runner's hot path, and the synchronous per-step call is removed. A provider of this role now serves `role.describe`, `compaction.setup`, `view.get` and `compaction.ride`; a runner that pairs with one (the `llm-runner/v1` `compaction` group) implements the runner's side: Setup, advance notices, view application, rides and the ceiling.

Added:

- Advance notices (`notice::AdvanceNotice`) go from the runner to the provider on ck-bus as claim checks. The subject is `ck.{acct}.event.<runner>.compaction.advanced.v1`, the id `<session_ref>:advanced:<seq>`, and the digest is SHA-256 over the length-framed canonical header set. Headers: `session_ref`, `seq`, `lineage_id`, `through_ordinal`, `through_digest` (SHA-256 of the message's JCS encoding), optional `descends_from` (fork descent), `applied_view_version` (the runner's durable view acknowledgement, repeated on every notice), `events`, `est_tokens`, `window`, `model` and `last_usage`. Events: `model_change`, `surface_change`, `ttl_expired`, `revert_below`, `marker_outcome`, `view_discarded`, `view_refused` (reasons `tool_pair_split`, `role_order`, `unknown_part`, `provider_constraint`), `view_deferred` and `view_companion`; unknown tokens decode and are ignored. Values are canonical ASCII, and the headers total at most 4,096 bytes. The provider reads content with `session.read` from its own cursor; the contract pins the exact resync it performs when bytes change under a tag or ordinal.
- Fork descent: a notice that moves the session to a new lineage may carry `descends_from = <lineage_id>:<through_ordinal>:<through_digest>`, parsed from the right because lineage ids may contain `:` (`notice::NoticeDescent`). The provider keeps state derived from entries through the cut only when the digest it recorded for the old lineage at that ordinal matches (`notice::descent_outcome`); otherwise it resyncs. The header is optional; without it the provider resyncs.
- Every event this role defines puts its event name in its bus message id (`<session_ref>:advanced:<seq>`, `<session_ref>:view_ready:<view_version>`), because the event stream deduplicates ids across the whole stream; without the event name a notice and a view-ready event with equal numbers collide and the second is dropped.
- Views (`view::CompactionView`): keep and replace ranges tiling the transcript prefix through `covers_through`. Keep ranges carry per-part edits (`PartEdit {ordinal, mid, part, bytes | drop}`) keyed by llm-runner's opaque part key. Replace and edit bytes are final: the runner renders tags only on parts it sends as written. Views also carry an optional native-marker `cut`, a per-session `view_version` and a length-framed SHA-256 digest. They are announced by `ViewReady` on `ck.{acct}.event.<provider>.compaction.view_ready.v1` and fetched with `view.get` (`ViewGetRequest`, `ViewGetAnswer`). `dispose_fetched` (over `TranscriptMessage`) pins the runner's checks in order. A view applies only at a step boundary, after a durable WAL record; the runner may defer it, apply it with a companion edit, or refuse it, and discards it after a revert below its coverage.
- `compaction.ride` (`RideRequest`, `RideAnswer`) with causes `ttl_expired`, `model_change`, `surface_change` and `ceiling` (decoded open), and a deadline of at most 10,000 ms for the three rewrite causes and at most 300,000 ms for `ceiling`, the only cause that uses `wait_ms` and can end in a refusal. A missed deadline counts as `nothing`.
- `SetupAnswer::Ready` gains optional `ceiling` (`ceiling::CeilingPolicy {at_share, on_overflow, wait_ms}`), with `CeilingPolicy::decide` for the end of the ceiling wait. A refusal uses llm-runner's `context_ceiling {window, size, cause}` with cause `view_timeout`, `provider_unavailable` or `over_window` (`ceiling::CONTEXT_CEILING`, `ceiling::context_ceiling_causes`).
- New modules `notice`, `view`, `ride`, `ceiling`, `digest` (content digest and framing) and `message` (the CompactionMessage Setup returns as `initial`). `MessageRef` and `DescendsFrom` move to `lineage`.

Removed, with what callers lose:

- The synchronous step: `ops::COMPACTION_STEP` and the `status` module (`StepStatus`, `StatusMessage`, usage, estimate, `prefix_rebuilding`, `last_applied`/`last_not_applied`, cursor paging with `take_capped`, `next_cursor` and `DEFAULT_MESSAGE_CAP_BYTES`); the step answers (`answer::StepAnswer`: `noop`, `compaction_message`, `wait`, step `refuse` with `RefuseDetail` and `history_gap_from`, `coverage`); the request fence and version rule (`fence`); `compaction.ready` (`ops::COMPACTION_READY`, the `ready` module); the step kill points `AnswerRecorded`, `WaitAnswered` and `WaitWorkDurable` (replaced by `ViewRecorded`); the codes `compaction_wait_exceeded` and `not_session_compaction_provider`, and `errors::Call` with `KnownCode::ends_run_at` (replaced by `KnownCode::ends_run_after_setup`).
- Setup's step-only declarations: `SetupAnswer::Ready.stability` (`StabilityRank`) and `call_when` (`CallWhen`, `calls_every_step`). A `ready` answer that still carries them decodes and they are ignored.
- Step and hook descent: `descends_from` is declared on Setup (and, for later forks, on advance notices); `DescentLink` no longer takes step or hook declarations.
- The OpenCode host runner lane (former CONTRACT.md §18): `served_through_ordinal`, `unserved_subjects`, sparse pre-op statuses, final-page `wait`, host admission and `SetupRequest::check_runner_params`.
- The native Claude Code lane (former §19), which provided: the `claude_code` capability bundle on `role.describe` and `SetupRequest` (`ClaudeCodeCapabilities`, shared with step-transform-provider); `raw_view` frozen raw replacements on CompactionMessages with exact native endpoints (`coverage.endpoint`, `NativeEndpoint`, `FrozenView`, `ViewRef`, hook-cut `retained_tail`); the optional `compaction.view.read` op (`ViewReadRequest`, `ViewReadAnswer`, `ViewReadRefusal`, `read_frozen_view`); current-request geometry and attributed fill on the status (`geometry`, `output_reserve`, `mid_turn`, `fill`); notice dispatch receipts (`channel2_directive`, `channel2_delivered_id`, `DispatchReceipt`, `NoticeLedger`); `raw_message` provenance sidecars on status messages; and bootstrap completion receipts (`bootstrap_complete`, `BootstrapReceipt`, `BootstrapLedger`, `source_sha256`). Which of these an async Claude Code lane still needs is to be re-specified on top of advance notices and views.
- The async-mode selection of the previous 0.3.0 draft (never released): `compaction_mode`, `setup::CompactionMode`, `session_mode`, `accept_mode`, `async_group::RUNNER_GROUP` / `compaction_async`, the `async_not_supported` refusal code, and the `async` describe group with `RoleDescribe::with_async` / `declares_async`. With every provider serving views, the group meant nothing: `view.get` and `compaction.ride` are required ops.
- The dependency on `cortexkit-role-step-transform-provider`.
- Vectors: `status.json`, `answers.json`, `fence.json`, `ready.json`, `host-runner.json`, `host-runner-lane.json`, `claude-code.json`, `claude-code-native-ingress.json`, `claude-code-bootstrap.json`, `CLAUDE_CODE_SHA256SUMS` and the `joint/` fixture. `setup.json`, `setup-descent.json` (and `SETUP_DESCENT_SHA256SUMS`), `role-describe.json` and `errors.json` drop their step cases; `setup.json` gains the CompactionMessage model-view and malformed cases from `answers.json`. New `async.json`, pinned by `ASYNC_SHA256SUMS`, whose digests a separate implementation (`generate_async.py`) computed.

Minor package bump: removed items and the changed `SetupAnswer::Ready` and `RefuseCode` break Rust callers. The wire also changes incompatibly (the required ops differ), within the v1 alpha draft: nothing live uses the removed step. Adds a `serde_jcs` dependency. Requires cortexkit-role-llm-runner 0.3.0 (was 0.2.0), whose `compaction` group is this role's runner half and which no longer has `compaction.ready`; the workspace's llm-runner conformance crate requires this 0.3.0.

## 0.2.4

- Contract wording only (§6 and §19.2), no code or wire change. For a native
  publication outside a declared partial cut, `coverage` and `endpoint` name the
  replacement boundary exactly: `coverage.ordinal` is `range.to - 1` and
  `endpoint` is the last native block of that message. Only a hook cut's head
  endpoint with its `retained_tail` keeps later blocks of the range. Requires
  cortexkit-role-step-transform-provider 0.2.3, published from the same
  revision.

## 0.2.3

- Require cortexkit-role-step-transform-provider 0.2.2 and publish both crates
  from the same commons revision because the Claude Code capability bundle
  records one revision for both roles. The protocol 0.32 update is confined to
  the step-transform provider's dev-only parity test; no consumer source or
  wire changes are required.

## 0.2.2

- No code change. Published from the same commons revision as cortexkit-role-step-transform-provider 0.2.1, because the Claude Code capability bundle records one commons revision for both roles.

## 0.2.1

- Add optional `SetupRequest.descends_from`, reusing `status::DescendsFrom`,
  and a matching builder. Setup decoding is lenient and the request is
  non-exhaustive, so this is an additive patch with wire major v1 unchanged.
- Advertise setup descent with optional describe flag `setup_descends_from`.
  Callers send setup `descends_from` only when the flag is true; otherwise
  they declare descent on the first step or hook. Absent/false means
  unsupported, avoiding silent loss through older lenient setup decoders.
- Pin parent/cut validation, durable descent links, idempotent repeats and
  `invalid_params` / `lineage_id` conflicts across setup, step and hook.
  Setup without descent leaves the lineage undeclared: first-step and
  first-hook declarations remain valid, and no declaration leaves a root.
- Add a provider-owned `lineage::DescentLink` reference check and shared
  setup/step/hook descent vectors, separately pinned without changing any
  existing vector bytes. The planned
  `cortexkit-role-compaction-provider-conformance` crate will exercise real
  provider modules over routes (CONTRACT.md §17); it is not implemented yet.

## 0.2.0

- Preserve native ingress/hook JSON with optional raw-message sidecars, required
  and syntax-checked only on the admitted Claude Code lane (CONTRACT.md §19.1).
  The provider must verify that `raw_message`, with `cache_control` removed only
  at the Anthropic transport positions listed in CONTRACT.md §19.6, equals the
  parsed `message`. Raw bytes and JSON escaping count toward payload byte caps.
- Reject raw payload references without selected retrieval. Add typed bootstrap completion receipts with generation-scoped replay/conflict checks, exact frozen bundle identity and length-prefixed raw-source SHA-256 framing; a final page alone is not completion.
- Add optional typed native endpoints, frozen raw replacements and bounded, optional `compaction.view.read` with typed digest/version refusals. Inline replacements still apply without retrieval; declared single-message caps refuse rather than truncate.
- Add typed optional geometry, output reserve, native continuation and attributed fill. Geometry is required and validated only for an attested connection whose frozen declaration selects Claude Code. Off-lane absence and supplied geometry preserve old behavior.
- Clarify the gateway geometry invariant: a present output reserve must fit under the absolute wall with the hard input ceiling; omitted fill means no reading. Notice dispatch acknowledgement requires successful 2xx response headers, not merely sending a request.
- Add optional notice directives and explicit next-status notice-id dispatch receipts, with an idempotent reference acknowledgement helper and conflict refusals. Pending notices remain unchanged until acknowledged; only real provider dispatch authorizes acknowledgement.
- Publish and echo the shared optional capability bundle at describe/setup. Existing vectors and SHA256SUMS remain byte-identical; new vectors have separate checksums.
- Minor package bump: optional fields on previously exhaustive `StatusMessage`, `CompactionMessage` and `StepAnswer` variants break Rust struct/variant literals. The wire major remains v1. Update the workspace conformance consumer's dependency range accordingly.

## 0.1.2

- Step statuses gain two optional host-lane fields: `served_through_ordinal`, the highest message ordinal the host has durably served, and `unserved_subjects`, the hook answers the host never served. The subject-part check is shared with the step-transform crate, and tests now load the host-lane vectors.
