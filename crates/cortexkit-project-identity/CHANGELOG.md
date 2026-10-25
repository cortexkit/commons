# Changelog

## 0.1.1

- Bundle `resolve-root-records.json`, five more replies captured from a running
  handler: populated Git remotes, an unreadable Git config with
  `remotesError`, a bound and approved root, and the `unbound`, `replaced` and
  `retired` root identities, the last through `containment` with `sourceRoot`.
  Each decodes and re-encodes to its exact bytes. The types are unchanged.

## 0.1.0

- Add non-exhaustive serde request, success, refusal, and reply types for
  `resolve`, `resolve_project_id`, and `enumerate`.
- Preserve unknown string vocabularies and missing/null root-mode fields while
  tolerating additive object members.
- Bundle replies captured from a running project-identity handler, unchanged
  with their checksum manifest, plus labelled synthetic forward-compatibility
  samples and decoding tests.
