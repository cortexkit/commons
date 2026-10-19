# Changelog

## 0.3.0

- Add the `async` describe group (CONTRACT.md §20), which takes compaction off the runner's hot path. A provider lists `async` in `role.describe.capabilities` and serves `view.get` and `compaction.ride`. `check_describe` refuses `async` without both ops (`missing_op`). New: `async_group::GROUP` and `GROUP_OPS`, `ops::VIEW_GET`, `ops::COMPACTION_RIDE`, `RoleDescribe::with_async` and `declares_async`. A session uses the mode only when the runner also declares its half in `llm-runner/v1` (0.2.7); otherwise nothing changes.
- Advance notices (`async_group::notice::AdvanceNotice`) go from the runner to the provider on ck-bus as claim checks. The subject is `ck.{acct}.event.<runner>.compaction.advanced.v1`, the id `<session_ref>:<seq>`, and the digest is SHA-256 over the length-framed canonical header set. Headers: `session_ref`, `seq`, `through_ordinal`, `through_digest` (SHA-256 of the message's JCS encoding), `applied_view_version` (the runner's durable view acknowledgement, repeated on every notice), `events`, `est_tokens`, `window`, `model` and `last_usage`. Events: `model_change`, `surface_change`, `ttl_expired`, `revert_below`, `marker_outcome`, `view_discarded`, `view_refused` (reasons `tool_pair_split`, `role_order`, `unknown_part`, `provider_constraint`), `view_deferred` (hold until a provider-safe boundary) and `view_companion` (applied with a companion edit such as `strip_signed_reasoning`); unknown tokens decode and are ignored. Values are canonical ASCII, and the headers total at most 2,048 bytes. The provider reads content with `session.read` from its own cursor; the contract pins the exact resync it performs when bytes change under a tag or ordinal.
- Views (`async_group::view::CompactionView`): keep and replace ranges tiling the transcript prefix through `covers_through`. Keep ranges carry per-part edits (`PartEdit {ordinal, mid, part, bytes | drop}`) keyed by llm-runner's opaque part key, so a dropped tool output, stripped image or compressed text never re-sends the whole message. Replace and edit bytes are final: the runner renders tags only on parts it sends as written. Views also carry an optional native-marker `cut`, a per-session `view_version` and a length-framed SHA-256 digest. They are announced by `ViewReady` on `ck.{acct}.event.<provider>.compaction.view_ready.v1` and fetched with `view.get` (`ViewGetRequest`, `ViewGetAnswer`). `dispose_fetched` (over `TranscriptMessage`) pins the runner's checks in order: requested, well formed, announced digest, newer, still a prefix (coverage and every edit's message id), part keys known (`refused:unknown_part`), edits in part order. A view applies only at a step boundary, after a durable WAL record. The runner may refuse it (`view_refused`) and discards it after a revert below its coverage (`view_discarded` with `revert_below`).
- `compaction.ride` (`RideRequest`, `RideAnswer`) with causes `ttl_expired`, `model_change`, `surface_change` and `ceiling` (the cause is decoded open), and a deadline of at most 10,000 ms for the three rewrite causes and at most 300,000 ms for `ceiling`, the only cause that uses `wait_ms` and can end in a refusal. A missed deadline counts as `nothing`.
- `SetupAnswer::Ready` gains optional `ceiling` (`async_group::ceiling::CeilingPolicy {at_share, on_overflow, wait_ms}`), with `CeilingPolicy::decide` for the end of the ceiling wait. A refusal uses llm-runner's `context_ceiling {window, size, cause}` with cause `view_timeout`, `provider_unavailable` or `over_window` (`async_group::CONTEXT_CEILING`, `context_ceiling_causes`).
- A session selects its mode in Setup. `SetupRequest` gains optional `compaction_mode` (`setup::CompactionMode`, `sync` | `async`, strict on value; absent means `sync`), and `SetupAnswer::Ready` gains an optional `compaction_mode` echo. The session is async only if the Setup asked for `async` and the ready answer echoes `async` (`setup::session_mode`); every other combination, including a missing echo, is sync. A runner asks for `async` only when the provider declares `async` and its own llm-runner describe lists `compaction_async` (`async_group::RUNNER_GROUP`, llm-runner 0.2.7). New refuse code `async_not_supported` (`RefuseCode::AsyncNotSupported`, `KnownCode::AsyncNotSupported`; not retryable, ends a run after Setup only) lets a provider refuse an async Setup from a runner without that group; `setup::accept_mode` is the reference. `errors.json` gains the code in its `refuse_codes` and `retryability` lists, and CONTRACT.md §11 and §14.3 gain matching rows. Adding a `RefuseCode` and a `KnownCode` variant breaks exhaustive matches on those enums, which this minor release already permits.
- `compaction.step` is marked as removed in the next breaking release, once every runner declares `async`. It stays required until then.
- Vectors: new `async.json`, pinned by `ASYNC_SHA256SUMS`. Its digests were computed by a separate implementation from this crate's. The only change to an existing vector is the `async_not_supported` entries added to `errors.json`; existing checksum manifests are unchanged.
- Minor package bump: adding `ceiling` to the exhaustive `SetupAnswer::Ready` variant breaks Rust struct-variant literals and exhaustive destructuring of `Ready`, as 0.2.0 did for its variants. The wire major stays v1, and a Setup answer without `ceiling` keeps its bytes. Adds a `serde_jcs` dependency. The workspace's llm-runner conformance crate now requires 0.3.0.

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
