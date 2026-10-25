# Changelog

## 0.1.2

- Add `resolve_remote` request and reply types with distinct found, none, and
  ambiguous payloads, required fields, and unknown status text preservation.
- Bundle eight unchanged handler samples covering remote matches, ambiguity,
  ASCII case-insensitive matching, skipped unreadable roots, and invalid params.
  Each reply decodes and re-encodes to its exact embedded bytes; all five sample
  files are checked against the published checksum manifest.

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
