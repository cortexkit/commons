# Changelog

## 0.1.1

- Documentation only: `StreamParser` now states that in program mode `finish`
  refuses a source cut off at the size limit (`E_SOURCE_TOO_LARGE`, no nodes),
  as `parse_bytes` does, so a program never runs from a cut-off prefix.

## 0.1.0

- Publish the rich UI v1 syntax front end, including `scan_references` for
  reply and program sources and `check_token_slot` for exact token slots.
- Ship the original and extended reference-scanning sample files behind the
  `test-vectors` feature, with SHA-256 checksums for their packaged bytes.
