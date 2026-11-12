# Changelog

## Unreleased

- Contract: a session subject's `runner` is the hosting runner module's id and
  `harness` its harness name, as the compaction `SessionRef` carries them; apps
  pass `{runner, harness, session}` verbatim, and a session no runner module
  hosts has no session subject.
- Contract: a `request_id` is scoped to its subject; the same id on another
  subject is a different run, not a repeat or a conflict.
- Contract: the `unavailable` refusal means nothing ran, so the same request
  may be repeated with the same `request_id`.
- Documentation only.

## 0.1.0

- Initial `user-commands/v1` management and app wire shapes, pure validators,
  pinned safety rules, shared fact vocabulary, JCS command digest and vectors.
