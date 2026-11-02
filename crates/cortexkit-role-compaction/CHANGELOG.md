# Changelog

## 0.1.0

- Introduce the `compaction-provider/v2` and `compaction-runner/v1` wire contract.
- Provide forward-compatible wire enums, extensible construction APIs, and
  JSON vectors for requests, answers and refusals, round-trip tested, with a
  SHA256SUMS manifest.
- Use the published runner contract's canonical content digest and existing
  `session.send` / `run.result` protocol.
