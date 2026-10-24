# Changelog

## 0.12.1

- Require llm-runner 0.6.3.
- Fix `attachments_default_read` and `attachments_tool_result`: they found
  the stored media message by searching the blocks page for the media's hex
  sha256, which a correct runner need not write anywhere in its own message
  schema, so both failed runners that store media inline. They now find that
  message through the default read's marker and its `mid`, and still compare
  its digest and part digests against independently computed ones, and its
  served bytes against `session.media_read`. The descriptors `media_ref`
  check is unchanged, since that shape is contract-defined.
- Add a fake that stores a sent upload and a tool-result image as a `data:`
  URL part with no digest in it: both read cases pass against it. Add a
  fault for each rule a blocks page must meet (markers served instead of the
  stored message, a wrong `digest`, wrong `part_digests`), each failing both
  read cases by name with its own reason.
- Add a check to `run_usage_counts_reported_steps` covering reasoning:
  a runner that bills `reasoning_tokens` on top of `output_tokens` reports
  more output than the scripted model did and fails by name. The case now
  runs twice, once with no step reporting reasoning and once with a step
  that does, so both the absent-total rule and the reasoning total are
  observable.
- Fix the test harness for faulty runners: `only_these_fail` checked an
  expected reason only on a failed case, so an expectation against a case
  that passed or ran inapplicable was never compared.

## 0.12.0

- Moves to subc-protocol 0.33; take subc-protocol 0.33, subc-transport 0.14, subc-control 0.33 and subc-client-rs 0.31 together.
- Breaking dependency update: `Reply::Error` and `SubscribeOutcome::Refused`
  contain subc-protocol 0.33's `ErrorBody`.
- No behaviour changes; existing test-vector bytes are unchanged.

## 0.11.2

- Require llm-runner 0.6.2. Gate `head_sender_is_callers_effective_grant`
  on the live `head_sender` describe group as well as the adapter-provided
  fixture for two caller identities. An absent group is inapplicable with a
  named reason.
- Make the reference fake declare `head_sender`; add a faulty runner that
  declares it but omits every `sender`, failing the case by name. Existing
  caller, owner and mode-grant faulty runners remain covered by name.

## 0.11.1

- Add an incarnation lifetime case with defaulted cut/re-create hooks and
  faulty runners that change identity at a cut or reuse it on re-creation.

- Require llm-runner 0.6.1. Add `Capability::SessionUsage` and cases for
  two-run sums and the first counted run, monotonic spend after a rewind,
  and unknown token fields remaining absent. Add a recorded model-switch
  case when the runner reports `head.model`.
- Add defaulted subject hooks for session-usage fixtures, host rewinds and
  recorded model switches; existing adapters compile unchanged and report
  unsupported observations as inapplicable. Faulty reference runners prove
  each case fails by name, including a wrong `since` and plan-derived model.
- Add `head_sender_is_callers_effective_grant`, with defaulted two-identity
  fixtures. Compare named A/B, unnamed and owner routes against the modes
  admitted from checked describe and the plan's flags. Faulty runners cover
  caller-independent grants, omitted owners and an ungranted `idle_start`.

## 0.11.0

A breaking release for `cortexkit-role-llm-runner` 0.6.0 and
`cortexkit-role-compaction-provider` 0.6.0, whose types this kit's public
API uses. Existing test subjects compile unchanged: the new hook has a
default.

- New subject hook `seed_untimed_tool_call`: put a finished call into a
  session's stored history the way history stored before the runner kept
  tool-call times holds it. Its default makes the new case inapplicable by
  name.
- New case `live_untimed_history_call_leaves_times_out`: after one run, the
  seeded call is replayed from start, and its `tool_started` and
  `tool_finished` must carry no `started_at` and no `finished_at`. A runner
  that invents a time fails it by name.
- `live_message_and_tool_events_durable` requires both times on the call the
  runner ran itself, the finish no earlier than the start, as the
  required fields did before.

## 0.10.6

- Exercise immutable output references, scanner samples, slot opt-in, snapshot
  limits, canonical digests, compaction/rewind and session isolation through
  runner routes, with a reference fake and named faulty runners.
- Use the runner-owned reference-slot type without a direct tool-provider
  dependency.
- Positive output-reference cases are N/A when the runner does not declare the group,
  rather than reporting a missing-capability skip for an undeclared group.
- The unsupported-plan refusal case binds non-advertisers on send, refresh
  and adopt, and is N/A for advertisers.
- Add the default-at-once `send_staged` subject hook and wait for it before
  releasing the first-possible-mode case's tool result. Check that a result
  sent before host staging cannot carry a subsequently staged send.

## 0.10.5

An additive release: per-case progress for runs that can be cut off before
they report. `run_suite` keeps its signature and report.

- `run_suite_with(subject, work_dir, SuiteOptions)`, beside `run_suite`.
  `SuiteOptions::new().with_progress(callback)` takes an
  `FnMut(&SuiteProgress) + Send`, called synchronously before each case
  starts and right after its outcome is known. `SuiteOptions` and
  `SuiteProgress` are `#[non_exhaustive]`.
- `SuiteProgress::CaseStarted { case, index, total }` and
  `SuiteProgress::CaseFinished { case, outcome, elapsed_ms }`. Every case
  gets both, in suite order, including a case skipped for a missing
  capability or inapplicable before it runs.
- `progress::stderr_lines()` writes `START <case>` and
  `END <case> <PASS|FAIL|SKIP|N/A> <ms>` lines to stderr, flushing after
  each. The outcome word is the one `SuiteReport::render` uses for the case.
  Nothing turns it on but passing it.

## 0.10.4

- Require `cortexkit-role-llm-runner` 0.5.4 and exercise `held_calls` when
  declared on the real `role.describe` reply. Existing adapters need no new
  enum match arms; undeclared `held_calls` is inapplicable by name and does
  not lower the runner's verdict.
