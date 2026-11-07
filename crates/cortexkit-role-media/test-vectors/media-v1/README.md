# Media v1 shared vectors

JSON files are UTF-8. Test quantities are decimal strings, never numbers.
Tokens are placeholders, not credentials. `SHA256SUMS` covers every other file.

Large examples are deterministic expansion recipes, not abbreviated wire values:

- chunks: generate exactly `decoded_bytes` copies of the byte `byte`, then
  standard padded base64-encode the result. The cases name 2,097,152 and
  2,097,153 decoded bytes. Reads use the stated length without expansion.
- budgets: each inline recipe becomes a media value with that many copies of
  byte 97, its actual sha256 and base64. Both inline values are individually
  legal. `text_bytes` becomes that many ASCII `a` characters in synthesis text.
- usage bounds: expand every string to `scalar_limit` copies of U+1F600,
  including open enum strings and map keys, except quantity, which is that many
  ASCII digits `9`. Distinct selector keys replace the first scalar with the
  corresponding U+1F601 through U+1F610. Failure.message uses 1,024 scalars;
  failure.detail has field and limit at 256 scalars each. Status and closed
  enums remain the stated values, so they cannot reach the generic string
  bound. Selectors and measurements each have 16 entries. The test validates
  the expanded record, increments each string/entry bound independently and
  rejects it. The local basis and other statuses are separate literal vectors.
- usage size: the oversize recipe expands failure.detail to contain the stated
  number of string-valued members, each the stated UTF-8 byte length. Tests
  construct records at 262,143, 262,144 and 262,145 encoded bytes. The size check
  rejects the latter two (the record bound is strictly less than 262,144).

Hand-off message expected strings are literal output bytes, not computed by the
helper under test. Each listed line break occurs in framing and transcript text.
The two send ids share one call id but have different principals. Usage feed
vectors deliberately include entries with both record/raw and with neither.

Generated synthetic WAV fixtures and their generator belong to the conformance
kit, not the wire-contract crate. No recorded voice is shipped here.
