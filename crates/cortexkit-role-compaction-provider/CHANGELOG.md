# Changelog

## 0.1.2

- Step statuses gain two optional host-lane fields: `served_through_ordinal`, the highest message ordinal the host has durably served, and `unserved_subjects`, the hook answers the host never served. The subject-part check is shared with the step-transform crate, and tests now load the host-lane vectors.
