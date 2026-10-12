# Changelog

## 0.2.1

- Add `session_ref(harness, project_root, session)`, the session reference from contract section 14.1, so every module that keeps state for a runner session derives the same value. It is checked against `test-vectors/llm-runner-v1/session-ref.json`.

## 0.1.1

- Clarify that compaction conformance accepts runner-driven recovery or an interrupted run resumed by a sender's new send; runtime rules are unchanged.