- Add default fixture hooks for a held first reply and separately published
  late entries, plus `ScriptedLateEntry`. Cases check the waiting list,
  removal and continuation with a late result, `not_started` conversion,
  and conversion of an `expired` answer to a tool result before continuing.
- Add never-listed and stale-entry faulty runners, each rejected by exactly
  one named case.

## 0.10.3

Requires `cortexkit-role-llm-runner` 0.5.3. Runners can now verify the optional
`attachments` group over their real session-bound routes; callers gain checked
upload resumption, integrity, model admission, legacy-safe reads and digest
isolation guarantees before relying on that group.

- Capability-gated attachment cases cover a multi-frame lifecycle, lost
  append replies, restart loss, the inclusive 2 MiB cap, size/hash refusals,
  new-run and frozen-model input, unknown/open/gone/other-session ids,
  default markers versus blocks/descriptors and stored digests, nested tool
  media, read/upload admission, and committed-media lifetime.
- Defaulted `attachment_model_fields`, `attachment_tool_result`, and
  `restart_attachments` fixture hooks keep existing adapters source-compatible.
  An adapter without the group reports every case inapplicable/skipped by
  name. Plan-sender and controlled-clock cases also require their existing
  capabilities. Declaring attachments requires implementing its fixtures.
- The reference fake implements uploads and a persistent byte store separate
  from history references. Each attachment case has a named faulty fake and
  an independent passing control, including cross-session status and commit.

## 0.10.2

Requires `cortexkit-role-llm-runner` 0.5.2 and
`cortexkit-role-compaction-provider` 0.5.1. An additive release: it adds no
subject hook, so an existing adapter builds and runs unchanged. The new
cases reuse the plan-sender and held-model-step hooks, and a case whose
hook or group is missing is skipped or reported inapplicable by name.

- New capability `Capability::RunCause` for the role's new `run_cause`
  group.
- `run_cause` cases: the run a `queue` send started names that send's own
  `send_id` on `run_started`, never an id the runner made for its queue;
  every run a send started carries a non-empty `send_id`; and an
  interrupt's run names the interrupting send, never the interrupted one
  (inapplicable without `interrupt` or the held-step hook).
- Interrupt-grant cases (`delivery_order`, inapplicable without
  `interrupt`): a plan sender whose entry lacks `interrupt: true` is refused
  `invalid_params` naming `delivery` for the `interrupt` string, writing
  nothing; one holding the grant interrupts a held run, which ends
  `cancelled`, and its message starts the next run.
- A plan sender granted `idle_start` that sends the string `queue` (not a
  list) is admitted and starts a run. A runner that takes only lists from
  named senders fails this case by name.
- The display `sender` case: a plan sender's message has a display row
  naming the sender's module, and the default `session.read` page carries
  no `sender` member. The projection case takes `sender` from the served
  row, as it takes `model` and `client_message_id`.
