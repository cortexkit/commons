# Changelog

## 0.2.1

- Add optional `SetupRequest.descends_from`, reusing `status::DescendsFrom`,
  and a matching builder. Setup decoding is lenient and the request is
  non-exhaustive, so this is an additive patch with wire major v1 unchanged.
- Advertise setup descent with optional describe flag `setup_descends_from`.
  Callers send setup `descends_from` only when the flag is true; otherwise
  they declare descent on the first step or hook. Absent/false means
  unsupported, avoiding silent loss through older lenient setup decoders.
- Pin parent/cut validation, durable descent links, idempotent repeats and
  `invalid_params` / `lineage_id` conflicts across setup, step and hook.
  Setup without descent leaves the lineage undeclared: first-step and
  first-hook declarations remain valid, and no declaration leaves a root.
- Add a provider-owned `lineage::DescentLink` reference check and shared
  setup/step/hook descent vectors, separately pinned without changing any
  existing vector bytes. The planned
  `cortexkit-role-compaction-provider-conformance` crate will exercise real
  provider modules over routes (CONTRACT.md §17); it is not implemented yet.

## 0.2.0

- Preserve native ingress/hook JSON with optional raw-message sidecars, required
  and syntax-checked only on the admitted Claude Code lane (CONTRACT.md §19.1).
  The provider must verify that `raw_message`, with `cache_control` removed only
  at the Anthropic transport positions listed in CONTRACT.md §19.6, equals the
  parsed `message`. Raw bytes and JSON escaping count toward payload byte caps.
- Reject raw payload references without selected retrieval. Add typed bootstrap completion receipts with generation-scoped replay/conflict checks, exact frozen bundle identity and length-prefixed raw-source SHA-256 framing; a final page alone is not completion.
- Add optional typed native endpoints, frozen raw replacements and bounded, optional `compaction.view.read` with typed digest/version refusals. Inline replacements still apply without retrieval; declared single-message caps refuse rather than truncate.
- Add typed optional geometry, output reserve, native continuation and attributed fill. Geometry is required and validated only for an attested connection whose frozen declaration selects Claude Code. Off-lane absence and supplied geometry preserve old behavior.
- Clarify the gateway geometry invariant: a present output reserve must fit under the absolute wall with the hard input ceiling; omitted fill means no reading. Notice dispatch acknowledgement requires successful 2xx response headers, not merely sending a request.
- Add optional notice directives and explicit next-status notice-id dispatch receipts, with an idempotent reference acknowledgement helper and conflict refusals. Pending notices remain unchanged until acknowledged; only real provider dispatch authorizes acknowledgement.
- Publish and echo the shared optional capability bundle at describe/setup. Existing vectors and SHA256SUMS remain byte-identical; new vectors have separate checksums.
- Minor package bump: optional fields on previously exhaustive `StatusMessage`, `CompactionMessage` and `StepAnswer` variants break Rust struct/variant literals. The wire major remains v1. Update the workspace conformance consumer's dependency range accordingly.

## 0.1.2

- Step statuses gain two optional host-lane fields: `served_through_ordinal`, the highest message ordinal the host has durably served, and `unserved_subjects`, the hook answers the host never served. The subject-part check is shared with the step-transform crate, and tests now load the host-lane vectors.
