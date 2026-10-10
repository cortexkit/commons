# Changelog

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
