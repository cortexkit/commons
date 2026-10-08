# Changelog

## 0.1.2

- Hook requests gain four optional host-lane fields: `served_through_ordinal` (the highest message ordinal the host has durably served), `unserved_subjects` (hook answers the host never served), `subject_part` (which tool part of a message the hook is about; non-empty, at most 256 bytes) and `pass_complete` (marks the last hook of a pass). Tests now load the host-lane vectors.
