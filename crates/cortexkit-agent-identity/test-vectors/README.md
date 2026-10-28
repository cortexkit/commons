# Bundled read vectors

The JSON files in `memory-context-v1/`, `agent-sessions-v1/`, and
`agent-identity-reads-v1/` are unchanged published captures from the
agent registry's handler. Each directory's `SHA256SUMS` lists exactly
its copied JSON files; the checksum test verifies the
manifests against the embedded bytes.

Only context project/session and agent resolve/where/batch, session lookup and
fleet identity records are included. Session writes and unrelated reads are not
part of this crate.
Context replies embed document metadata, not a full workspace document, so the
separate `workspace-document.json` operation is not copied. `agent-resolve.json`
includes seven direct resolve records: live head, live hire, retired, unknown, and three
invalid-request refusals. `agent-resolve-merged.json` holds two records captured
through a real merge: the merged agent and the live agent it merged into.

`session-for-ref.json` (five records), `session-for-runner.json` (four),
`fleet-identity.json` (three), and `session-lookup-cases.json` (nineteen) are
byte-identical copies from the owner's published export. The last file covers
remote finished-session holders, rootless/runnerless rows, both lookup misses,
strict params refusals, a populated fleet, unchanged and stale tokens, and
numeric/malformed-string token cache misses. Fleet log positions are all zero;
see the crate README for every remaining untested case. All existing JSON
captures remain unchanged.

`session-for-harness.json` contains five unchanged handler records: a session with
a runner, an OpenCode head with a null runner, a remote ended session, a `not_found`
refusal, and an `invalid_request` refusal for an extra `session_id` param. Its
SHA-256 is `32ec4b73a8571b8d534b76c4858d4d0190063c8343fe5aea5f2630e50a2c9544`.

Record metadata is not part of a reply. Tests decode each reply and serialize
through a sorted-key JSON value to compare with its exact embedded compact bytes.
Synthetic compatibility tests live separately in `tests/decoding.rs`.
