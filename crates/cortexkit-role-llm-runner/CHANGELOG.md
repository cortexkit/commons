# Changelog

## 0.7.37

- Add CONTRACT §4.2.1 and `served_bytes`: a runner that reads from a host
  may keep each served message's digest and a `stripped` flag instead of its
  body, rebuild every read from the host's stored row, and serve it only when
  the rebuilt body's digest matches. Any other difference is refused
  `served_bytes_changed` (`errors::ServedBytesChangedDetail`, `{mid}`), and
  that message is served again only under a new lineage.
- Per-profile strip lists name the members a host rewrites after a message
  finished without changing what the model sees. `opencode-aisdk` leaves out
  a user message's `info.summary`; every other profile's list is empty.
  Messages served before a runner adopts the list keep the bytes they were
  served with (`stripped: false`).
- Add `served-bytes.json` and a `served_bytes_changed` row to `errors.json`.

## 0.7.36

- Add opt-in provider service levels, validated per-wire support in discovery,
  fixed per-run sends, and applied versus provider-confirmed served usage.
- Keep record levels open strings and request levels strict, with named
  unsupported-tier refusals. Export the wire API list for cross-crate drift checks.
- Preserve explicit standard on sends; only an absent request member is omitted.
  Absence means standard when starting a run and inherits a running run's tier.
  Callers that never set the field retain their bytes; older runners ignore it.
- Keep token-only `StepUsage` Copy; owned step tier metadata and a tier-aware
  aggregation helper retain only unanimous reported served values for a run.
- Specify cache-busting tier-change rides and the readers-first notice rollout.
  The closed step-finished bus event is unchanged; tiers live in usage rows.


## 0.7.35

- `role.describe` gains optional `step_hooks`: the step-transform hook call
  sites ([CONTRACT.md](CONTRACT.md) §11.2) a runner runs for every session,
  named as `step-transform-provider/v1` names them (`describe::step_hooks`).
  Previously §11.2 required every hook call site of every runner, and a plan
  composer couldn't tell which runners take step-transform items. The
  contract now requires a composer to add an item only when its
  subscriptions name at least one hook and the runner lists every hook they
  name (`RoleDescribe::serves_step_hooks`), leaving it out otherwise, and a
  runner to refuse a plan whose item subscribes an unlisted hook. Absent or
  empty advertises no hook. `RoleDescribe::with_step_hooks` builds the
  member, and `check_describe` reports a duplicate name as
  `duplicate_step_hook`. Additive: older versions of this crate ignore the
  member when decoding.
## 0.7.34

- `display::projects(profile)` says whether this crate has a display
  projection for a serializer profile: `anthropic-messages`, `opencode-aisdk`,
  `opencode-v2` and `pi`. CONTRACT.md §4.7 now states that the projected-row
  rule applies to those profiles only. For any other profile, such as the
  runner-owned profile, the runner supplies its own projection; the contract
  doesn't prescribe that profile's raw-body-to-row mapping, and its other
  display-row requirements still apply. Additive; no wire change.

## 0.7.33

- Correct two contract sentences and the `try_number_parts` docs, which said a
  dropped part is rendered as its `[dropped §N§]` placeholder. Under an
  applied view, a `drop: true` edit is left out of the request on every
  profile and for every part kind, as compaction-provider §7 already pinned;
  a counted part keeps its number and still counts. A runner-rendered
  placeholder comes from an applicable truncate edit, such as
  `truncate: "placeholder"`. Text only; no code or wire change.

## 0.7.32

- Add `tags::truncate_part`, non-exhaustive `TruncateMode`, `PartContainer`
  and `TruncateRefusal` for profile-exact runner-rendered compaction edits.
  Refuse unknown or inapplicable modes with `truncate_kind`; derive container
  context from stored rows, including Pi result blocks.
- Pin 184 reference vectors across all five profiles, attachment and error
  preservation, numbered/unnumbered placeholders and UTF-16 input boundaries.

## 0.7.31

- Pi (§9.6): a runner's compaction-marker checkpoint is the member
  `details.cortexkit_checkpoint` of its `compaction` entry, written in the
  same append. Neither Pi nor Oh My Pi copies anything from a compaction
  entry's `details` into model content. `details` may hold other members (an
  extension's record, a runner's private records, an early draft's
  `details.cortexkit`); a reader takes only this one, so none of them is
  ever an anchor. An offline move that adopts a Pi head writes the
  member on the in-effect compaction entry and rewrites only that entry's
  `firstKeptEntryId`, in one atomic replacement of the journal.
- Add `tags::PI_MARKER_CHECKPOINT_MEMBER` and
  `tags::pi_marker_checkpoint(entry, session)`. It returns `None` unless the
  entry is a `compaction` whose `details.cortexkit_checkpoint` decodes as a
  `MarkerCheckpoint` naming that session; every other member of `details` is
  ignored.
- `message-checkpoint.json` adds a `pi_marker` section: one compaction entry
  before and after a move's apply, the checkpoint it carries, and seven
  entries that are no anchor.

## 0.7.30

- OpenCode 1 (§4.8): a dead step, an assistant message with no parts, no
  `time.completed` and no `error`, is no longer counted or served by a
  runner declaring `message_checkpoint`. OpenCode 1 never sends a part-less
  message to the model, and never adds a part to a dead step. The rule
  applies to settled rows; a step that may still write to its assistant is
  unfinished. Other part-less messages, including a part-less assistant
  that finished with an `error`, still count, so no other row changes
  eligibility. A message's ordinal and `message_count` change wherever a
  dead step precedes it, so anything that counted dead steps recomputes
  from the predicate. `tags::opencode1_message_counts_with_parts` returns
  `false` for a dead step; the data-only `opencode1_message_counts` is
  unchanged. The `message-checkpoint.json` counting vector adds an
  uncounted `dead-step` between two counted messages and a counted
  `error-step`; `message_count` is now 7.
- Add `serializer_profile::opencode_v2_projected_content(row)`: one decoded
  served OpenCode 2 row's model-visible native units, including synthesized
  shell and working-directory text, so a codec computes part digests and tag
  text from that row alone. It refuses compaction rows and checks only the
  row. A test checks it against `projected_content` for every ordinal-bearing
  row in the recorded and synthetic `opencode-v2-session.json` cases.

## 0.7.29

- Add the opt-in `retract_own_send` describe group, which requires `retract`.
  A daemon-attested sender may withdraw its own pending submission on the
  same session, with the same answers and withdrawal semantics, including
  durability, as an owner's retract. On a session with a recorded owner,
  other non-owners and submissions without a recorded sender stay
  owner-only; request fields never supply the caller's identity.
- Runners that do not declare the group retain the owner-only retract rule
  on owner-protected sessions. On a session with no recorded attested owner, a
  runner may admit any reachable caller; that existing rule is unchanged.

## 0.7.28

- Add the compatible `opencode-v2` serializer profile for OpenCode 2's decoded
  native messages, with dense sequence-ordered ordinals, native-slot part
  counts and inclusive compaction boundaries. Never-sent rows retain ordinals.
- Report self-referencing runner cut markers separately from native cuts;
  the runner's own checkpoint determines their served range. Pin marker
  removal and fallback with recorded revert rows.
- Follow OpenCode 2.2's loader, with 2.0.x support best effort. Preserve
  recorded 2.0.26 stale fork references as named refusals, not repaired rows.
- Share session projection and display helpers. Notices keep their native
  roles; tool calls and outcomes share one native slot; images use descriptors.
- Ship extracted OpenCode 2.0.26 and 2.2.3 stored rows, a deterministic extractor,
  independent session/display expectations and labelled synthetic cases.
- Add `serializer_profile::pi_projected_content(row)`: one served Pi row's
  model-visible content, including synthesized shell text and branch
  summaries, so a codec computes part digests and tag text from the served
  row alone. A test checks it against `projected_content` for every
  projected entry in `pi-session.json`.

## 0.7.27

- Add `serializer_profile::pi_served_body(entry, PiEdit)`, the served body
  for one ordinal-bearing Pi entry given the active path's last context edit
  for it (`PiEdit::None`, `Removed` or `Replaced`). It shares its code with
  `pi_session_projection`, so a runner or reader that receives entries one
  at a time computes the same `served_body` without a second copy of the
  normalization, removal and string-to-text rules. It checks the entry and
  the replacement content only, not journal relationships. A test compares
  it with `served_body` for every projected entry of the recording and the
  synthetic journals in `pi-session.json`.

## 0.7.26

- Serve Pi rows with their context edits applied. `PiOrdinalEntry` gains
  `served_body`: the stored journal row as Pi's loader reads it, with the
  active path's last edit for it applied to its model-visible content.
  An entry no edit targets keeps its stored JSON value; legacy missing or
  null content is normalized as Pi's loader does. A null edit serves the
  row with nothing for the model: `content: []` for a message or custom
  message, an empty `summary` for a branch summary, and
  `excludeFromContext: true` added to a shell run. The row's digest and
  part count are computed over the served body, so a reader that has only
  served rows recounts the runner's parts.
- A late Pi context edit to an entry already served requires the runner to
  start a new lineage from that entry's ordinal. The contract said
  "permits"; this aligns it with the helper's documentation, which already
  said "requires".
- Inside the served window, history entries and display rows follow the
  served body. Only host-history pages below the window serve raw stored
  rows. A planner that reads Pi's journal directly uses
  `pi_session_projection` on the whole journal.
- `pi-session.json` pins every entry's served body, including the recorded
  edit, and adds a synthetic case for each removal shape.

## 0.7.25

- Complete the Pi profile: share durable active-path selection, edited part
  counts, ordinals and native cuts between runners and offline planners.
  Excluded shell runs and removed content retain zero-part ordinals.
- Add Pi display projection with runtime roles, shell command/output text,
  and image descriptors. Keep ordinary image parts and count shell images
  separately from shell text. Raw journal rows remain unchanged.
- Document template sends as visible custom messages, other sends as user
  entries, and late context edits as step-boundary lineage changes.
- Ship checksum-pinned Pi 1.3.0/1.4.0 recordings and independent expected
  session/display results, with clearly labelled synthetic edge cases.

## 0.7.24

- Clarify the runner's optional `history_edit_cost` notices and pre-render
  `chain_reset` rides. Deploy readers first: every compaction provider that
  receives a runner's notices must pin compaction-provider 0.7.9 (or a later
  release defining the header) before that runner sends it, or notice
  digests disagree. No runner wire types
  or behavior outside these optional signals change.

## 0.7.23

Text only; no type, wire shape or test vector changes.

- §9.13 names the `cortexkit-rich-ui-syntax` version this crate depends on
  (0.3.0 at this release) for `scan_references`, the same scanner
  `output_refs::Resolution` uses. It said 0.1.1, which this crate never used.
- The case table lists `held_calls_status_event_matches_run_status`, the
  conformance case added in kit 0.15.25, with its known limit: this document
  doesn't mark which control events are status events.
- Examples that named one implementation now describe the behaviour without
  naming it, and the closing section comparing one implementation with this
  document is removed. It was pinned to a release long superseded.
- `AMENDMENT-NOTES.md`, the review log of an earlier amendment, is no longer
  shipped in this repository. Its outcomes are recorded in `CONTRACT.md`.

## 0.7.22

- Text only: clarify that display `call_key` presence follows either
  `session.read` attribution (§5) or `run.tool_calls` (§6.3), whose keys have
  the same lifetime. No rule, wire shape or type changed.
- Document either reference in the `call_identity` conformance requirements.

## 0.7.21

- Spell out how `turn_system` renders on each provider route: an in-order
  developer input item on OpenAI Responses, a mid-conversation system message
  only on listed Anthropic models and at legal boundaries, and user-authority
  fall-through elsewhere. Keep stored entries at their recorded positions,
  using user authority when the later route cannot accept system text.
