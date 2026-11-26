# Media v1 shared vectors

JSON files are UTF-8. Test quantities are decimal strings, never numbers.
Tokens are placeholders, not credentials. `SHA256SUMS` covers every other file.

`attribution.json` names the `media.submit` and `media.stream_transcribe` wire
operations and includes requests with and without `agent_id`. Its usage records
cover a relayed call with stated agent and daemon-stamped device, local
dictation with a stated agent and null device, and an absent caller. Caller is
record content, not identity or permission; absent caller means unknown owner.

`call-usage.json` is one attached voice-front call with routed realtime token
usage and billed companion transcription seconds. Both records carry the same
admission decision; the companion is not the model named by that decision.
Record ids are derived from the principal/call/provider/model tuple and redeliver
unchanged.
The duplicate-model refusal recipe selects records by zero-based `record_indices`.

Large examples are deterministic expansion recipes, not abbreviated wire values:

- chunks: generate exactly `decoded_bytes` copies of the byte `byte`, then
  standard padded base64-encode the result. The cases name 2,097,152 and
  2,097,153 decoded bytes. Reads use the stated length without expansion.
- budgets: each inline recipe becomes a media value with that many copies of
  byte 97, its actual sha256 and base64. Both inline values are individually
  legal. `text_bytes` becomes that many ASCII `a` characters in synthesis text.
- usage bounds: expand every string to `scalar_limit` copies of U+1F600,
  including open enum strings and map keys, except quantity, which is
  `9223372036.854775807`. Quantities have separate decimal bounds:
  at most 9 fractional digits and a coefficient (all digits with the decimal
  point removed) at most 9223372036854775807.
  `refused_records` names the malformed records and their refusal code and field.
  Distinct selector keys replace the first scalar with the
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
Realtime vectors keep the original watch event without tool and add ask, hand_off
and an unknown tool. Refusal vectors include both known front_unavailable reasons
and an unknown reason, all preserved verbatim as permanent refusals.
Realtime token answers cover absent and HTTPS connect_url members; the URL
over-length recipe fills to total_bytes, including the prefix. Stream invalid
data expands decoded_bytes copies of byte, then standard padded base64-encodes
them; each invalid frame refuses on data without consuming retry_seq.

Generated synthetic WAV fixtures and their generator belong to the conformance
kit, not the wire-contract crate. No recorded voice is shipped here.
