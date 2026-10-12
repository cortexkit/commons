# Changelog

## 0.2.0

- Add optional typed native endpoints, frozen raw replacements and bounded, optional `compaction.view.read` with typed digest/version refusals. Inline replacements still apply without retrieval; declared single-message caps refuse rather than truncate.
- Add typed optional geometry, output reserve, native continuation and attributed fill. Geometry is required and validated only for an attested connection whose frozen declaration selects Claude Code. Off-lane absence and supplied geometry preserve old behavior.
- Add optional notice directives and explicit next-status notice-id dispatch receipts, with an idempotent reference acknowledgement helper and conflict refusals. Pending notices remain unchanged until acknowledged; only real provider dispatch authorizes acknowledgement.
- Publish and echo the shared optional capability bundle at describe/setup. Existing vectors and SHA256SUMS remain byte-identical; new vectors have separate checksums.
- Minor package bump: optional fields on previously exhaustive `CompactionMessage` and `StepAnswer` variants break Rust struct/variant literals. The wire major remains v1. Update the workspace conformance consumer's dependency range accordingly.

## 0.1.2

- Step statuses gain two optional host-lane fields: `served_through_ordinal`, the highest message ordinal the host has durably served, and `unserved_subjects`, the hook answers the host never served. The subject-part check is shared with the step-transform crate, and tests now load the host-lane vectors.
