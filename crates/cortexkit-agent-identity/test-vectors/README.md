# Bundled read vectors

The JSON files in `memory-context-v1/`, `agent-sessions-v1/`, and
`agent-identity-reads-v1/` are unchanged published captures from the
project-identity module's handler. Each directory's `SHA256SUMS` lists exactly
its copied JSON files; the checksum test verifies the
manifests against the embedded bytes.

Only context project/session and agent resolve/where/batch records are included.
Context replies embed document metadata, not a full workspace document, so the
separate `workspace-document.json` operation is not copied. `agent-resolve.json`
includes seven direct resolve records: live head, live hire, retired, unknown, and three
invalid-request refusals. `agent-resolve-merged.json` holds two records captured
through a real merge: the merged agent and the live agent it merged into.

Record metadata is not part of a reply. Tests decode each reply and serialize
through a sorted-key JSON value to compare with its exact embedded compact bytes.
Synthetic compatibility tests live separately in `tests/decoding.rs`.
