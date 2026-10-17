# Changelog

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