- Add six provider-shaped rendering vectors and a structural test for order,
  Anthropic placement and unsupported-route authority. No wire or type change.

## 0.7.20

- Add opt-in `run_cancel_by_interrupt`, requiring `run_cancel` and
  `delivery_order`: a session's currently interrupt-granted plan sender may
  cancel `now`, without starting another turn. Other callers and cancellation
  points remain refused; `session.retract` stays owner-only.
- Add opt-in `refresh_relaunch`, requiring `session_change` and
  `session.subscribe`, for a runner whose host fixes its tool set at start.
  Apply tool-set changes by relaunch at the rung's run boundary, preserving
  history, lineage and ordinals. Optional failures fall back to the old
  definitions while immediately gating removed tools; required failures end
  the run `host_relaunch_failed`. Failed changes remain pending, with an
  optional `Baseline.pending_failure` record and a durable owner control event,
  `session_refresh_failed`. Baseline decoding remains lenient for rollback.
- Preserve exhaustive `RefreshRequest` struct literals with additive strict,
  non-exhaustive `RelaunchRefreshRequest`, carrying optional `required` on the
  same wire body. The next breaking release folds `required` into
  `RefreshRequest` and marks that type non-exhaustive. Add a required request
  vector and old-shape baseline compatibility coverage.
- Make `surface_change` ride ordering explicit: after successful relaunch,
  before the first new-surface request, also for system-text changes without
  restart; no ride on unchanged-surface fallback. Runners declaring neither
  new group retain their existing behavior.

## 0.7.19

- Fix OpenCode 1's finished-summary test in `opencode1_message_counts` and
  `opencode1_message_counts_with_parts`: `summary` now matches JSON `true` or
  any number equal to 1, so a stored `1.0` is excluded like `1`. Before, a
  `{"summary": 1.0, "finish": "stop"}` row was counted. §4.8 states the rule,
  and the row-list vector adds that row as not counted.

## 0.7.18

- Exclude nonempty compaction-only OpenCode 1 messages from §4.8 counts
  and served reads so native cuts preserve §4.5's session-wide ordinals.
  Messages with compaction and any other part still count; empty-part
  messages retain their existing treatment.
- Add `opencode1_message_counts_with_parts`, taking the message row's data
  and the types from its separately stored part rows. Keep
  `opencode1_message_counts`'s existing finished-summary-only behavior for
  compatibility. Update the row-list vector to cover compaction-only,
  mixed compaction/text, and empty-part messages with separate message
  and part rows.

## 0.7.17

- Add opt-in `message_checkpoint` (requires `tag_checkpoint`), optional
  paired `message_count` / `head_cut` on Setup and native marker keys, and
  checked native `next_message_count`. Reuse `read::HeadCut` and its ordinary
  full inline digest (SHA-256 over JSON Canonicalization Scheme, RFC 8785,
  bytes) and Unix epoch millisecond timestamp.
- Pin adoption message numbering, distinct message and tag anchors, the
  after-folded-message anchor when the retained cut disappears, reload and
  missing-key behavior, and OpenCode 1 counted-message rules. Runners without
  the new group keep their existing rules. Add shared anchor and counting
  helpers with worked digest, backward-decoder and revert-table vectors.
- For `message_checkpoint` runners, uncounted host rows are never served on
  any read, on adoption and native lineages alike. Clarify for every runner
  that read stability includes host changes made while the runner was
  stopped: check frozen messages before the next model call and change
  lineage on any difference. No offline certificate format is prescribed.
- Require `message_checkpoint` runners to answer `session.head` during
  their own outstanding Setup without waiting on that Setup or its gates,
  using the newest complete in-effect marker key or earlier Setup checkpoint.

## 0.7.16

- Add the `best_effort` runner reason `host_unsupported` when the host cannot
  send a schema in a constrained form, regardless of route support. Consumers
  still decode reasons as open strings. Add a canonical result/status vector
  and a round-trip/well-formedness test.
- Add the optional `structured_output_per_session` describe group, requiring
  `structured_output`. Its first admitted send fixes the whole member, present
  or absent, for the session; later sends must also omit an omitted first
  member, or match a present first member's whole-member JCS bytes. Changes
  are refused `invalid_params` naming `structured_output` without writes.
  `check_describe` refuses the group alone by name. The default per-run rule
  is unchanged.
- Add the optional `submit_streamed_siblings` describe group, extending
  `session.send` and requiring `structured_output`; `check_describe` refuses
  the group alone with `submit_streamed_siblings_without_structured_output`.
- Contract §6 and §9.4 declare a variant for a runner whose host dispatches
  calls as they stream: earlier submit-step siblings may keep their normal
  dispatched results and are not skipped. Every later call, including later
  `submit_result` calls, is still never dispatched and receives a durable
  `skipped_after_submit` result. The first submitted value still ends the run
  `completed` and is validated. Without the group, the strict rule is unchanged.
- A caller requiring that no sibling ever runs must avoid `submit_tool` on
  such a runner, or offer no tools with side effects in that run.

## 0.7.15

- Text only. Contract §4.3 now says which message `session.head.cut` names
  for a cut a host made before the runner adopted the session: the last
  message the host's compaction folded away, never a message the host keeps
  visible after compacting, such as its compaction marker or the summary it
  shows. A consumer that drops everything up to `cut` therefore never drops
  that summary. `read::HeadCut`'s doc says the same. No type, wire shape or
  vector changes.

## 0.7.14

- Add the `rich_ui_stamp` describe group (contract §9.17), listing
  `session.read` and `session.head`. A runner declaring it copies the rich UI
  stamp `{dialect, catalog_digest}` verbatim from its frozen plan's fetched
  system-text answers (tool-provider 0.13.2's `system_text.rich_ui`) and serves
  it, fixed per run, on every assistant display row the run writes, on the
  run's `run_started`, on `session.head` as `last_run_rich_ui` while
  `last_run_state` names the run, and on the run's assistant `session.read`
  entries when the read opts in with `include_rich_ui: true`. Never on user,
  steered, tool, imported or adopted rows. Absent from such a runner means
  unstamped; from any other runner it says nothing.
- Two fetched system-text answers declaring different stamps are refused at
  admission `plan_stale`, writing nothing, with one `{kind: "rich_ui",
  provider, rich_ui}` difference per stamped answer.
- Add `rich_ui::RichUiStamp` (the same wire form and validation as
  tool-provider's type: a malformed stamp is refused naming
  `rich_ui.dialect` or `rich_ui.catalog_digest`), `rich_ui::plan_stamp` and
  `rich_ui::RichUiConflict`; optional `DisplayRow::rich_ui`,
  `RunStarted::rich_ui`, `ReadMessage::rich_ui` and
  `HeadMeta::last_run_rich_ui`, omitted when absent, with `with_*` setters;
  `ReadRequest::include_rich_ui`, omitted when false, with
  `ReadRequest::check_rich_ui` naming it for a runner without the group.
  `HeadMeta::lineage_consistent` refuses a last-run stamp without
  `last_run_state`. `LastRunState` is unchanged.
- Without the opt-in, `session.read` replies keep their bytes, and every
  existing vector is unchanged. New `rich-ui-stamp.json`.

## 0.7.13

- `reasoning-pruned-profiles.json`: the `owned-broca` `applied_edits`
  example is now a shape a served page carries. It was a `text_with_state`
  text edit, but a served `owned-broca` page serves text as plain `text`,
  because provider state never leaves the runner, so a compaction provider
  building part edits from served parts cannot produce one. It is now
  `tool_call_input_trimmed_keeps_signature`: an assistant `tool_call` with
  trimmed `input` and its `thought_signature`, before the prune and as
  `prune_edit_in` rewrites it (without `thought_signature`). No rule
  changed. The vectors README is updated, and both checksums with it.

## 0.7.12

- Pin that a prune also rewrites the runner's applied view (CONTRACT
  §9.14.1, new "Applied edits are pruned with their parts"). A keep-range
  part edit can re-serialize a part the runner later prunes, such as a drop
  placeholder on an `opencode-aisdk` tool part that still carries the
  call's `thoughtSignature`. When it prunes, the runner applies the same
  per-profile rule to every such edit, in the same commit as the prune, so
  nothing it sends afterwards carries pre-prune bytes. It never drops the
  edit, and it refuses to prune a message whose edit holds a pruned part
  in a form the rule cannot express.
- Add `archive_prunes_reasoning::prune_part_in` (a part's pruned form for
  a profile and kind), `prune_edit_in` (an edit's bytes after a prune of
  its message) and `PruneRefused`, whose `Inexpressible` variant is that
  refusal. Both functions are tested against every profile's `prunes`,
  `not_carried` and `edit_refusals` vectors.
- `reasoning-pruned-profiles.json` gains `applied_edits` per profile: for
  `owned-broca`, `opencode-aisdk` and `pi`, an applied edit before and
  after the prune; empty for `anthropic-messages`, which carries no
  `signature` content. Its checksum is updated.

## 0.7.11

- Correct the `owned-broca` prune rules, which are what
  `carries_pruned_content`, `restored_part` and `restored_by_edit` check.
  0.7.10's `text_with_state` rule looked for `provider_state.signature`,
  which matches no real stored part: a real `provider_state` is
  `{format, data}`. Under `signature`, the rule now goes by `format`
  (`provider_state_formats`). For `gemini` and `google_code_assist`,
  `data` is the JSON string `{"thoughtSignature": ...}`: it is emptied to
  `""`, and non-empty `data` counts as restored. For `open_ai_responses`
  and `open_ai_responses_chatgpt`, `data` is item state: it stays
  byte-identical and never counts. Any other format is not a signature,
  and must be classified in the contract before a runner prunes it. Also,
  0.7.10's `opaque_reasoning` ignored a `reasoning` part's `signature`; it
  now removes that optional signature and keeps `text` byte-identical,
  because on Claude the signature is the encrypted thinking and on Gemini
  it is the thought signature. The `text_with_state` cases in
  `archive-prunes-reasoning.json` and `reasoning-pruned.json` now use the
  real shape, with digests recomputed.
- Pin the runner's duty for requests after a prune (§9.14.1), for every
  profile. Drop any reasoning-type part whose signature or opaque payload
  a prune emptied or removed. Send any other pruned part without the
  emptied member. Never send an emptied member.
- Add per-profile archive-pruning rules (CONTRACT §9.14.1, new per-profile
  table). Until now the prune reference read only `owned-broca` shapes: it
  missed an `anthropic-messages` `thinking` block under `reasoning`, flagged
  its signature under `signature`, and found no parts at all in an
  `opencode-aisdk` (`parts`) or `pi` (`/message/content`) body. New
  `archive_prunes_reasoning::kinds_carried`, `carries_pruned_content_in`,
  `restored_part_in` and `restored_by_edit_in` take the serializer profile
  and read its own part container and members. A profile the table does not
  name answers `UnsupportedProfile`, never "nothing pruned". The functions
  without `_in` are documented as the `owned-broca` reference and answer
  as `_in(owned-broca)` does; their two corrections are in the entry
  above. Every row cites its source. `anthropic-messages` cites Anthropic's
  extended-thinking documentation (a `thinking` block's `signature` is the
  opaque payload, emptied by `opaque_reasoning`). `opencode-aisdk` cites
  OpenCode v1.18.35, which copies AI SDK `providerMetadata` verbatim into
  `part.metadata`. Only listed provider entries count (`anthropic`,
  `bedrock`, `google`, `vertex`, `openai`), plus OpenRouter
  `reasoning_details`, whose `reasoning.text`, `reasoning.encrypted` and
  `reasoning.summary` entries each kind prunes in place; an unlisted entry
  carries nothing until the table lists it. `pi` cites pi's public
  `packages/ai`: a `textSignature` that is OpenAI Responses
  `TextSignatureV1` item state (`{"v": 1, "id", "phase"?}`) stays
  byte-identical under `signature`, while a Google or Vertex thought
  signature is removed.
