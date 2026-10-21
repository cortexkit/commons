# Changelog

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
