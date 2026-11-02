# tool-display/v1 vectors

- `canonical.json`: equivalent bodies with differing spacing, key order and
  number spelling, with the exact RFC 8785 JCS bytes.
- `field-kinds.json`: valid and invalid scalars for all nine field kinds,
  including traversal, drive/UNC paths, uppercase or suffixed origins and app paths.
- `records.json`: required and fixed fields, held reasons, location, declared
  scalars and arrays, forbidden nested/undeclared fields and completions.
- `details.json`: free-form detail objects and standard cut metadata.
- `writes.json`: limits, wire params, stored/already-stored/conflict/too-large
  answers and invalid params, with the retryable not-ready code.
- `views.json`: copied byte for byte from
  `crates/cortexkit-role-tool-provider/test-vectors/tool-provider-v1/views.json`,
  including a manifest and an unknown pin. Both packages validate these bytes,
  and the provider's `tests/views.rs` checks the two copies are identical.
- `view-tools.json`: copied byte for byte from the same provider-owned vector
  directory. Pins matching and mismatching tool definitions, and a legacy view
  whose missing `tool` keeps its serialized bytes and requires a catalog check.

`SHA256SUMS` covers every JSON file and this README. Tests read every JSON file;
`scripts/gate.sh fast` at the repository root verifies every checksum.