- Add `test-vectors/llm-runner-v1/reasoning-pruned-profiles.json`: per
  profile, its prunes in native shapes, parts a kind must not match, and a
  refused and an allowed replacement and part edit per carried kind.

## 0.7.10

- Add the session's prune state for runners that declare
  `archive_prunes_reasoning`: `reasoning_pruned: {generation,
  through_ordinal, at_ms}` (`archive_prunes_reasoning::ReasoningPruned`) on
  `session.head` (the authority) and on raw and model `session.read` pages.
  It is absent until the first prune; the generation is 1 after it and rises
  by one in the same commit as every later prune; `through_ordinal` is the
  highest ordinal any prune touched; `at_ms` is for display only. Every
  member is optional and omitted when absent, so existing bytes are
  unchanged (§4.2, §4.3, §8, §9.14.1).
- Pin the runner's duties after a prune of an enrolled session: call the
  provider's Setup, carrying `reasoning_pruned`, before any other provider
  call for a run; let a ready answer replace only `accepts` and, while no
  view has applied, `initial`; and refuse a view whose replace range would
  restore pruned content, `view_refused:<version>:pruned_reasoning_restored`,
  by the kind recorded for each pruned part. The rule is about content, not
  edit shape: it covers a replace range's bytes and a keep-range part edit's
  bytes (a drop restores nothing), from `view.get` and ride answers alike,
  and the bytes include a held part re-serialized inside an edit. The refusal is permanent for
  that view. State the brief head-index window after a prune as allowed
  (§9.14.1, §11.1).
- Add `PRUNED_REASONING_RESTORED`, `PrunedPartAt`,
  `carries_pruned_content`, `restored_part` (replacements) and
  `restored_by_edit` (part edits) as the reference check, and
  `test-vectors/llm-runner-v1/reasoning-pruned.json`: heads never pruned and
  pruned twice, pages repeating the head, an invalid generation 0, and
  refused and allowed replacements for each pruning kind, including a cut
  lineage's head range standing for ordinals below the cut, and keep-range
  part edits refused and allowed for each kind. Existing vector
  files are unchanged.
- Add the four new conformance cases to the §15 table.

## 0.7.9

- Widen the `owned-broca` error output's `module` member: it is now 1 to
  255 bytes of UTF-8 with no control character, instead of 1 to 128 bytes
  of printable ASCII. Readers used to refuse a whole history page over a
  module id with a space, non-ASCII text, or 129-255 bytes, even though
  such ids occur in real deployments; the member's only job is one line in
  the model's text, so the only property it needs is no line break. A
  reader that refuses less cannot break a producer that followed the
  narrower rule, so this change is compatible (§4.4, §13).
- Move the space, non-ASCII and 129-byte module samples from
  `invalid-error-outputs.json` to `valid-error-outputs.json`. Add a valid
  sample with a module of exactly 255 bytes spelled with multi-byte
  characters, so the limit is exercised in bytes, and invalid samples for
  a 256-byte module, a module with a newline, and one with a C1 control
  character. The tab sample stays invalid. `code`, `detail`, `retry` and
  `nothing_sent` are unchanged.

## 0.7.8

- Add the tool-outage members of an `owned-broca` error output: optional
  `module`, `code`, `detail`, `nothing_sent` and `retry` after `message`,
  each omitted when absent, never `null` and never empty. `module` is 1-128
  bytes of printable ASCII 0x21-0x7E, `code` matches
  `^[a-z][a-z0-9_]{0,63}$`, `detail` is one line of at most 1 KiB, `retry`
  one line of at most 256 bytes, and `nothing_sent` only the literal `true`.
  The member set is closed, and readers refuse every violation by member
  and rule. A plain error is unchanged byte for byte (§4.4, §13).
- Pin the runner-neutral rendered block every runner uses for a module
  outage, and the exact `nothing_sent` condition: a refusal at route open,
  or `module_reloading` before the send, never a timeout or a lost reply
  (§11.4).
- Add `serializer_profile::error_output`: `ErrorOutput` with `decode`,
  `check`, `render` and an encoder in the wire order, the non-exhaustive
  `ErrorOutputRule`, `ErrorOutputViolation`, `is_valid_module` and
  `is_valid_code`. Add `test-vectors/owned-broca-tool-outage/` with real
  producer captures, valid boundary samples and one refusal per rule.

## 0.7.7

- Include compaction-provider's `account_change` among the rewrite causes
  covered by `compaction_wait`; account-switch rides follow the same wait
  reporting rule as model-switch rides.

## 0.7.6

- Pin `input_tokens`, cache reads and cache writes as disjoint counts across
  run usage, per-model and session totals, and step events. Require provider
  normalization and deduplication, with separate cost pricing. Document that
  raw normalization belongs in each runner's parser tests, outside the kit's
  canonical scripted usage. Add a one-step disjoint-input usage vector.

## 0.7.5

- Add optional `read::HeadMeta::last_run_error`, its builder/accessor, and
  `read::LastRunError` with open classification and provider retry timing from
  the same run's `run.status`. Absence means not reported, never permanent;
  paused and interrupted runs carry no error summary. Omit text and diagnostics.
- Add `read::RetryAfterSecs`, a finite nonnegative fractional-seconds newtype
  encoded as a JSON number, preserving `HeadMeta: Eq`. Invalid delays are
  omitted from the head, not rounded or replaced with zero. Add an error-head
  vector without changing existing vector bytes.

## 0.7.4

- Add the declared `archive_prunes_reasoning` group, its three pruning kinds,
  and optional `pruned_reasoning` metadata on raw/model read and raw/display
  history pages. Pin native required-field preservation, write-time digests,
  reader replacement rules, at-rest exclusion and compaction Setup opt-in.
  Ship original/served native-format vectors with matching JCS identities.
- Clarify display `model`: runner-written assistant messages always carry
  their recorded model; imported messages without one may omit it. Absence
  means unknown, never the current model or a guess. Document reader rollout.

## 0.7.3

- Add the durable provider-attempt `attempt_id` to `step.finished.v1`, required
  without `harness_step_ref`, with a builder, producer check and decode check.
  Pin its grammar and restore-safe identity independently of display facts.

- Pin a tool result as one numbered part: prefix only its first non-empty
  exposed text block, ignoring empty text for classification and placement.
- Add `ToolResultMediaOnly`, `media_label` and `MEDIA_LABEL_TEXT` for results
  containing non-image media without text; preserve the image-only label.
- Add shared `ToolResultBlock`, `tool_result_kind` and `numbered_block_index`
  helpers. Empty and structured-value-only results do not count; JSON in a
  text block remains ordinary counted text. Add block-rendering vectors.

## 0.7.2

- Add the `call_identity` describe group (contract §9.16). A runner declaring
  it puts `call_key` (the key `session.read` and `run.tool_calls` expose for
  the call, once prepared or dispatched) and `schema_pin` (the
  `tool-provider/v1` pin of the definition the call was dispatched against,
  frozen at dispatch) on every display tool call and on the live
  `tool_started` and `tool_finished` events. A call never dispatched or one to
  a runner-native tool has no pin. `check_describe` refuses the group without
  `history_display` or `live_events` (`call_identity_without_display`).
- Add optional `call_key` and `schema_pin` to `display::ToolCallPart`,
  `live::ToolStarted` and `live::ToolFinished`, omitted when absent, with
  `with_call_key` and `with_schema_pin` setters, and
  `DisplayRow::tool_calls_mut` for filling them. Every existing vector keeps
  its bytes; the new `call-identity.json` pins both members, `call_key`
  only, and neither.

## 0.7.1

- Add default-omitted `min_hold_days` and `deletion_waits_for_readers` describe
  limits; reject a floor above `max_days`. Floors are minima, not additions.
- Add optional `HeadMeta::released` and typed release timestamps and refusal
  detail. Release remains pending and readable throughout the hold, including
  a zero-floor drain window. Existing constructors and absent-member bytes stay
  unchanged; the extended structs were already non-exhaustive.
- Pin same-computer reader-gated deletion and durable, caller-scoped
  `log-read-watermark/v1` progress. Export its capability/op/refusal names and
  strict request and number/null reply types, with new packaged vectors.

## 0.7.0

- Keep the per-session retention policy and live step event releases intact.
  Section 14.3 pins the event's session reference to the shared section 14.1
  derivation on the bare runner module id.
- Breaking: `session_ref(&SessionIdentity { runner, harness, session })` names a hosted session by
  its runner module id, harness and session, with no folder input. The shared
  helper hashes hand-ordered JCS fields exactly as given, without normalisation.
- Named fields make old three-string positional calls fail to compile instead
  of silently reusing the wrong inputs; a compile-fail doctest pins that fence.
  `runner` is the attested principal's bare module id, never `reserved:<id>`
  or any display-form prefix. A vector pair pins those strings as distinct.
- Keep deprecated `session_ref_v1(harness, project_root, session)` only for
  migrating stored refs: re-key live sessions once; ended rows keep v1 refs.
- Pin both derivations in packaged vectors, including runner separation,
  Unicode, escaping and empty-string edges for the new pure derivation.

## 0.6.11

- Additive release alongside `session_retention` from 0.6.10; its policy group
  and §9.14 remain unchanged.

- Omit folder paths from the closed live hint and refuse `project_root` as
  unknown content. Treat `session_ref` as the runner's opaque reference.
  Add `decode_step_finished`, which takes the publisher from the actual live
  subject and refuses a different body `runner`.

- Add the closed, content-free `step_events::StepFinished` body for plain
  `step.finished.v1` bus hints, native durable `StepRef`, and checked
  `role.describe.step_events` policy metadata. Publish only after durability;
  consumers subscribe/buffer before reading durable history and re-query after
  reconnect. Keep step/run usage types unchanged; copy per-step counters with
  `with_usage` and add optional TTL splits and reasoning inclusion facts.
- Add full/minimal vectors and named declaration/closed-decode refusals.

## 0.6.10

- Add the separate `session_retention` group: strict per-session policy on send
  and adopt, describe limits, frozen policy and source/limitation state on head,
  and session-bound owner release with durable pending deletion. Legacy §9.1
  retention and existing wire vectors remain unchanged.
- Pin activity clocks, release ownership independent of parent scopes, deletion
  and restore incarnation safety, backup refusal and best-effort request dumps.
  Add policy/release types, validation, refusal names and shared vectors.
- Additive: the extended public structs (`RoleDescribe`, `SendRequest`,
  `AdoptRequest`, `HeadMeta`) are non-exhaustive. New describe problem names use
  the existing `Undecodable` variant rather than extending the exhaustive enum.

## 0.6.9

- Use `cortexkit-rich-ui-syntax` 0.3.0, which pins `pulldown-cmark` exactly at 0.13.5 instead of 0.13.0, so dependent crates no longer have to downgrade the parser. Every sample parses as before; the runner API and wire are unchanged.

## 0.6.8

- Add `serializer_profile::OWNED_BROCA` and `PI`, and support their part containers in `top_level_part_count` and `ReadMessage::check_descriptor_rows(profile)`. Malformed containers are refused by profile name; existing profiles and wire types are unchanged.
- Pin the four supported profiles' part containers in the contract, with unit tests for message samples, descriptor shapes and malformed-container refusals.

## 0.6.7

- Update the tool-provider test dependency to 0.13.0 for the `tool-result/v1` format rename. Runtime dependencies and wire behavior are unchanged.

## 0.6.6

- Pin part keys as runner-defined opaque strings, not serializer-profile derivations; descriptors list exactly the complete keys served by the entry's tags rows, in the same order, preserving the existing tags Reads rule.
- Add provider-side `ReadMessage::check_descriptor_rows(profile)` for digest shapes, unique keys, profile-derived part counts and exact equality with tag-row keys. Missing tag rows are refused by name. Keep `check_descriptors(keys)` as the runner-side strict order check.
- Add `serializer_profile::top_level_part_count` for `anthropic-messages` and `opencode-aisdk`, refusing unknown profiles by name. Existing vector bytes are unchanged.

