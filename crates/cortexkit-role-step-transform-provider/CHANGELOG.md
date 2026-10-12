# Changelog

## 0.2.0

- Add optional Claude Code capability publication to describe/declare, frozen connection-scoped admission, daemon-compatible capability identifier validation and declared payload-cap checks. The gateway requires both a daemon-attested principal and the selected bundle; body fields never grant authority. Existing absent-field wire bytes and vector checksums are unchanged.
- Extend host admission to the attested Claude Code gateway, including answer observation and bounded host paging without transcript reads or a ready service.
- Minor package bump: adding an optional field to the previously exhaustive public `Declaration` breaks Rust struct literals. Wire major remains v1; no existing wire member changes meaning. New vectors use synthetic epochs, not released codec epoch claims.

## 0.1.2

- Hook requests gain four optional host-lane fields: `served_through_ordinal` (the highest message ordinal the host has durably served), `unserved_subjects` (hook answers the host never served), `subject_part` (which tool part of a message the hook is about; non-empty, at most 256 bytes) and `pass_complete` (marks the last hook of a pass). Tests now load the host-lane vectors.
