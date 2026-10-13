# Changelog

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