## 0.6.5

- Add the `system_text_default` describe group and typed accessor, plus the provider capability name without a runtime tool-provider dependency.
- Pin the joint composer rule and catalog admission check, with terminal `system_text_preset_required`, its typed provider detail, and describe/refusal vectors. Existing vector data is unchanged.
- Keep refusal codes open strings and unknown codes terminal; no refusal-code enum is introduced.

## 0.6.4

- Require cortexkit-rich-ui-syntax 0.2.0. The functions this crate calls
  behave as before, so nothing else changes.

## 0.6.3

Text and vectors only: no wire type or behaviour changes.

- Say what `media: "blocks"` returns: the stored message exactly as the
  runner stores it, including media bytes the runner's own schema keeps
  inline (a `data:` URL, for example). The group adds no member carrying
  bytes, and a stored message need not hold the media's sha256. The frame
  budget still applies: a stored message too large for one reply is refused
  `reply_too_large` in blocks mode, and descriptors mode is how a reader
  pages it.
- Clarify that `reasoning_tokens` is a breakdown of `output_tokens`, never an
  addition to it, so it never exceeds `output_tokens`. A consumer charges
  `output_tokens` once and never adds `reasoning_tokens` on top; a missing
  `reasoning_tokens` means not reported, not zero.

## 0.6.2

- Add the `head_sender` describe group for `session.head.sender`. With the
  group, absence means no grant for the caller; without it, absence means
  unknown, including on older runners. Declaring runners must report the
  owner's grant and every plan-named caller's grant.
- Add `HeadMeta::sender_grant` and non-exhaustive `read::SenderGrant` with
  `Granted`, `NoGrant` and `Unknown` outcomes. Model, usage and incarnation
  semantics and all existing vector bytes are unchanged.

## 0.6.1

- Add ungated `session.head.incarnation`, an optional opaque 1–128-byte log
  identity. Pin OS randomness/non-reuse and preservation across cuts, reverts,
  restarts, restores and exported moves; unknown means readers fall back to
  lineage-scoped claims. Add a vector and byte-length validation.

- Add optional `session.head.model` (`read::HeadModel`) for the runner's
  recorded next-send selection, independent of frozen plan bytes and with no
  describe gate. An absent model means unknown.
- Add the `session_usage` describe group and optional `session.head.usage`
  (`usage::SessionUsage`): flattened run-usage totals with required `since`
  run ID. Pin incremental, durable spend across runs, restarts, adopted turns,
  history versions and compactions; rewinds never subtract spend.
- Add head vectors for both optional members, including two `by_model`
  selections with and without a variant. Existing vector bytes are unchanged.
- Bound every session usage count, including `by_model`, to the exact JSON
  integer range 0 through 2^53 − 1; add accepted/rejected boundary vectors.
- Add ungated `session.head.sender` (`read::HeadSender`), the attested
  caller's delivery grant under the effective next-send plan, including the
  owner form. Absence means no grant, not unknown.
- Clarify `compaction_item.preset` is optional: only name catalog-listed
  presets, and preserve omission on provider calls. Plan items remain generic
  JSON; add a preset-less plan vector and admission/round-trip regression.

## 0.6.0

A breaking release. A runner could not always report a tool call's start
or finish time honestly: a call replayed from history stored before the
runner kept times has none, and the only choices were inventing one or
failing.

- `live::ToolStarted::started_at` and `live::ToolFinished::finished_at` are
  now `Option<u64>`, left out of the event when `None`. A present time keeps
  its exact bytes. `ToolStarted::new` and `ToolFinished::new` take the time
  as `Option<u64>`.
- CONTRACT §7.1: a runner reports the time it recorded, and leaves the
  member out only when it recorded none; it never invents, estimates or
  substitutes one (not now, not zero, not the message's time). A call the
  runner sends to its tool itself carries both. Readers accept a missing
  time and never reconstruct one.
- `live-events.json` adds a `tool_started` and a `tool_finished` replayed
  from history stored without times, and a `tool_times` section naming the
  times a reader gets from them (`tool_times_are_recorded_or_left_out`).
  Existing vectors keep their bytes; `SHA256SUMS` is refreshed.

## 0.5.5

- `output_refs`, requiring `tags`: immutable writing-request records, snapshots,
  descriptors, display metadata, paging and byte-range read shapes. Shared
  vectors pin the canonical JCS record encoding and digest.
- Use published `cortexkit-rich-ui-syntax` 0.1.1 for reply/program scanning and
  exact token slots; enforce reference and snapshot budgets across each set.
- `output_refs::ReferenceSlot` is a runner-owned, non-exhaustive wire type
  with a constructor and getters. Provider-slot byte compatibility is checked
  against the shared catalog vectors only in tests; tool-provider is not a
  runtime dependency.
- Clarify that delivery boundaries depend on host staging, not acceptance;
  staging remains pending, retractable and expiring until delivery.

## 0.5.4

- Add the optional `held_calls` describe group: typed `run::HeldCall` entries
  on run status and status events identify provider late-result waits with no
  deadline. Progress watchdogs leave these runs alone; owner cancellation is
  still allowed. Held reasons remain open, display-only strings.
- Pin late-result application and omission of empty lists, including exact
  status bytes and feature-gated vectors. No public enum variants added.

## 0.5.3

Additive `attachments` group (CONTRACT §9.12). Runners gain session-bound
chunked uploads, decoded-size and commit-integrity admission helpers, and a
shared media marker/text-result renderer. Callers can upload a photo or PDF
larger than one daemon frame, then name committed ids in optional ordered
`session.send.content` after the prompt. Absent content is omitted; existing
prompt-only sends keep their exact bytes. Callers must discover the group
before sending references, since older runners ignore unknown send members.

- New non-exhaustive upload and media-read types, constructors/accessors,
  `ContentPart::upload`, `SendRequest::with_content`, and open upload states.
  Every new refusal travels as a plain code string with a helper predicate,
  so a reader built before 0.5.3 decodes it as an unknown code instead of
  failing. An id from another session is indistinguishable from an unknown
  one.
- Default reads render media as text markers and multi-part tool results as
  ordered newline-joined text. `MediaMode::Blocks` opts in to stored blocks;
  descriptors keep `media_ref`. Message/part digests stay over stored content.
- `session.media_read` serves at most 2 MiB of decoded bytes per chunk, only
  for digests the bound session's history references. Zero-length requests
  probe full size/MIME without returning data. Upload status distinguishes
  lost open data after restart; committed media has pinned retention.
- Packaged vectors cover discovery, every new request/reply/refusal, legacy
  send bytes, marker renders, a text/image/text tool result, and cross-session
  status/commit refusals; `SHA256SUMS` pins their bytes.

## 0.5.2

An additive release. The Alfonso apps now send messages into sessions
through Thalamus, a sender the session's plan names, while the session's
owner (core) keeps sending its own messages into the same sessions. These
additions let the owner tell a person's turns from its own, let an app interrupt,
and say whose authority such a run uses. Every new wire member is optional
and left out when absent, so existing payloads keep their bytes; stored
message bodies and the default `session.read` entry gain nothing.

- New capability group `run_cause` (CONTRACT §3, §7.1,
  `capabilities::RUN_CAUSE`). On a runner declaring it, the live
  `run_started` event carries `send_id`, the send that started the run
  (`live::RunStarted::send_id`, `RunStarted::with_send_id`): a queued send's
  own `send_id` (never an id the runner made for its queue), the
  interrupting send for an interrupt (never the interrupted one), and
  nothing only for a turn the host started with no send, such as a person
  typing in the host on an adopted session (§9.5). It is never empty and
  never guessed. It is a group rather than a bare optional member because
  absence must mean something: from an older runner a missing `send_id`
  says nothing, while from a `run_cause` runner it means no send started
  the run. The owner then reads its own `send_id` as its own message, any
  other as an app's (a person's), and none as the person typing in the
  host. A runner that cannot name the causing send does not declare the
  group. `check_describe` refuses `run_cause` without `live_events` by name
  (`describe::problem_names::RUN_CAUSE_WITHOUT_LIVE_EVENTS`).
- The interrupt grant (§9.7). A plan `senders` entry may carry
  `interrupt: true` (`delivery_order::PlanSender::interrupt`,
  `PlanSender::with_interrupt`), frozen with the plan at admission. A named
  sender may send the `interrupt` delivery string only with that grant;
  otherwise it is refused `invalid_params {field: "delivery"}`, writing
  nothing. A granted interrupt behaves exactly as the owner's. `interrupt`
  stays a string; it is not a list mode. The new
  `delivery_order::admit_delivery` admits a string `delivery` from the
  caller: `queue` and `steer` as the lists they stand for (so a sender
  granted `idle_start` may send the string `queue`), `interrupt` on its
  grant. `PlanSender` is `#[non_exhaustive]`, so the new field breaks no
  caller; one that builds it uses `PlanSender::new` and the setters.
- Authority stays the owner's (§9, §9.7), for every runner: a run that a
  named sender's send starts or joins runs under the session owner's
  principal and scope, never the sender's, and its tool calls carry the
  owner's principal.
- A display row (§4.7) of a user message may carry `sender`, the attested
  module id of whoever sent it, where the runner recorded it
  (`display::DisplayRow::sender`, `DisplayRow::with_sender`). Like
  `client_message_id` it is kept beside the stored message, never in its
  body or in the default `session.read` entry, whose readers decode stored
  bodies strictly.
- Runner-owned waits are recognized by `deadline_ms`, not by `reason` (§6,
  §6.1, §7). A run in `active` or `paused` that carries `deadline_ms` is
  in a wait the runner owns and ends by itself, whatever its `reason`; a
  watchdog or owner must not nudge, stop or end it before `deadline_ms`,
  and once that time has passed the run counts as ordinary silence again.
  Callers should key on `deadline_ms` (the new
  `run::RunStatus::runner_wait_until` returns it for such a run), never on
  `reason`, which stays an open, display-only string, so a later wait kind
  needs no watchdog change. A runner sets `deadline_ms` only during such a
  wait, never on a run waiting for a caller, and never later than the
  wait's own bound. The context-ceiling wait is the first such wait.
- New known run `reason` `compaction_wait` (CONTRACT §6,
  `run::RUN_REASON_COMPACTION_WAIT`, `RunReason::CompactionWait`,
  `RunStatus::compaction_wait`). A runner holding a step for a rewrite ride
  (compaction-provider's `ttl_expired`, `model_change` or `surface_change`)
  reports it once the ride's answer has been pending for 1 s
  (`run::COMPACTION_WAIT_REPORT_AFTER_MS`): the run stays `active` with
  that reason and `deadline_ms`, the absolute time the ride's deadline
  ends, on `run.status` and on a durable control event when the wait
  starts; `session.head`'s run state, where served, carries the reason
  only. Both go when the ride
  resolves. A ride answered within 1 s is never reported. It is a
  runner-owned wait, so a watchdog keys on its `deadline_ms`. How long the
  ride may take is now the compaction provider's choice, within the
  runner's cap (compaction-provider 0.5.1, Setup's `ride_wait_max_ms` and
  `ride_wait_ms`). `context-ceiling.json` gains a `compaction_waiting`
  section.
- §9.7 gains informative caller guidance: a sender whose message only
  makes sense inside the run in flight sets `expires_at_ms`, so a send that
  cannot be delivered now does not land in a later run. It adds no runner
  obligation. CONTRACT defines the `[informative]` marker it uses.
