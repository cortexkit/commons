# Changelog

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
