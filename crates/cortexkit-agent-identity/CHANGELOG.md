# Changelog

## 0.1.1

- Add strict params and separate reply types for `agent.session_for_ref` and
  `agent.session_for_runner`, retaining their different session wire names,
  runner presence, lifecycle fields, and camelCase `heldOn`.
- Add `agent.fleet_identity` params accepting arbitrary JSON tokens and a reply
  for full/unchanged fleets, with typed live-agent, avatar and placement summaries.
  Missing arrays/objects stay distinct from present-empty values; string
  vocabularies preserve future spellings and replies remain open to new members.
- Reuse project-identity 0.1.4 envelopes and its named `RefusalCode::NotFound`.
  Existing 0.1.0 public types are retained unchanged.
- Bundle 31 byte-identical handler captures for the three new reads, including nineteen
  session lookup cases, with per-record exact-byte tests and checksum manifests.
  Document remaining untested cases, including nonzero fleet `logPosition`.

## 0.1.0

- Add serde-only context project/session, agent resolution and whereabouts reads
  for the agent registry, shared envelopes, and exact-byte handler captures.