- Vectors: `role-describe.json` accepts `run_cause` beside `live_events`
  and refuses it alone; `live-events.json` gains `run_started` with
  `send_id` and a `run_causes` section (a queued send, an interrupt, a host
  turn without `send_id`); `display-requests.json` gains a page whose user
  rows name their `sender`; `delivery-order.json` gains a sender granted
  `interrupt`, a plan refused for a grant that is not a boolean, and a
  `string_admission` section of send requests with a string `delivery`.
  `SHA256SUMS` is updated.

## 0.5.1

An additive release. Every new wire member is optional and left out when
absent, so existing payloads keep their bytes; stored message bodies and the
default `session.read` entry gain nothing. Three new capability groups gate
what an older runner does not serve.

- `session.history` takes `form: "display"` on a runner declaring the new
  `history_display` group (CONTRACT §4.7). Each entry then carries a
  canonical row (`display::DisplayRow`: role, text, parts in order, tool
  calls with their arguments, tool results, files described by refs, and
  `model`) in place of the stored body, on a `display::DisplayPage`.
- The new `display` module holds one pure projection per serializer
  profile, `display::anthropic_messages` and `display::opencode_aisdk`
  (and `display::project` by name). A runner calls it and fills what its
  body lacks with `DisplayRow::with_model` and
  `DisplayRow::with_client_message_id`.
- Caller-chosen cuts: `limits: {arguments_bytes, result_bytes,
  error_result_bytes}` (`display::Limits`) on the display read and on
  `session.subscribe`. A cut ends on a UTF-8 character boundary
  (`display::cut`) and sets `arguments_truncated` or `truncated`. Without
  `limits` nothing is cut. `display::APP_LIMITS` holds the Alfonso apps'
  values: 2,048, 16,384 and 65,536 bytes.
- `serializer_profile::OPENCODE_AISDK` names the `opencode-aisdk` profile
  and pins its body: OpenCode's stored message `{info, parts}` (§4.4).
- `session.subscribe` carries typed events on a runner declaring the new
  `live_events` group (§7.1, the `live` module): durable
  `message_started`, `message_finished`, `tool_started`, `tool_finished`,
  `run_started`, `run_finished` and `run_paused`, and display deltas in
  Broca's deployed shape with `mid` and `part` added. Every frame carries
  `seq`, from 1, rising across both lanes; a newly attached stream first
  gets `display_reset` and one coalesced delta per part of the step in
  flight. A reader decodes an unknown event type as `display_reset`.
  `live::LiveSubscribeRequest` is the request with `limits`.
- `session.send` takes `client_message_id` on a runner declaring the new
  `client_message_id` group (§9.11): 1 to 64 characters of `[A-Za-z0-9_-]`,
  shown on the user message's display row, never in `session.read`, and part
  of the send's identity (`SendRequest::with_client_message_id`,
  `client_message_id`, `same_client_message_id`).
- `session.head` may carry `input: {attachments, modalities?, mime_types?}`
  (`read::HeadInput`), what a send that names no model can carry. Absent
  means unknown.
- New vectors: `display-anthropic-messages.json`,
  `display-opencode-aisdk.json`, `display-requests.json` and
  `live-events.json`, pinned in `SHA256SUMS`.

## 0.5.0

A breaking release. A runner never does per-step work, or holds memory, in
proportion to a session's whole length; reads stay within the current
lineage, and the whole history moves to an on-demand op.

- Breaking: CONTRACT §14.2 (new) states the cost rule. Per step, a runner's
  work scales with the messages added since its previous step, plus a
  constant; it never re-reads, re-checks or re-renders the window. Writing
  the model request is excluded. Resident memory scales with the window
  since the latest cut. Acceptance is each runner's own benchmark: 150,000
  seeded messages against 1,000, median step time within 2× and memory
  flat.
- Breaking: a lineage is the readable history from the latest cut, and a
  cut starts a new lineage (§4.5). `lineage_id` changes at a cut, a revert
  and a recovery that drops or rewrites written messages, and at nothing
  else. Read stability holds within one lineage. Ordinals stay
  session-wide and never renumber.
- Breaking: `session.head` gains `first_ordinal`, the current lineage's first
  ordinal (`HeadMeta::first_ordinal`). It is present whenever `lineage_id`
  is, and `HeadMeta::lineage_consistent` now requires it and checks it is at
  most one above `last_ordinal`. `HeadMeta::new` takes it as a parameter.
- Breaking: `session.head` gains `cut: {mid, digest, written_at}`
  (`read::HeadCut`, `HeadMeta::cut`, `with_cut`): the message the latest
  cut folds last, by identity, never content. A lineage without a cut
  carries none and starts at ordinal 0; `lineage_consistent` requires a cut
  whenever `first_ordinal` is above 0. A cut the host made before the runner
  adopted the session may come with `first_ordinal` 0.
- Breaking: a revert that removes the latest cut's message undoes that cut.
  The new lineage starts at the newest cut whose message survives, and its
  first notice descends from the lineage the runner was on at that cut,
  only when the runner had one there; below every cut it starts at
  ordinal 0 with no cut and no descent. Setup runs again either way.
- Breaking: range and after-message reads, and `session.resolve`, refuse a
  position below `first_ordinal` with the new code `before_lineage_start`
  and `detail.first_ordinal` (`errors::BEFORE_LINEAGE_START`,
  `errors::BeforeLineageStartDetail`). The tail reads the newest page
  within the lineage. Resolve no longer answers numbers inside summarized
  history.
- Breaking: Setup runs once per lineage, before the lineage's first model
  call, for every new lineage, a cut's included; a runner may run a cut's
  Setup as soon as it prepares the view carrying the cut. Each tag
  checkpoint belongs to exactly one lineage (§9.6, §11.1). A checkpoint
  comes only on a Setup without `descends_from`; a cut lineage's Setup
  carries `descends_from` and its numbering continues from the old
  lineage's highest number, so a cut renumbers no part. A checkpoint on a
  descending Setup is a failed Setup with reason `checkpoint_on_descent`.
  A runner may refuse reads `transient` while a tag-rendering run waits for
  a Setup without descent, never during a cut's Setup.
- Breaking: `session.head` and every `session.history` page carry
  `history_version` (`HeadMeta::history_version`,
  `HistoryPage::history_version`): it rises at every lineage change but a
  cut (a revert, a rewriting recovery, a restart after lost state), never
  at a cut or an append, and never decreases. `lineage_consistent` and
  `HistoryPage::check` require it with a lineage.
- `session.history` gains ordinal seeks, `from_ordinal` and
  `before_ordinal` (`HistoryMode::FromOrdinal`, `BeforeOrdinal`,
  `read::fields::BEFORE_ORDINAL`), for ordinals the runner assigned; any
  other is refused with the new code `unknown_ordinal`
  (`errors::UNKNOWN_ORDINAL`).
- New group `host_history` with the op `session.history`
  (`ops::SESSION_HISTORY`, `capabilities::HOST_HISTORY`, the new `history`
  module): the session's whole surviving history, paged oldest first by
  opaque `older` and `newer` cursors, from the newest page, `from_mid` or
  `before_mid`. Entries carry `mid` and `written_at` always and `ordinal`
  only where the runner has one; window entries are exactly what
  `session.read` serves, and entries below `first_ordinal` carry no tags.
  A cursor survives a cut; one a revert invalidated is refused with the new
  code `cursor_expired`. A runner that is its own host must declare the
  group; a runner on a host may. The compaction lane never syncs from it.
- `read::fields` gains `CURSOR`, `FROM_MID` and `BEFORE_MID`.
- Vectors: `head.json` regenerated (every answer with a lineage now carries
  `first_ordinal`, and every one above 0 its `cut`; new consistent and
  inconsistent answers, a host cut with `first_ordinal` 0 among them); `errors.json`
  adds the two codes; new `history.json`. `SHA256SUMS` refreshed.

## 0.4.3

Contract text only; no code, wire or vector changes.

- §11.1: the ceiling's run state and `context_ceiling_wait` status apply
  wherever those surfaces are served. A runner that serves none of them,
  such as one without `run_ops` that fronts a host's own requests, reports
  the ceiling to its host, and the refused host request carries the window,
  the size and the cause.
- §11.1: Setup does not run again while the runner holds its recorded
  answer. A runner that no longer holds it, as when a detected restore
  starts a new lineage whose recorded answer did not survive or that the
  runner discards with the lost lineage, calls Setup again before its next
  model call.

## 0.4.2

- Correct the descriptor-mode resolve vector: it previously resolved a
  user-message image, which this contract never numbers. It now resolves an
  image-only tool result with its image ref nested in place; a test checks that
  the tool-result part gets a tag number.

## 0.4.1

Additive for 0.4.0 callers: no published signature changes.

- `session_change` (§10.2) has pinned shapes in the new `session_change`
  module. `session.refresh {plan, policy, generation}` (strict) fetches the
  plan's items as admission does and holds them as the pending change; it
  answers `{accepted_generation, pending, run_in_progress,
  applies_from_next_run}`, and an identical repeat gets the same answer.
  `session.refresh_policy {generation, policy, send_id}` answers `{pending}`
  and is idempotent on `send_id`; `session.flush_prefix {send_id}` answers
  `{pending_generation?}`. Refusals carry typed details:
  `rung_unsupported {surface, rung}`, `pending_superseded {generation,
  pending}`, `already_applied {generation, applied_by}`, and the new
  `generation_stale {accepted}` and `tool_outside_host_set {names}` (a tool
  outside the set the host fixed at start, or a changed definition under a
  kept name; `outside_host_set` decides membership). A refresh can also meet
  admission's `fetch_unavailable`, `plan_stale` and `tool_name_collision`.
- The next-run rule: a change never applies inside a running run; the
  earliest is the start of the next run, and `immediately` means from the
  next run. `session.flush_prefix` applies at the next run, never mid-run
  (it previously said the next step). While a change is pending, a send
  carrying the effective or the latest accepted plan is not drift
  (`check_send_drift`); `plan_drift` then names the latest accepted plan.
- `stop_when.max_steps` (§9.3.1): a run ends `max_steps` only when the step
  that reached the limit requested a tool call that would lead to another
  step. A limiting step with no tool call ends `completed` with its final
  message, directives and structured output, and one calling
  `submit_result` ends as `submit_result` always does.
- Reply bodies: every role reply is exactly `{"result": <answer>}`. New
  `wire::reply_body` builds one, and `wire::read_reply` refuses a bare
  answer or extra top-level keys by name (`ReplyShapeError`). Stream events
  stay bare; a refusal's fields other than `code` and `message`, `class`
  included, live inside `detail`. New `reply-envelope.json` vectors.
- `TRANSPORT_FRAME_LIMIT` (4,194,304 bytes, header included) names the
  limit `reply_too_large` reports; it equals the daemon's
  `MAX_DELIVERED_FRAME_LEN`.
- `tags::try_number_parts` refuses numbering that would pass `u64::MAX`
  with `NumberingOverflow {checkpoint, counted}` instead of repeating a
  number. `tags::number_parts`, which saturates, is deprecated.
  `tag-numbering.json` gains `overflow` vectors.

## 0.4.0

- In-process hosts clean the model's text at their save seam (the hook where
  the host stores a finished reply), by the frozen plan of the run that wrote
  it. The Claude Code gateway cleans on every exposure instead.
- Command cutoffs and expiry follow the same forwarded/never-forwarded distinction as
  crashes: `outcome_unknown` versus `failed`, retained and replayed unchanged.


- `session.read` and `session.resolve` accept strict `media: "inline" |
  "descriptors"` (inline by default). Descriptor reads replace byte-backed
  media and large text, arguments and reasoning payloads in place without
  losing tool pairing or signatures. `ResolveReply.content` is now JSON: the
  inline string is unchanged, while descriptors can be structured content.
  The shared `read::descriptors` primitives provide strict refs, UTF-8 heads,
  and a deterministic 3-MiB per-message budget with a largest-first fallback
  for smaller payloads. Only an over-budget non-payload envelope can still
  refuse `reply_too_large` in descriptors mode.
