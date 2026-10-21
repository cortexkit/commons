# Changelog

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