- Provider-set rewrite-ride waits (compaction-provider 0.5.1 §4 and §8,
  llm-runner §6 `compaction_wait`), each case inapplicable unless the
  runner sends `ride_wait_max_ms` in its Setup request:
  `ride_wait_provider_wait_honoured` (Setup answers `ride_wait_ms: 4000`
  and the provider's view for a `ttl_expired` ride, answered after 3 s,
  applies at the ride's step), `ride_wait_slow_answer_reported` (a ride
  answered after 2.5 s reads `compaction_wait` while it waits),
  `ride_wait_fast_answer_not_reported` (one answered within 200 ms never
  does) and `ride_wait_above_cap_fails_setup` (a `ride_wait_ms` above the
  runner's cap fails Setup, naming `ride_wait_ms`). New
  `CompactionSetup::ReadyWithRideWait` (the enum is non-exhaustive; an
  adapter's `_` arm answers it as a failed Setup until it handles it), new
  `ScriptedRide`, and two subject hooks with defaults that make the cases
  inapplicable: `observed_ride_wait_max_ms` and `script_next_ride`.
- Fix: `media_descriptors_pairing` no longer reports "descriptors lost
  non-payload pairing field id" when a runner correctly describes a call's
  arguments as an `args_ref` whose inline arguments hold an `id` or `name`
  member, as tool parameters often do. The pairing check stops at a value
  described as a `*_ref`, which stands for the whole payload; pairing
  fields beside the ref, on the part that holds it, are still checked. The
  case's call now carries such arguments, over the inline limit.
- New narrowings: the owner's authority over a run a named sender started
  is not observable through the route (CONTRACT §9), and `run_cause`'s
  omission for a host turn on an adopted session is checked against the
  vectors only.

## 0.10.1

Requires `cortexkit-role-llm-runner` 0.5.1. An additive release: every new
subject hook has a default, and a case whose hook is missing is reported
inapplicable by name, so an existing adapter builds and runs unchanged.

- New capabilities `Capability::HistoryDisplay`, `Capability::LiveEvents`
  and `Capability::ClientMessageId` for the role's new groups.
- Display cases (`history_display`): a served row equals the serializer
  profile's projection of the same entry's raw body, every assistant row
  names its model, `limits` cut on a UTF-8 boundary and set the flags,
  absent `limits` serve whole content, and display pages carry finished
  messages only.
- `client_message_id` cases: the id appears on the user message's display
  row and never in the default `session.read`; a re-send under the same
  `send_id` with another id, or without it, is refused `send_id_reuse`
  naming `client_message_id`.
- Live-event cases (`live_events`): message and tool lifecycle events that
  replay identically and resume from a cursor, run events, the attach
  replay of the step in flight, and `seq` rising across both lanes.
- Live `cache_ttl` cases (compaction-provider §8.1): an entry for another
  model is ignored, a configured `ms: null` never declares `ttl_expired`
  even with an observed lifetime, and after a missed `model_change` ride
  the runner falls through to the observed lifetime, never the old model's
  entry.
- New subject hooks, each with a default: `serializer_profile`,
  `hold_next_model_stream`, `advance_prompt_cache_clock`, `switch_model` and
  `set_observed_cache_lifetime`. New `CompactionSetup::ReadyWithCacheTtl`
  answers Setup with a `cache_ttl` entry; the enum is non-exhaustive, and an
  adapter's `_` arm answers it as a failed Setup until it handles it.

## 0.10.0

Requires `cortexkit-role-llm-runner` 0.5.0 and
`cortexkit-role-compaction-provider` 0.5.0. A breaking release.

- Breaking: `session.head` answers must carry `first_ordinal` (the role's
  `HeadMeta::lineage_consistent` now requires it), and the shared full read
  starts at the lineage's `first_ordinal` instead of ordinal 0.
- Breaking: `CompactionCall` gains `lineage_id`, the lineage the Setup body
  named, and `CompactionObservation` gains `notices`, every advance notice
  the runner published as its bus headers. Adapters that build them with
  struct literals add the fields.
- New `Capability::HostHistory` for the role's `host_history` group.
- New subject hooks, each defaulting to an error that makes the cases
  needing it inapplicable by name: `publish_compaction_view` (with the new
  `ScriptedView` and `ScriptedRange`), `revert_session`,
  `seed_host_history` and `seed_session_history`.
- Cut cases, gated on `compaction`: `cut_starts_a_new_lineage`,
  `old_lineage_reads_refused_after_a_cut` (`after_mid` included),
  `read_below_first_ordinal_refused` (with `detail.first_ordinal`),
  `ordinals_never_renumber_across_a_cut`, `setup_once_per_lineage`,
  `view_ranges_start_at_first_ordinal` (a view tiling from 0 refused
  `ranges_start`) and `cut_lineage_notice_descends_from`.
- Cut cases on the head, the notices and reverts: `head_names_the_cut`,
  `notice_carries_first_ordinal` (`AdvanceNotice::first_ordinal`),
  `revert_below_the_cut_restarts_at_the_previous_cut` (with two cuts, and
  below every cut) and `revert_below_a_cut_before_adoption_has_no_descent`.
  `view_ranges_start_at_first_ordinal` also refuses a cut lineage's view
  that leads with a keep range. `seed_host_history` takes `cut`: the host's
  history ends in a cut made before adoption.
- `checkpoint_on_descending_setup_refused` (gated on `tag_checkpoint`),
  `cut_setup_leaves_reads_alone` (inapplicable unless
  `holds_reads_during_setup` says the runner ever holds reads, and needing
  the new `hold_next_compaction_setup`, `await_compaction_setup_held` and
  `release_compaction_setup` hooks) and
  `cut_numbering_continues_from_the_old_lineage` (gated on `tags`).
- `history_version_unchanged_by_a_cut`, `history_version_rises_at_a_revert`,
  `history_from_ordinal_and_before_ordinal`,
  `history_unknown_ordinal_refused` and
  `history_before_lowest_ordinal_reaches_unnumbered_entries`.
- History cases, gated on `host_history`:
  `history_pages_back_to_the_first_message`, `history_mid_matches_read`
  (bytes, ordinal and tags too), `history_no_tags_below_first_ordinal`,
  `history_ordinal_kept_below_the_cut`, `history_cursor_survives_a_cut`,
  `history_never_serves_a_reverted_branch`,
  `history_cursor_expired_after_a_revert_past_it`, `history_boundary_order`
  and `history_before_mid_continues_below_the_window`.
- `per_step_cost_scales_with_new_messages`, opt-in through
  `seed_session_history`: with 1,000 and 150,000 seeded messages behind a
  cut that leaves the same window, the median of nine timed exchanges on
  the large session is at most 2.0 times the small one's (medians under
  5 ms count as 5 ms). Without the hook it is inapplicable, never passed.
- The suite's own reference fake implements every new rule, including a
  variant whose every session holds host history without ordinals, and a
  faulty fake fails each new case by name.

## 0.9.2

Requires `cortexkit-role-llm-runner` 0.4.1.

- The media cases' PNG now has a valid IDAT checksum, and the frame-budget
  case pads the image with a zero-filled private `ckPd` chunk inside the PNG
  (before IEND, same 2,400,000-byte total) instead of zeros after IEND, so a
  host that decodes tool-result images (OpenCode) passes them. A unit test
  checks every scripted PNG's chunk CRCs and its IHDR to IEND structure.
- `session_change` cases, gated on the group: `session_refresh_applies_from_next_run`,
  `session_refresh_waits_for_its_rung`, `session_refresh_never_changes_a_running_run`,
  `session_refresh_repeat_same_answer`, `session_refresh_generation_stale_refused`,
  `session_refresh_tool_outside_host_set_refused` and
  `flush_prefix_applies_pending`. They need the new
  `LlmRunnerSubject::refresh_plan` hook (`RefreshVariant`, `RefreshPlan`),
  defaulted to `None`, and `model_requests`.
- `max_steps_final_step_completes` and `max_steps_submit_result_completes`:
  a step that reaches `max_steps` without a tool call leading to another
  step ends the run `completed`, not `max_steps`.
- `RunnerRoute` documents that adapters decode every reply body with
  `wire::read_reply` and return the answer inside; the suite sees no raw
  reply bytes, so it has no case for a bare reply.

## 0.9.1

`media_descriptors_frame_budget` and `media_descriptors_stable_across_pages`
now fit runners whose module tool traffic crosses the subconscious daemon.
The daemon never delivers a module-to-client frame over 4,194,304 bytes
(header included), so a scripted tool provider could not return either case's
result and the run never ended; some client SDKs also refuse to send a
request of that size. The rules both cases check are unchanged.

- `media_descriptors_frame_budget`: the image result is 2,400,000 decoded
  bytes (3,200,000 base64) instead of 3,200,000. The single 4,194,304-byte
  `oversize` argument is now two sibling calls in the same step, each with a
  2,200,000-byte `oversize` argument: no one request exceeds the frame, and
  the assistant message holding both is still over 4 MiB. The case checks
  that both calls are described as `args_ref`s in that one message, that an
  inline read of it is still refused `reply_too_large`, and that the
  mixed-size fallback still describes parts `[1, 3]`.
- `media_descriptors_stable_across_pages`: the multipart result is 14 parts
  of 256,000 bytes (3,584,000) instead of 20 (5,120,000). Still over the
  descriptor message budget (3,145,728), it now expects exactly 2 described
  parts instead of 8, and still the same selection on every page.
- A unit test checks that every payload a scripted tool provider receives or
  returns in the media cases, as JSON plus an envelope allowance, stays under
  the daemon's 4,194,304-byte frame.
- `tags_on_every_read_entry` and `tags_count_only_parts_rendered_with_a_number`
  now accept embedded layouts such as `opencode-aisdk`, where a call's result
  is stored in the requesting entry's tool part, so that entry carries two
  numbers: the model's text's and its tool result's (a tool result counts, a
  tool call never does). Both cases used to require exactly one number per
  entry. The number of a marker is now the number of the part holding it,
  and the counted-parts case finds the part holding the call's result from
  what the runner serves (the call's `result_ordinal` where `run_tool_calls`
  is declared, otherwise the entry holding the result's marker), requires
  that part to carry a number wherever it is, and still requires reasoning
  and a part holding only a tool call to carry none.

## 0.9.0

Breaking. Requires llm-runner 0.4.0 and compaction-provider 0.4.0, whose tag
numbers are computed from the transcript and are no longer a never-reused
ledger. The tags cases are rewritten for that, and three are renamed because
their old names state the rule that no longer holds.

- Renamed and rewritten: `tags_never_reused_across_runs` is now
  `tags_numbers_stable_within_a_lineage` (numbers consecutive and kept within
  a lineage, changed only with a lineage change);
  `tags_model_written_tags_stored_verbatim` is now
  `tags_model_written_tags_cleaned_on_exposure` (cleaned on every exposure for
  tag-rendering runs, byte for byte for untagged runs, raw for a
  structured-output run's final text where `structured_output` is declared);
  `tags_resolve_left_lineage_refused` is now
  `tags_resolve_answers_the_current_lineage` (every number the new lineage
  shows resolves there; numbers may repeat across lineages);
  `tags_exist_whether_or_not_rendered` is now
  `tags_only_on_tag_rendering_sessions`, and
  `tags_resolve_returns_stored_content` is now
  `tags_resolve_returns_exposed_content`.
- Every tags case opens its sessions with `render_tags: true` through
  `send_fields_with_plan`, since numbering applies only to such sessions. The
  crash scenario's session B does so on a tags runner.
- New cases: `tags_numbered_in_rendered_order`,
  `tags_renumbering_waits_for_the_step_boundary`,
  `tags_count_only_parts_rendered_with_a_number`,
  `tags_setup_checkpoint_anchors_at_its_cut`,
  `tags_fork_of_a_managed_session_refused` (inapplicable unless the adapter
  can create a fork the runner can detect),
  `delivery_order_template_part_gone_refused` and `adopt_host_turn_is_a_run`
  (inapplicable unless the adapter can start a host turn).
- `compaction_setup_route_session_matches_setup` also checks that a Setup
  refused `setup_identity_mismatch` ends the run `error` with that code (now a
  typed compaction-provider code), with no model request.
- `CompactionSetup` is now `#[non_exhaustive]`: an adapter's `match` on it
  needs a `_` arm.
- Subject API: `Capability::TagCheckpoint` and
  `Capability::RenderedModelInput`; `ModelRequestObservation::rendered` with
  `with_rendered`; defaulted hooks `image_tool_result(marker)` and
  `fork_session(handle, parent, fork)`; `CompactionSetup::ReadyWithCheckpoint`.
  `structured_output_frozen_across_restart` ignores `rendered` when comparing
  requests.
- The reference fake numbers, cleans and renders through the role crate's
  shared functions, renders a steer stored during a held call after the
  call's result, and moves to a new lineage at the next step boundary when
  that renumbers an exposed part. Each new or rewritten case has a defect
  that fails it by name in the suite's own tests.
- Add descriptor-read cases for decoded media hashes, nested
  media, frame-sized arguments, UTF-8 heads, inline compatibility, full-inline
  message/part identities, pairing preservation, strict digest keys, and stable
  per-message fallback across page caps. Adapter media fixtures gate these
  cases; each supported rule has a named defect-fake check.

- Check lost marker-key fallback, in-process save seams,
  standalone refusal handoff only before first service, and command cutoffs
  on each side of forwarding. Adapter-only situations are named inapplicable
  when unsupported; defect fakes exercise the corresponding failures by name.
- The frame-budget case also checks mixed-size fallback selection: largest
  payload first, with the earlier of two tied parts selected at the cut.
  A smallest-first defect fake fails this case by name.
- Preserve the 0.8.11 route-before-prepare fix for the crash-cut command case;
  adapters may derive session references from the runner bound by the route.

## 0.8.11

- The crash-cut user-command case (`user_commands_outcome_unknown`) now calls
  `prepare_user_commands` after spawning the runner and opening the session's
  route, as every other user-command case does. Before, it called the hook
  before the runner existed, so an adapter that computes the session reference
  from the runner instance the route bound failed the case before it reached
  the runner. `prepare_user_commands` documents this order, and the reference
  fake now refuses a call made before the session's route is open.

## 0.8.10

- Require llm-runner 0.3.10. Add a crash-cut user-command case proving that a
  forwarded invocation whose reply was lost is stored as `outcome_unknown`,
  replayed by invocation and status reads, and never forwarded again. The
  reference runner persists its reservation before forwarding; faulty runners
  that forward again or report `failed` fail this case by name.

## 0.8.9

- Require `cortexkit-role-compaction-provider` 0.3.3. Add
  `compaction_setup_route_session_matches_setup`, which rejects a runner's
  single-session provider route when its bind session differs from Setup's
  session, while accepting shared project routes with an empty bind session
  and generic `runner` harness. The scripted provider observation now carries
  the Setup body identity and the actual incoming route bind identity
  separately. A faulty reference runner binding another session fails the
  case by name.

## 0.8.8

- Require llm-runner 0.3.9. Add live-route user-command checks for history and
  model-input isolation, once-only forwarding, no-person sessions, collisions,
  ordered progress, owner-only relay and unsafe host menus. Adapters install
  scripted providers and observe provider/model/host boundaries independently.
- Add faithful and deliberately faulty runner self-tests, failing by case name.

## 0.8.7

- Accept an unreadable requesting-message ordinal while a tool call is open in `run_tool_calls_result_ordinal_tracks_storage`: embedded call/result messages are not finished until the tool returns. If the message is already readable, it must still carry the call's arguments; open calls still require position 0 and no `result_ordinal`.
- After release, require the requesting message to exist and carry the arguments, as well as checking that `result_ordinal` locates the stored result. Self-tests cover an embedded layout with equal call and result ordinals, a faulty runner that loses the finished requesting message's arguments, and an early-readable placeholder that gains the arguments only after release.

## 0.8.6

- Require llm-runner 0.3.8. Add `delivery_order_declared_modes_respected`, gated on the live describe answer advertising `delivery_order_modes`. It tries every non-empty ordered list of distinct modes: lists containing only undeclared modes must be refused `delivery_unsupported` with the original `detail.delivery` and no transcript writes; lists containing a listed mode must be accepted.
- Mode-specific delivery scenarios are inapplicable when the describe answer excludes the modes they need. The fake can advertise only `steer`; a fake that ignores the member fails the new case by name in `ignoring_delivery_order_modes_fails_by_name`.

## 0.8.5

- Require llm-runner 0.3.6.
- Add `steer_without_run_ops_reports_state_and_step`, which runs only when the runner does not declare `run_ops`. A scripted steer into an active turn must answer with state `active` or `finished`, `delivered.as: "step"`, a non-empty `ref` and no `run_id`. `step` is never a state value.
- Add `delivery_order_retract_pending_tool_result`, gated on `retract` and `delivery_order`: while a tool call holds the run before its result, a `[tool_result]` send answers pending with a `submission_id`, retract answers `retracted` and then `not_pending`, and releasing the call does not deliver the send. The fake now keeps this withdrawn submission out of later delivery boundaries.
- Add `delivery_order_undeclared_mode_refused`, gated on `delivery_order`, `queue`, a held tool call and the absence of `steer`. A `[steer]`-only list must be refused `delivery_unsupported` with its exact array in `detail.delivery`; `[steer, idle_start]` is accepted because `queue` declares `idle_start`, but waits for the run to end and then delivers only as `idle_start`. Cases that require steer delivery also require the `steer` group; template fallback is inapplicable if neither `steer` nor `turn_system` is available. A fake defect that queues the all-undeclared list fails this case by name in `queuing_a_list_with_no_declared_mode_fails_by_name`.
- Keep the durable re-send and `send.status` cases gated on `run_ops`; both still require `run_id` for their delivered `step` answers. The fake defect that returns `state: "step"` fails the no-`run_ops` case by name in `a_steer_reply_with_state_step_fails_only_the_no_run_ops_case`; the fake defect that omits `run_id` while declaring `run_ops` fails the re-send and status cases by name in `a_delivered_steer_without_its_run_while_declaring_run_ops_fails_both_follow_cases`. The fake does not simulate provider switches, so the stored `turn_system` rendering rule has no separate case.

## 0.8.4

- Require llm-runner 0.3.5 and add `Capability::TurnDirectives` and `Capability::InterruptReason` to the non-exhaustive enum.
- Add seven named turn-directive cases, gated on the new group with named skips: each completed wake form appears on result and, where served, status without text edits; max_steps, cancelled and unrequested runs report none; unknown/non-array/duplicate names are refused. Each case has its own deliberately broken fake runner, which fails that case and no other.
- Add requesting-message sibling-position and open/finished result-ordinal cases under `run_tool_calls`; swapped positions and a premature result_ordinal each fail by name.
- Add a graceful handle-stop/restart case for `interrupt_reason`, checking runner_shutdown on result/head and on status where served. The fake distinguishes an orderly handle drop from a crash-cut handle and records the reason durably; a fake that omits the shutdown reason fails the `interrupt_reason` case, named in the report.
- Additive patch: no new adapter hooks, and callers not using new groups retain their wire shapes. `run.tool_calls`' new `position` and `result_ordinal` are optional on the wire, so older runners still decode, while the contract now requires a runner declaring `run_tool_calls` to send them.

## 0.8.3

- Require llm-runner 0.3.4. Add the independent `Capability::{MaxSteps,MaxOutputTokens,NoTools}` variants to the already non-exhaustive enum.
- Add four named, discovery-gated cases: `max_steps_seals_pending_calls` (two responses, pending siblings sealed at the terminal without dispatch), `max_output_tokens_caps_response` (positive dispatched maximum and single-response usage at or below the requested cap, also admitting a request above the model maximum), `no_tools_offered_to_model` (an unsuppressed control offers tools, the suppressed request offers none), and `no_tools_later_plan_send_refused` (empty and non-empty later lists both refused naming `tools` without writes). Missing groups are named skips; the output case also requires `run_usage`, and the late-tools case is inapplicable by name when the adapter cannot create a fetch-plan session.
- `ModelRequestObservation` gains optional `max_output_tokens` and a builder, recorded from the dispatched request. `model_requests` is now required when declaring `no_tools` or `max_output_tokens` as well as `structured_output`. In output-cap cases the scripted provider honours the maximum it actually receives and reports the truncated response's actual usage.
- Add five fake defects, each failing only its named case: ignored step limit and unsealed pending calls fail `max_steps_seals_pending_calls`; ignored output cap fails `max_output_tokens_caps_response`; offered tools despite an empty list fails `no_tools_offered_to_model`; ignored later-plan tools fails `no_tools_later_plan_send_refused`. The fake stores limits per run, counts model responses, caps its whitespace-tokenized response at its model maximum, and seals pending calls in the terminal record.
- Additive patch: no role wire bytes change and existing adapters need no new hooks unless they declare the new groups.

## 0.8.2

- Require cortexkit-role-llm-runner 0.3.3. `Capability` gains `SendStatus` (the new `send_status` group), listed last in `Capability::GROUPS`; `Capability` is already `#[non_exhaustive]`, so this is additive.
- New `tags_resolve_left_lineage_refused`, gated on `tags`, `queue`, `transcript_reads`, `dispatch_attribution` and the `StepRecorded` kill point. It reads the `crash_at_StepRecorded` scenario: after a seal that drops the never-sent call or its step onto a new lineage (§14 shape (a)), every tag number below the new lineage's highest that no part read from it carries (at most 64, the dropped part's tag among them, since the kill comes before any read of the cut step) must be refused `unknown_tag`, and the cut run's prompt must resolve with its ordinal and `mid` in the new lineage. It is reported inapplicable, by name, when the runner resumes the run or keeps the call, or when the seal dropped no tagged part.
- New `delivery_order_steer_receipt_durable_across_restart`, gated on `delivery_order`, `queue`, `transcript_reads` and held tool calls: on its own state root, a `[steer]` list sent while a run waits on a held call must answer `pending` with a `submission_id`, be delivered once as a steer when the call is released, and its re-send must answer `delivered: {as: "step", ref}` with `run_id` the run it joined, before a harness restart (no kill) and unchanged after it.
- New `send_status_answers_as_a_resend_after_restart` (the same, through `send.status`, which must answer exactly what a re-send answers before the restart and `step` with the joined run's `run_id`, never `pending`, after it) and `send_status_unknown_send_refused` (`send.status` of a settled send equals its re-send's answer, an unrecorded `send_id` is refused `unknown_send`, an unknown field is refused `invalid_params` naming it). Both are reported as named skips without `send_status`, and send `send.status` only after the live `role.describe` declares the group.
- The restart cases run their own runner and restart it with `Harness::restart`, without a kill, so they add nothing to the run's kill ledger.
- The reference fake answers a delivered queued send with the run that carried it and how (`step` with the row, or `turn` with the run), serves `send.status` when it declares the group, and keeps a dropped step's messages only for a new defect that resolves their tags. Three new defects fail their cases by name in the suite's own tests: `send.status` answering `pending` after a restart for a steer delivered before it fails `send_status_answers_as_a_resend_after_restart`; a delivered steer answered without `run_id`, on a re-send and on `send.status`, fails both `delivery_order_steer_receipt_durable_across_restart` and `send_status_answers_as_a_resend_after_restart`, each naming the run the steer joined; and resolving a left lineage's tag with its old content fails `tags_resolve_left_lineage_refused`.

## 0.8.1

- `crash_at_StepRecorded` accepts both seal shapes that llm-runner 0.3.2 (CONTRACT §14, property 4) names for the never-sent call, and fails anything else, naming the shape it was closest to (`closest to shape (a)` when the cut-run transcript has no attribution entry for the call, `closest to shape (b)` when it has one) and why:
  - (a) the call is absent from the cut-run and follow-up transcripts, the session is on a new lineage, and a read naming the old lineage is refused `lineage_changed` (the refusal is new: 0.8.0 checked only that the lineage moved);
  - (b) the lineage is unchanged, every message of session B read before the kill reads back identical, the call is kept and not indeterminate, the message right after the one carrying it holds its never-executed result once the run has ended `interrupted`, and the owner's follow-up leaves that transcript unchanged and writes no message carrying the call.
- Narrowed: 0.8.0 also passed a kept call whose result was written only when the owner's next send arrived. That result is in a later commit than the run's end, and the transcript read between the two holds a dangling call, so the case now fails it.
- The reference fake's keep-the-call seal writes the never-executed result in the record that ends the run. Two new defects each fail the case by name in the suite's own tests: writing the result only at the owner's next send, and changing the bytes of a message read before the kill while keeping the lineage. The existing defect that never writes the result fails it with the shape (b) reason.
- Still requires cortexkit-role-llm-runner 0.3 (0.3.1 or later); nothing in this release needs a newer type.

## 0.8.0

- Require cortexkit-role-llm-runner 0.3.1 and add `run_usage_counts_reported_steps`, run only when the subject declares the new `run_usage` group (otherwise reported as skipped, by name). After a two-step run whose scripted model reports input, cache-read, cache-write and output counts but no reasoning, `run.result` must carry `usage` with `steps: 2`, no unreported step, exactly the scripted totals mapped to `input_tokens`, `cached_input_tokens`, `cache_write_tokens` and `output_tokens`, a `by_model` that sums to them (`RunUsage::consistent`), and `reasoning_tokens` absent rather than `0`. Where the subject declares `run_status`, the ended run's `run.status` must carry the same `usage`. The case never reads `run.status` before the first step, where `usage` may be present or absent.
- Breaking: `Capability` gains `RunUsage` and is now `#[non_exhaustive]`, so a `match` on it outside this crate needs a wildcard (`_`) arm; later groups then arrive without a breaking release. `Capability::GROUPS` lists `RunUsage` after `RunToolCalls`, in the order of the role's `capabilities::GROUPS`.
- The reference fake reports usage when it declares `run_usage`; its new defect, reporting an absent `reasoning_tokens` as `0`, fails the case by name in the suite's own tests.

## 0.7.1

- `run_tool_calls_listed_in_dispatch_order` no longer gives two sibling tool calls in one step the same model id. A real provider never repeats an id within one response, and a host that keys tool parts by id (OpenCode does) stored the two siblings as one part, so the case failed a correct runner. Siblings now have distinct ids that sort against dispatch order, and the next step's call reuses the first id, so ordering or identifying calls by model id still fails.

## 0.7.0

- Requires cortexkit-role-llm-runner 0.3.0 and cortexkit-role-compaction-provider 0.3.0, which is async only. Removed with the synchronous compaction interface, with what callers lose:
  - the cases `compaction_wait_cap`, `compaction_late_or_stale_answers_discarded`, `compaction_step_timeout_uses_last_view`, `compaction_one_view_per_step` and `compaction_fence_crash_replays_model_view` (the last cut a step answer at `CompactionApplied`);
  - the types `CompactionScript`, `CompactionStep`, `CompactionAnswer` and `CompactionCallKind`, and `CompactionCall`'s `kind`, `issued_at_ms` and `deadline_ms`;
  - the `LlmRunnerSubject` hooks `answer_compaction` and `advance_compaction_clock`; `install_compaction_script(session, CompactionScript)` is replaced by `install_compaction_setup(session, CompactionSetup)`.
  - `SendRequest.prompt` is `Option<String>` in llm-runner 0.3.0: template sends now carry no `prompt`, and `delivery_order_template_renders_as_system` also checks that a template send carrying `"prompt": ""` is refused naming `template`.
  Kept, Setup-only: `compaction_setup_durable_once`, `compaction_unavailable_distinct_from_refuse` (its step-`REFUSE` half removed) and `model_view_half_open_ranges_and_travel` (now Setup's head insertion and the held request's canonical view; the summary half needed a step answer). A NARROWINGS entry says the runner half of compaction-provider 0.3.0 §3 has no live case yet.
- `RunState` is now non-exhaustive in llm-runner 0.3.0 with `BudgetExhausted` and `ContextCeiling` variants; every match in this crate keeps a catch-all arm.
- Add cases for llm-runner's new groups, each run only when the subject declares the group (otherwise reported as skipped, by name, with no op of the group sent):
  - `tags` (seven cases): part rows and tags on every entry, never reused across runs, present whether or not rendered, `session.resolve` content, ranges and refusals, model-written `§N§` stored and dispatched as written;
  - `run_tool_calls` (three cases): dispatch order through `next` paging, `unknown_run` and field refusals, `indeterminate` until a held call's result;
  - `delivery_order` (eight cases): a list without `idle_start` never starts a run, the first possible mode wins, dedupe supersedes, expiry, `template`/`dedupe_key`/`expires_at_ms` refused by name without a list in `delivery`, plan-named senders and the `idle_start` grant, a template rendered once for whichever mode carries it, and an out-of-range slot refused.
- Add `delivery_order_list_refused_without_group`, run only when the subject does not declare `delivery_order`: a list in `delivery` is refused naming `delivery` and writes nothing, the refusal that makes the overload of `delivery` safe.
- Add `run_context_ceiling_wait_is_not_a_stall` (harness capability `HoldContextCeiling`) and `read_stable_across_host_metadata_change` (harness capability `HostMetadataChange`).
- `crash_at_StepRecorded` now gives session B one finished run before the kill and fails a recovery that drops the never-sent call or its step without moving B to a new lineage.
- Breaking: `Capability` gains `Tags`, `RunToolCalls`, `DeliveryOrder`, `HoldContextCeiling` and `HostMetadataChange`, so an exhaustive `match` needs the new arms.
- `LlmRunnerSubject` gains defaulted `send_fields_with_plan`, `plan_sender`, `turn_system_supported`, `hold_context_ceiling`, `end_context_ceiling_wait` and `change_host_metadata` hooks; the new `PlanSenderIdentity` is non-exhaustive with a constructor.
- The reference fake's defects (tag reuse, rewritten model tags, reversed tool calls, runs started without `idle_start`, unchecked template slots, unnamed senders admitted, dedupe and expiry ignored, last possible mode, a list accepted without the group, list members accepted without a list, drop recovery keeping its lineage, reads serving live host metadata) each fail their named cases in the suite's own tests.

## 0.6.0

- Require cortexkit-role-llm-runner 0.2.6 and add `session.adopt` cases, run only when the subject declares `adopt` (otherwise each is reported as skipped, by name, and no `session.adopt` is sent); each case also refuses to send one unless the live `role.describe` declares `adopt`. They check that:
  - a first adoption, with no message yet and after the person's first turn, answers `owner_bound` with the transcript's last ordinal, writes nothing and starts no run, and `session.baseline` then answers only the adopting owner;
  - an exact repeat replays the original reply; the same `send_id` with another plan is refused `send_id_reuse` naming `plan`;
  - a plan other than the host's is refused `plan_drift` with distinct `frozen` and `sent` identities and no content, binding no owner and recording no `send_id`; a session the host never started answers `plan_drift` without `frozen`;
  - an adoption during a person's held run leaves the run in progress, and it completes once released;
  - another caller, even repeating the owner's request, is refused `scope_owner_mismatch`; the same owner re-adopting succeeds;
  - after adoption the owner's send runs, and another caller's is refused.
- Breaking: `Capability` gains `Adopt`, so an exhaustive `match` on it needs the new arm.
- `LlmRunnerSubject` gains defaulted `host_plan` and `start_host_session` hooks, required only when declaring `adopt`.

## 0.5.0

- Require cortexkit-role-llm-runner 0.2.5 and add structured-output cases, run only when the subject declares `structured_output` (otherwise each is reported as skipped, by name). They check that:
  - `final_text` on a tool-free run whose route reports support completes `enforced` and `passed`;
  - `final_text` on a run with tools, an admitted tool named `submit_result`, and a schema with an external `$ref` or `$dynamicRef` are refused at admission naming their fields, writing nothing;
  - `submit_tool` offers `submit_result` from the first request, and calling it ends the run `completed`;
  - a sibling of `submit_result` in the same step is never invoked at the scripted tool provider, even when emitted first, gets a never-executed `skipped_after_submit` result and is listed in `skipped_siblings`; a second `submit_result` is a skipped sibling;
  - a run finishing without `submit_result` reports `absent`, and a value failing the schema reports `failed` on a `completed` run;
  - the next model request after a kill and restart carries the same tools and schema bytes;
  - a later send without `structured_output` carries no schema.
- Breaking: `Capability` gains `StructuredOutput`, so an exhaustive `match` on it needs the new arm. That public type change makes this a minor (0.5.0) rather than a patch.
- `LlmRunnerSubject` gains defaulted `send_fields_with_tools`, `set_structured_output_support` and `model_requests` hooks, required only when declaring `structured_output`; the new `ModelRequestObservation` is non-exhaustive with a constructor. A `ScriptedToolCall` may name `submit_result`.

## 0.4.0

- Require runner 0.2.4 and add input-token cap cases, run only when the subject advertises `input_token_cap` (otherwise each is reported as skipped, by name). They check that:
  - a step does not start once consumed input equals the cap;
  - when a step crosses the cap, none of its tool calls run, and each gets a never-executed result;
  - the cap and the running total survive a runner restart between episodes;
  - a later send can lower the cap but not raise it;
  - unknown members of `budget` and of a nudge are refused;
  - a step with no reported usage ends the run as `budget_exhausted` with cause `usage_unreported`.
- Breaking: `ScriptedTurn` gains optional per-step `ScriptedUsage` and becomes non-exhaustive. Replace struct literals with `ScriptedTurn::new(parts)` and `.with_usage(...)`; absence deliberately omits usage. Report input/cache-read/cache-write/output exactly as scripted, without estimates.
- Input-cap adapters must implement external `model_invocations` observations and decode native not-executed results through `not_executed_tool_result`. The suite does not impose a shared tool-result body.
- The fake runner reads typed budget copies through `SendRequest::budget()` while preserving raw `runner_params["budget"]`; malformed budget refusals retain their field paths.

## 0.3.1

- Require cortexkit-role-llm-runner 0.2.3, which adds the terminal `reply_too_large` refusal and its frame-size detail.

## 0.3.0

- Requires subc-protocol 0.32 and cortexkit-role-compaction-provider 0.2.3.
  Consumers must use the new conformance major with protocol 0.32 because its
  public route errors expose `ErrorBody`; update the paired compaction provider
  dependency as well. No conformance behavior changes.

## 0.2.2

- Add discovery-gated boundary cancellation, unlisted-point refusal and immediate escalation cases. Missing `step_boundary` support is reported as named skips, never passes. The boundary case also checks owner-only admission and idempotent repeats.
- Add defaulted held-model control/observation hooks to the subject adapter and broken fake-runner variants for each new rule, with tests proving each case rejects its corresponding defect.
- Require `cortexkit-role-llm-runner` 0.2.2 for `RunCancelRequest.at`, cancellation-point constants and `RoleDescribe.run_cancel` discovery metadata.

## 0.2.1

- Update the compaction-provider dependency to 0.2.0, which adds optional native endpoints, immutable view retrieval, context geometry and notice receipts for Claude Code. Publish this dependency change under a new patch version; existing conformance cases and behavior are unchanged.

## 0.1.4

- The compaction cases that assert how a run ended now wait for it to end
  instead of reading `run.result` once. A runner may accept a send and end the
  run a moment later, so a single read could see `active` and fail
  `compaction_unavailable_distinct_from_refuse` with "Setup failure ended
  "active", not error" on a correct runner under load. `compaction_wait_cap`
  still reads once before the cap, where it checks the run has not ended.

## 0.1.3

- `compaction_setup_durable_once` checks that compaction Setup survives a crash
  without being run again. When the runner resumes by a new send, the case
  recognised that send's user message partly by the run named on the message.
  It now recognises it by its prompt text and the send's new run only. Naming
  the run on each message is a separate contract rule (section 6, run
  attribution), still required of runners that declare `run_ops` and checked
  by its own case, `run_attribution_per_message`.

## 0.1.2

- Accept both automatic and sender-driven recovery in `compaction_setup_durable_once`: an interrupted run resumes through a new send on the same session without repeating Setup.
- Check that the held model request preserves the recovered summary and kept messages, including message ids, content and ordering, with only the resume user's message appended.
- Exercise both recovery shapes, repeated Setup on a resume send, changed kept content, and unexpected recovery states in the suite's fake-runner tests.

## 0.1.1

- Add the optional controlled retention clock for conformance subjects, so long retention cases run without wall-clock delays; cases without the controlled retention clock skip waits longer than 30 seconds and report the skipped case by name.