- `ReadMessage.digest` and `part_digests` carry full-inline content identities
  in descriptors mode, optional in inline mode and independent of tag rows.
  Compaction's existing JCS content-digest implementation moved here and is
  re-exported from the provider crate; payload SHA-256 remains a separate hash.
  New `read-descriptors.json` vectors cover the Anthropic content layouts,
  nested media, paired calls/results, a UTF-8 head crossing, and frame fallback.

Breaking. Tag numbers are no longer a stored, never-reused ledger: a number is
only the name a part shows the model, computed from the transcript, and
everything durable is keyed by the host's own identity (message id, part key,
content digest). CONTRACT.md §9.6 is rewritten around that.

- **Which parts count** (§9.6). A part gets a number if and only if it is
  rendered with one: user text (a steer stored as a user message included),
  a tool result's text (error text included), the model's text, and an
  image-only tool result, which renders a label `§N§ [image]`. Images and
  files in user messages, reasoning, tool calls, the history head, compaction
  markers, `turn_system` messages and empty text never count. One predicate,
  `tags::counts` over the new `tags::PartKind`, drives both counting and
  rendering. Earlier revisions let a runner tag reasoning and tool calls.
- **Numbers.** `checkpoint + position` in the order the runner renders the
  history from the cut, never storage order (`tags::number_parts`, with
  `tags::RenderedPart`). The cut is keyed on the field (`tail_start_id`, else
  the message holding the compaction part), and the anchor falls back from
  in-effect markers carrying a checkpoint for the session to the Setup
  checkpoint at its cut (`tags::anchor`, `tags::Marker`,
  `tags::MarkerCheckpoint`, `tags::SetupCheckpoint`). Gaps exist only behind
  the tail; a dropped part keeps its number as `[dropped §N§]`
  (`tags::dropped_placeholder`).
- **Lineage.** Renumbering a part a read exposed starts a new lineage, and a
  renumbering detected mid-step takes effect at the next step boundary, never
  before that step's tool calls run. That is why `session.resolve` keeps its
  shape with no anchor: it answers the current lineage, and a number no part
  of it shows is `unknown_tag`. Numbers may repeat across lineages.
- **Scope.** Numbering and rendering apply only to sessions whose plan sets
  `render_tags: true`; elsewhere rows carry keys and no numbers. Earlier
  revisions numbered every session of a `tags` runner.
- **Cleaning the model's own tag notation.** The shared functions port the tag
  cleaning in standalone Magic Context, the plugin that manages context inside
  OpenCode and Pi (`tag-content-primitives.ts`, `text-complete.ts`).
  `tags::clean_model_text`, `tags::strip_leading_tag_notation` and
  `tags::render_counted_text` apply on every exposure of text a tag-rendering
  run wrote (`tags::exposed_model_text`,
  `tags::TextWriter`). Untagged runs' text and a structured-output run's final
  text are never cleaned; nor are signed thinking, encrypted reasoning or
  tool-call arguments. In 0.3, rendering left a model's text as written when
  its text part carried provider state (a Gemini thought signature or
  ChatGPT's `phase`). Cleaning now applies on every provider. Earlier revisions
  stored and answered model-written `§N§` byte for byte.
- Descriptor fallback sizing serializes the whole message once, adjusts its
  encoded size by each replacement's exact compact-JSON size difference, then
  checks the final full encoding. A mixed-size vector pins largest-first
  selection and a tie at the cut, alongside the equal-size vector.
- **Freeze and switching.** The rules are frozen per run under
  `runner-tags/v2` (`tags::RULES_FEATURE`). A runner moving from a stored
  ledger switches at its next run or user turn, keeps every stored number,
  takes the ledger's high-water as its starting checkpoint and stays on the
  same lineage; a detected restore starts a new lineage.
- **New describe group `tag_checkpoint`** (requires `compaction`; refused as
  `tag_checkpoint_without_compaction` otherwise): the runner honours a
  compaction provider's Setup checkpoint `{tag_checkpoint, cut}`.
- **New terminal refusals** in `errors::CODES`: `fork_unsupported`
  (`errors::ForkUnsupportedDetail {parent?}`) on `session.send` and
  `session.adopt` for a session the runner knows was forked from a managed
  session, and `template_part_gone` (`errors::TemplatePartGoneDetail {slot}`).
- **Template slots** (§9.7). The `tag` slot type is replaced by `part_tag`,
  whose value is `{mid, part}` (`delivery_order::PartRef`), never a number;
  the runner renders the number the carrying request shows. A part not shown
  refuses the send `template_part_gone` at admission, or ends a queued send in
  the new terminal send state `template_part_gone` (`send::states`,
  `SendReply::slot`). `delivery_order::render` now takes a `shown` lookup
  instead of an `assigned` predicate. **`template_digest` changes** for every
  template with a tag slot: the slot is spelled differently, and a template
  still declaring `tag` no longer decodes, so senders regenerate their
  catalogs.
- **Person-started turns are runs** (§9.5): on an `adopt` session every turn
  the host starts has a `run_id`, appears on `session.subscribe`, and answers
  `run.status` and `run.result`. It carries no `send_id`.
- Every public enum this release adds or extends is `#[non_exhaustive]`
  (`tags::PartKind`, `tags::Anchor`, `tags::TextWriter`, and the extended
  `send::SendState`, `delivery_order::Slot` and
  `delivery_order::TemplateProblem`): a later variant is additive, and a
  `match` on one outside this crate needs a `_` arm.
- Removed: `tags::next_after`, `tags::RenderKind` and `tags::render_text`
  (replaced by `tags::render_part`).
- Vectors: new `tag-cleaning.json` (generated by running standalone's own
  functions, with its test cases and this crate's own), `tag-numbering.json`
  and `tag-checkpoint-describe.json`; `tags.json`, `delivery-order.json` and
  `errors.json` follow the new rules; `SHA256SUMS` regenerated.

## 0.3.10

- Persist user-command invocation records and the terminal `outcome_unknown`
  answer/status after a crash, timeout or lost reply. Retries and status reads
  replay the stored outcome without forwarding again; callers must not retry it
  automatically and must tell the person it may have run. Add `is_outcome_unknown`
  helpers for the open user-command state strings and a status vector without
  changing wire members.

## 0.3.9

- Keep the role independent of tool-provider at runtime. Runner-owned command
  and reply types are checked against tool-provider vectors through a dev-only
  dependency; `unknown_user_command` carries `{command}` detail on both roles.

- Add the independent `user_commands` group, frozen provider-qualified
  baseline commands, owner invoke/status shapes and refusals, and the plan's
  optional boolean `interactive`. Absent baseline bytes stay unchanged.
- Pin history isolation, once-per-invocation forwarding, bounded ordered
  progress, retention and the 2–10 s pull rhythm. Host menus are optional:
  only hosts with out-of-history replies may list commands; built-in or
  already-registered names are qualified by module. Keys use the host's id or
  a shim id minted once and reused on retry. Add packaged vectors.

## 0.3.8

- Add optional `RoleDescribe.delivery_order_modes` and `with_delivery_order_modes` so a runner can name only the list modes it can ever land. Absence keeps the existing group mapping. Presence only narrows it; `steer` still requires `steer`, and `idle_start` still requires `queue`. Legacy string deliveries are unchanged.
- Add `Mode::is_declared_by` for describe-aware decisions; keep `is_declared` as the absent-member mapping. `check_describe` rejects unknown or duplicate mode names, an empty list, missing mode groups, or the member without `delivery_order`. Packaged vectors cover each refusal and a runner advertising only `steer`.
- A list containing only undeclared modes, including modes excluded by this member, is refused immediately as `delivery_unsupported` with the original array in `detail.delivery`, rather than waiting for a boundary it can never reach. The `steer` group does not require `run_ops`: a runner may steer turns it does not own.

## 0.3.7

- Name a third interrupted-run reason, `host_exited` (`run::interrupt_reasons::HOST_EXITED`): the harness host the runner drives exited while the runner itself stayed up. Consumers can match it without a runner's private string.

## 0.3.6

- A delivered steer reports the run's `active` or `finished` state, while `step` belongs only to `delivered.as`. Its stored-message `ref` is always present; `run_id` is present only when the runner declares `run_ops`. A caller without that run capability stops following a delivered `step` with no `run_id`, treating it as neither an error nor a reason to poll `run.status`. The same reply rule covers durable re-sends and `send.status`.
- A `delivery_order` send still waiting for its next possible boundary is a queued send. A runner declaring `retract` answers it `pending` with a `submission_id`, and `session.retract` withdraws it until delivery; a `guaranteed` steer already accepted into its run is not pending and is not withdrawn.
- A `delivery_order` list with no declared mode is refused immediately as `delivery_unsupported`, with the original array in `detail.delivery` and no writes. A list with one declared mode remains accepted; its undeclared members can never be chosen. `delivery-order.json` and the conformance case cover a `steer`-only list against a runner that declares `queue` but not `steer`.
- A stored `turn_system` entry keeps its bytes and role after a model switch. If the next provider cannot accept turn-scoped system messages, the runner sends the same text at user authority instead of dropping the entry or failing the request; `session.read` stays unchanged.
- Add a packaged `send.json` reply for a runner without `run_ops`, alongside the existing delivered-step reply with `run_id`.

## 0.3.5

- Add the independent `turn_directives` group (§9.9), opt-in frozen `directives: ["wake"]`, typed `SendRequest` accessors/builders, and optional `RunResult::directives` / `RunStatus::directives`. Existing sends and results without the group keep their wire bytes.
- Add the stable shared `directives::parse_trailing` parser: completed runs evaluate stored final text only, outside fenced code, without rewriting it. Packaged vectors cover durations, notes, `off` and each parser error. Consumers decode directive names and error reasons as open strings, so a later name or reason still decodes. Range policy (how soon or late a wake may be) stays with the caller.
- `send.status` never acts. Even where a re-send would re-drive a run paused for authentication, it reports `paused` with `pause_reason` and leaves resuming to the caller.
- Pin `run.tool_calls`' requesting-message `ordinal`; add optional `position: u32` and `result_ordinal: u64` plus builders. `position` is the call's 0-based index among its requesting message's tool-call blocks; `result_ordinal` is the `session.read` ordinal of the entry holding its result. From this release a runner declaring `run_tool_calls` MUST send `position` on every entry and `result_ordinal` on every finished call. Consumers match `(ordinal, position)` and locate stored results through result_ordinal, not model ids, list order or profile-specific layouts. Vectors cover both result layouts and repeated model ids across sibling positions.
- Add `interrupt_reason` with durable, open `reason` values on interrupted seals, known `runner_shutdown` and `runner_crash` constants, and result/status/head decode vectors. An interrupted run sealed without a recorded reason (for example, before the runner adopted the group) reports none; consumers never guess one.

## 0.3.4

- Add three independent describe groups, `max_steps`, `max_output_tokens` and `no_tools`, each requiring `session.send`, with matching `capabilities` constants. Callers send each member only after its group is declared; undeclared members retain the existing honour-or-refuse-by-name rule.
- CONTRACT §9.3.1 pins positive `stop_when.max_steps` (one model response per step, ending `max_steps` under §6's atomic hard-stop rule), positive per-response `generation.max_output_tokens` (frozen at admission, capped to the lesser of request and model maximum, never refused for a lower model maximum), and `tools: []` (no plan tools or host built-ins). Non-empty tool lists stay runner-specific; a plan session refuses any later `tools` member. Joining steers retain §9's first-differing-field refusal rule.
- Add `SendRequest::{max_steps,max_output_tokens,no_tools}` accessors that name malformed field paths and matching `with_*` builders. All values remain in `runner_params` at their existing wire positions; limit builders preserve sibling members. `no_tools()` returns `None` for absence, `Some(true)` for an empty list and `Some(false)` for a runner-specific non-empty list.
- Add packaged `run-parameters.json` and `run-parameters-describe.json` vectors, byte-for-byte raw-parameter round-trip and malformed-value tests. No existing wire shapes or vector JSON bytes change.

## 0.3.3

- `session.resolve` answers about the session's current lineage (CONTRACT §9.6). A tag whose part is in the current lineage resolves with that part's current `ordinal` and `mid`, including a part carried across a lineage change that kept its tag. A well-formed tag that no part of the current lineage carries is refused `unknown_tag`, terminal: one the session never assigned, and one assigned only under a lineage the session has left (a fork, a §14 shape-(a) drop, a retention expiry), since tags are never reused and a left lineage never returns. This replaces "a well-formed tag the session never assigned" in §9.6, in the §12.1 error table and on `errors::UNKNOWN_TAG`.
- Following a queued send (§9, §9.7). Once a send answered `pending` with a `submission_id` is delivered, a re-send of its `send_id` answers `delivered: {as: "step", ref}` (`ref` the stored row) with `run_id` the run it joined when it went in as a steer, `{as: "turn", ref}` with `run_id` the run it started when it started one, and `{as: "unknown"}` with `run_id` the run it was handed to, when known, when delivery cannot be confirmed. These answers are durable, §9's "final" rule made explicit for queued sends: a restart never turns them back into `pending` or absent, and `unknown` stays `unknown` even if delivery is later proven. A caller MUST NOT derive a run id from a `submission_id`, which is opaque; `run.status` on an id that names no run answers `{state: "unknown"}` (§6.1). `SendReply::submission_id` and `SendReply::delivered` say so.
- New `send_status` describe group (`capabilities::SEND_STATUS`, §3 and the new §9.8) with the op `send.status` (`ops::SEND_STATUS`), checked by `check_describe` for that op. New `send_status` module: `SendStatusRequest {send_id}`, strict (an unknown field is refused `invalid_params` naming it), and `SendStatusReply`, an alias of `SendReply`. It answers exactly what a re-send of that `send_id` would, from the runner's durable send record, writes nothing and dispatches nothing. A `send_id` the session has no record of for the caller is refused the new terminal code `unknown_send` (`errors::UNKNOWN_SEND`, in `errors::CODES`, not retryable). A caller sends it only to a runner declaring the group and otherwise re-sends the full body.
- Vectors: `send-status.json` (requests, refused requests, replies for `pending`, `step` before and after a restart, `turn` and `unknown`, and the `unknown_send` refusal) and `send-status-describe.json`, listed in `vectors::FILES` and `SHA256SUMS`; `errors.json` lists `unknown_send`, so its bytes and checksum change.
- §15 lists the new conformance cases: `tags_resolve_left_lineage_refused`, `delivery_order_steer_receipt_durable_across_restart`, `send_status_answers_as_a_resend_after_restart` and `send_status_unknown_send_refused`.
- Patch release: every addition is a new item (a module, constants, a group, an error code) or text; no existing type changes shape.

## 0.3.2

- CONTRACT §14 property 4 now names the two valid shapes of a seal around a written but never-dispatched tool call (a run cut after its step was durable but before the call's dispatch intent). (a) Drop the call, which rewrites a written message and so moves the session to a new lineage whose old reads are refused `lineage_changed` (the existing text). (b) Keep the call and append a never-executed result for it in the same durable commit as the run's `interrupted` terminal state, directly after the message carrying the call: the shape the hard-stop rule (§6) already gives every pending, undispatched call (`max_steps`, `stop_when`, `budget_exhausted`, `context_ceiling`), with no dispatch intent recorded. In shape (b) the call is never dispatched and never reported indeterminate, `run.tool_calls` (§6.3) lists it with `outcome: not_executed`, no later model request carries it without its result, the follow-up writes no second result, and every message already read keeps its bytes, so the lineage stays the same. Broca seals this way.
- Read stability (§4.2) stays absolute: its paragraph now says that a recovery that only appends, such as shape (b), keeps the lineage because no written byte changes. §15's `crash_at_StepRecorded` row lists both shapes.
- Text only: no type, constant or vector changes.

## 0.3.1

- Add per-run token usage, behind the new `run_usage` describe group (`capabilities::RUN_USAGE`, checked by `check_describe` for `run.result`, the op whose answer it extends): `run.result` and `run.status` gain an optional `usage` (`RunResult::usage`, `RunStatus::usage`, `with_usage`), standardizing the shape Broca already sends. A runner declaring the group includes it on every ended run's `run.result`, and on `run.status` once a step has completed (it may send `steps: 0` earlier). An absent `usage`, or an absent total, means not known, never zero.
- New `usage` module: `RunUsage` `{steps, unreported_steps, input_tokens?, cached_input_tokens?, cache_write_tokens?, output_tokens?, reasoning_tokens?, by_model}` and `ModelUsage` (one `by_model` entry per provider, model and variant, in order of first use), both non-exhaustive with constructors; `StepUsage` and `RunUsage::record_step` accumulate a run's steps as §6.4 counts them (a step reporting any field, even a zero, is reported; a total stays absent until a step reports it), and `RunUsage::consistent` checks that `by_model` sums to the totals. `by_model` decodes as empty when absent, which a zero-step usage may be, and always encodes. `run` re-exports the three types.
- Vectors: `run_status.usage_plain.json`, `run_status.usage_unreported_step.json`, `run_status.usage_model_switch.json` and `run_status.usage_reasoning.json`, `run.status` bodies from Broca's real run loop. Each `usage` round-trips byte for byte, its `by_model` sums to its totals, and absent totals decode as `None`. The Broca members `final_step_finish_reason` and `retries_used` beside it stay runner members the role does not type.
- The role's vectors move from the repository's top-level `test-vectors/llm-runner-v1/` into this crate's own `test-vectors/llm-runner-v1/` (same file names and bytes, plus the four above), so they ship in the package. The new `test-vectors` feature makes `vectors` a public module: `FILES` (every file as name and contents), `SHA256SUMS`, `README` and `file(name)`, so a runner pinning this crate by version tests against the same bytes without a checkout. A test checks every file on disk is listed with its exact bytes and its SHA-256. Anyone who read the old top-level path (by checkout or `include_str!`) reads `crates/cortexkit-role-llm-runner/test-vectors/llm-runner-v1/` or `vectors::file` instead.
- CONTRACT gains §6.4 for `run_usage` and its opening paragraph names the new vector location; §3 lists the group, §6 and §6.1 add `usage?` to the answers (§6.1 no longer lists `usage` as a runner member), and §15 lists the conformance case.
- Patch release: `RunResult` and `RunStatus` are already non-exhaustive, so the new field is additive; every other addition is a new item.

## 0.3.0

Breaking: compaction-provider 0.3.0 is async only, so the synchronous compaction interface is removed here too, and the deferred `RunState` change lands. What callers lose or must change:

- Removed `ops::COMPACTION_READY` (`compaction.ready`), `compaction::CompactionReady` and `compaction::ReadyOutcome`, the error code `errors::NOT_SESSION_COMPACTION_PROVIDER` (`not_session_compaction_provider`) and the provider code `errors::provider_codes::COMPACTION_WAIT_EXCEEDED` (`compaction_wait_exceeded`), with the `compaction-ready.json` and `fence.json` vectors. `compaction` keeps only the plan admission check (`PLAN_COMPACTION_ITEM`, `PLAN_COMPACTION_ITEM_FIELD`, `plan_compaction_provider`, `check_plan_compaction`).
- The `compaction` describe group is redefined: a runner listing it implements the runner half of compaction-provider 0.3.0 §3 (Setup, advance notices, views applied at step boundaries, rides, the ceiling, native markers), with `tags`, `context_ceiling` and the `context_ceiling_wait` status. Its ops are now `session.read` and `session.resolve`, and `check_describe` refuses an answer declaring `compaction` without `tags` (`problem_names::COMPACTION_WITHOUT_TAGS`, through `DescribeProblem::Undecodable`). CONTRACT §11.1 describes only that runner half; the per-step status, cursor, request fence, `WAIT` and `compaction.ready` rules are gone, and the `CompactionApplied` kill point now means a fetched view recorded in the write-ahead log.
- `SendRequest.prompt` is now `Option<String>`: a template send carries no `prompt` member, and every other send carries a non-empty one, which a runner checks with the new `SendRequest::check_prompt` (refusing either mismatch `invalid_params {field: "template"}`). A decode no longer fails on a missing `prompt`, so a runner that relied on that must call `check_prompt`; code reading `request.prompt` as a `String` reads `prompt_text()` or the `Option`. `SendRequest::new(prompt, send_id)` is unchanged; `SendRequest::template(send_id, &order)` builds a template send.
- `RunState` is `#[non_exhaustive]` and gains `BudgetExhausted` and `ContextCeiling`, so those states no longer decode as `RunState::Other`; a `match` needs a catch-all arm, and code matching `Other("budget_exhausted")` must match the variant. `RunState::as_str` is new; `is_budget_exhausted`, `is_context_ceiling` and `is_terminal` keep working. The top-level constants `RUN_STATE_BUDGET_EXHAUSTED` and `RUN_STATE_CONTEXT_CEILING` are removed; use `run::states::BUDGET_EXHAUSTED` and `run::states::CONTEXT_CEILING`.

Added, unchanged from the never-published 0.2.7 draft:


- Add runner-owned tags behind the new `tags` describe group (checked by `check_describe` for `session.read` and `session.resolve`). Every exposed part gets a stable key, and every text-bearing part (and any other the runner tags) a durable, never-reused `§N§` per session, assigned before storage or at the runner's first ingest of a part its host stored, recorded in the runner's ledger by (message, part); a fork continues after its parent's highest number. The model's output is stored byte for byte: a model-written `§N§` is never a tag, is never rewritten in storage, and stays as written in signed thinking, encrypted reasoning and tool-call arguments. Correcting one in the model's text is allowed only in the rendering, only where the provider's text part carries no provider state, and only if fixed at the runner's first render of that message.
- `ReadMessage` gains optional `tags: [{part, tag?}]` (`tags::PartTag`, non-exhaustive, `tagged`/`untagged`): one row per exposed part, so every part's stable key is readable, `tag` absent on untagged parts such as images. With `render_tags` on, the exact rendering is the tag, one space, then the text, on user text, a tool result's first text block and stateless assistant text only (`tags::render_text`, `RenderKind`, `RENDER_SEPARATOR`). It also gains the envelope members `role` (`read::role`) and `written_at` (Unix epoch milliseconds), and `delivered_as` (below). CONTRACT §4.2 now pins the read envelope: runner-neutral facts ride beside the opaque body, never only inside it. Message origins and attestation are not in this release.
- New op `session.resolve {tag, range?}` (`tags::ResolveRequest`, strict; `tags::CharRange` counts Unicode scalar values) answering `{tag, ordinal, mid, part, content, range?}` (`tags::ResolveReply`, lenient) with the part's exact stored content. A malformed tag or an out-of-range range is `invalid_params` naming `tag` or `range`; an unassigned tag is the new terminal `unknown_tag`.
- Tag rendering is the fetch plan's optional `render_tags` member (`tags::plan_render_tags`, `tags::check_plan`, `SendRequest::check_render_tags`); absent means off, and tags exist either way. A non-boolean, or `true` to a runner without `tags`, is refused `invalid_params {field: "plan.render_tags"}`.
- Add `delivery_order` behind the new group of that name: `delivery` on `session.send` is now a string or an ordered list, `delivery: [tool_result | turn_system | steer | idle_start]`, decoded once (`SendRequest` gains the field `delivery_order: Option<Vec<delivery_order::Mode>>`, set when the wire `delivery` is a list; read with the members as `SendRequest::delivery_order()`, written with `with_delivery_order`). At each boundary the first possible mode wins (`choose`, `Boundary`); a list without `idle_start` never starts a run. A string keeps its meaning: `queue` (or absent) = `[idle_start]`, `steer` = `[steer, idle_start]`, `interrupt` has no list form (`legacy_equivalent`). The overload fails closed: a runner without the group refuses a list `invalid_params {field: "delivery"}` (`SendRequest::check_delivery_order`), as an older runner's strict decode already does. The members `dedupe_key`, `expires_at_ms` and `template` are valid only beside a list (`delivery_order::MEMBERS`); on a group runner, any of them with a string or absent `delivery` is refused naming the member.
- `dedupe_key` replaces a queued, undelivered send from the same sender (`replaces`), which ends in the new terminal send state `superseded`; `expires_at_ms` drops an undelivered send as `expired` (`send::states`, `SendState::Superseded`/`Expired`, observed by re-sending the `send_id`). A delivered send is an entry carrying `delivered_as: {mode, call_key?}` (`EntryDelivery`); `tool_result` never changes the stored tool result.
- `turn_system` is possible only for a plan template's rendering under the sender, matching its JCS SHA-256 digest (`template_digest`), with typed slots validated by the runner (`render`, `Slot`: integer with min/max, percentage, tag, enum; no string slots). The plan's new `senders: [{module, idle_start?, templates?}]` (`PlanSender`, `Template`, `plan_senders`, `check_plan`) names the non-owner senders; on a `delivery_order` runner any other non-owner is refused `scope_owner_mismatch`, matched on the attested principal (`admit`), and a sender may list `idle_start` only with its plan grant.
- A template send carries `template: {id, digest, slots}` and no `prompt`, and has one text, the runner's rendering, whatever mode carries it (`SendRequest::template` builds one); a template beside a `prompt`, empty or not, or a send with neither a template nor a non-empty `prompt`, is refused naming `template` (`SendRequest::check_prompt`), and a template that cannot render (unknown id, digest mismatch, a missing, undeclared or invalid slot) refuses the send naming `template`. Only a rendered template that cannot go out as `turn_system` falls through, as the same text at user authority.
- `session.read` pins three runner obligations (CONTRACT §4.2): only finished messages are entries; an entry's bytes, part keys and tags never change under its lineage (a host change that reaches the model, or a crash recovery that drops or rewrites a written message, starts a new lineage; §14 property 4 now says so); and host records a runner writes for its own compaction bookkeeping are outside the transcript.
- Add `run.tool_calls` behind the new `run_tool_calls` group: a run's calls in dispatch order, `{tool_call_id, call_key?, name, args, outcome, result_bytes?, dispatched_to?, ordinal}` (`tool_calls::RunToolCall`; `outcome` open, `tool_calls::outcomes`), paged by count and bytes like `session.read` with an opaque `after`/`next` cursor and `limit`. `args` is never truncated; a single call too large for a frame is `reply_too_large` with the new `ReplyTooLargeSubject::ToolCall {run_id, tool_call_id, call_key?}`. An unknown run is `unknown_run`.
- Add the terminal refusal `context_ceiling` with `errors::ContextCeilingDetail {window, size, cause}` (non-exhaustive; causes `view_timeout`, `provider_unavailable`, `over_window` in `errors::context_ceiling_causes`, decoded open). A send that would start or join a run at or over the ceiling is refused and writes nothing; a run reaching it at a step boundary ends in the new non-error terminal state `context_ceiling` (`RunState::ContextCeiling`, `RunState::is_context_ceiling`, recognised by `is_terminal`), with the detail on `RunResult`/`RunStatus` (`context_ceiling`, new optional members) and pending calls given the never-executed result `tool_result_reasons::CONTEXT_CEILING`. A provider's own overflow still ends `error` with `context_overflow`.
- A run held at the ceiling waiting for a view stays `active` with the new waiting reason `context_ceiling_wait` (`RUN_REASON_CONTEXT_CEILING_WAIT`, `RunReason::ContextCeilingWait`, a new variant of the non-exhaustive enum) on `run.status`, `session.head` and its stream event, and `RunStatus` gains `deadline_ms` (`RunStatus::context_ceiling_wait`), so a watchdog does not take it for a stall. The wait ends in progress or in `context_ceiling`. It reuses the state-plus-`reason` shape `paused` uses, not `paused` itself, whose send rules differ.
- Define the `anthropic-messages` serializer profile (new module `serializer_profile`, CONTRACT §4.4): an entry body is the message as the client sent it in its Messages request, with `cache_control` removed only at top-level content blocks and in `text`, `image` and `document` blocks nested in a `tool_result` (`strip_cache_control`), never inside `tool_use` input or unknown blocks; the first-ingested form is served (`ingest`); compaction view bytes for such a session use this format. Vector `serializer-anthropic-messages.json`.
- Vectors `tags.json`, `tags-describe.json`, `delivery-order.json`, `delivery-order-describe.json`, `run-tool-calls.json`, `run-tool-calls-describe.json` and `context-ceiling.json`; `errors.json` lists the two new codes. CONTRACT gains §6.3, §9.6 and §9.7, and §1, §3, §4.2, §6, §6.1, §9, §10.1, §12, §13 and §15 list the new ops, groups, fields, states and codes.

## 0.2.6

- Add `session.adopt {send_id, plan}`, behind the new `adopt` describe group (checked by `check_describe` for `session.adopt`): the session's owner takes over a session the host started with a plan already applied, without a prompt. It binds the owner and frozen scope to the plan as owner admission does, so `session.baseline` answers the same afterwards; it starts no run, writes no message, and neither interrupts nor refuses a run in progress. The reply is `{owner_bound, applied_plan_digest, last_ordinal?}`.
- Admission reuses §9's owner rule; checks run caller (`scope_owner_mismatch`), then `send_id` (exact repeat replays the original reply; another plan is `send_id_reuse` naming `plan`), then drift: a plan other than the one applied at host start is the existing terminal `plan_drift {frozen, sent}`, whose fields are the two plans' digests (never plan content), with `frozen` absent when no plan was applied. Refusals write nothing. The same owner re-adopting with the same plan succeeds; adopt never takes over another owner's session.
- New `adopt` module: `AdoptRequest` (strict, so a request naming its session is refused) and `AdoptReply` (lenient), both non-exhaustive with constructors; `AdoptRequest::reused_field` and `check_drift`. `ops::SESSION_ADOPT`, `capabilities::ADOPT` and its `GROUPS` entry. `PlanDriftDetail` gains a `new(frozen, sent)` constructor. No new error code: `plan_drift`, `send_id_reuse` and `scope_owner_mismatch` already exist.
- Vectors: `adopt.json` covers a first adoption, an exact replay, `send_id_reuse`, `plan_drift`, adoption during a run, another owner's attempt and the same owner re-adopting; `adopt-describe.json` covers describe with and without the group. CONTRACT gains §9.5 for `session.adopt`; §1, §3, §9, §10.1, §12.1, §13 and §15 now list the op, its group and the codes it reuses.
- Patch release: every addition is a new item or a non-exhaustive type; `PlanDriftDetail` keeps its public fields and stays exhaustive.

## 0.2.5

- Add structured output to `session.send`, behind the new `structured_output` describe group (checked by `check_describe` for `session.send`). Modes `final_text` and `submit_tool`; the reserved `submit_result` tool; per-run JCS-frozen schemas; refusals at admission for an external `$ref`/`$dynamicRef` (runners never fetch), a schema over 65,536 JCS bytes, another `$schema` dialect, `final_text` with tools, and an admitted tool named `submit_result`. Validation is draft 2020-12 with `format` as an annotation and unknown keywords ignored. Enforcement and its reason are decided at admission from the runner's own per-family subset profile and fixed for the run.
- `structured_output` stays inside `SendRequest.runner_params` as `budget` does; read it with `SendRequest::structured_output()` (returns the offending field path) and write it with `with_structured_output(...)`. No typed field is added to `SendRequest`. New `StructuredOutput` and `structured_output::Mode` are non-exhaustive.
- `RunResult` and `RunStatus`, already non-exhaustive, gain an optional `structured_output` (`StructuredOutputResult`, non-exhaustive): enforcement and its closed-for-runners, open-for-consumers `reason`, validation, the value, `errors` bounded to 32 entries of 256 bytes with `errors_truncated`, and `skipped_siblings`. `well_formed()` names a broken member rule.
- Siblings of `submit_result` are never dispatched; they get the hard-stop never-executed result with the new `tool_result_reasons::SKIPPED_AFTER_SUBMIT`. `ReplyTooLargeSubject` (non-exhaustive) gains `Run { run_id }` for a run answer made too large by its value.
- Patch release: every new member sits in a non-exhaustive struct or rides in the existing flattened map. Adds a `serde_jcs` dependency.

## 0.2.4

- Add discovery-gated, durable, lower-only `budget.max_input_tokens`, budget/nudge closed-member checks, budget-exhaustion detail and cause constants, and the `budget_exhausted` result code. Cache reads/writes count at face value across all session episodes; missing usage fails closed.
- Keep `budget` in `SendRequest.runner_params` unchanged for existing consumers. The typed `budget()` accessor parses a copy and returns the offending field path on malformed data; `with_budget(...)` replaces that raw map entry, avoiding duplicate wire keys. No typed `SendRequest.budget` field is added.
- When a run ends at `max_steps`, `stop_when` or `budget_exhausted`, every tool call the model asked for that has not run gets a never-executed result, written in the same commit as the run's end state, so a transcript never holds a tool call without a result. The result uses the runner's own format; the contract adds no shared tool-result body.
- Keep states open. The exhaustive public `RunState` enum retains `Other(String)` for `budget_exhausted`; `RUN_STATE_BUDGET_EXHAUSTED`, `is_budget_exhausted()` and `is_terminal()` recognize it without breaking downstream exhaustive matches. At the next breaking release, make `RunState` non-exhaustive and promote this state to a variant.
- Patch release: new fields are confined to already non-exhaustive run reply structs with constructors; send budgets remain in the existing flattened map. New `Budget` and `BudgetExhaustedDetail` are non-exhaustive from introduction.

## 0.2.3

- Add the terminal `reply_too_large` refusal and safe size/subject detail. Bound whole-message and atomic model-view-group replies by the 4,194,304-byte transport frame limit; replies that exceed it are refused, never truncated.

## 0.2.2

- Add optional `run.cancel.at` (`now` by default, or `step_boundary`) and open-string constants. Pin discovery gating, whole-step/tool-batch completion, paused cancellation, escalation, idempotence, terminal races and unchanged owner-only admission.
- Add optional `RoleDescribe.run_cancel` metadata with a non-exhaustive `RunCancel` struct and builders. Refuse metadata without the group, empty lists, missing `now` and duplicates by name through the existing `DescribeProblem::Undecodable` variant, without adding variants that would break exhaustive downstream matches; unknown advertised values remain forward-compatible.
- Add cancellation and discovery vectors without changing existing vector bytes.
- `RunCancelRequest` gains the optional `at` field and becomes `#[non_exhaustive]`; build it with `RunCancelRequest::new(run_id)` and `.with_at(...)`. Strictly, adding a field to the exhaustive struct breaks struct-literal construction and exhaustive destructuring. A search of every CortexKit repository found no use of the type outside this crate, so this ships as a patch, and marking it non-exhaustive now keeps later fields additive.
- `DescribeProblem` becomes `#[non_exhaustive]` at the next breaking release; the four run_cancel refusals then get their own variants.

## 0.2.1

- Add `session_ref(harness, project_root, session)`, the session reference from contract section 14.1, so every module that keeps state for a runner session derives the same value. It is checked against `test-vectors/llm-runner-v1/session-ref.json`.

## 0.1.1

- Clarify that compaction conformance accepts runner-driven recovery or an interrupted run resumed by a sender's new send; runtime rules are unchanged.
