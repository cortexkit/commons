# Changelog

## 0.1.5

- Add open measurement roles `input_image`, `output_image`, `output_video` and
  `cache_read`, all in tokens. `cache_read` reports text input tokens served
  from the vendor's cache, including in media jobs.
- Document exact same-named token billing rates and the unit-billed
  `rate.<role>.per_<unit>` lookup. Unknown roles still round-trip unchanged;
  readers do not need an upgrade before engines emit the new names.

## 0.1.4

- Key engine identity and describe/declaration set equality by
  `(kind, provider, model)`. The same kind and model may be served through
  different providers; a repeated full identity remains `duplicate_engine`.
- Allow the engine set to change while a provider runs, keeping describe and
  the declaration equal and listing only engines it can run right now. Update
  the declaration through `catalog.update_role_declarations` when the daemon
  advertises it in `HELLO_ACK`; otherwise change it only at start. Remove a lost
  engine before refusing its jobs where possible; accepted jobs keep their rules.
- Readers first: media 0.1.3 readers, and routing's declaration parser before
  `cortexkit-role-routing` 0.7.12, reject the same `(kind, model)` under two
  providers as `duplicate_engine`. Do not declare such a pair until the router
  runs routing 0.7.12 or later.
- Add shared describe identity vectors without changing existing vector bytes.

## 0.1.3

- Pin every successful reply, including a streamed call's terminal reply, to a
  RESPONSE body exactly `{"result": <answer>}`. STREAM_DATA event bodies and
  referenced output-object contents stay unwrapped; refusals remain ERROR
  frames `{code, message, detail}`.
- Add `wire::reply_body`, `wire::read_reply` and `wire::ReplyShapeError`, parallel
  to routing's helpers, refusing bare answers and extra top-level keys by name.

## 0.1.2

- Newly refuse usage quantities whose coefficient (all digits with the decimal
  point removed) exceeds 9223372036854775807, even with at most 9 fractional
  digits, as `invalid_usage_record` with field `quantity`. For example,
  `9223372036.854775808` and `9223372036854775807.999999999` are now malformed.
- Correct 0.1.1's wrong claim that bounding only the whole part and fractional
  digit count lets a fixed-point ledger store every valid quantity exactly.
  The ledger needs the entire coefficient to fit one signed 64-bit integer,
  together with a scale of at most 9. Decimal shape rules are unchanged.
- Ship as a patch because no producer emits media usage yet. Add the valid
  coefficient edge `9223372036.854775807` and move the old whole-plus-fraction
  sample to the refusal vectors; the whole-number maximum remains valid.

## 0.1.1

- Newly refuse usage quantities with more than 9 fractional digits or a whole
  part above 9223372036854775807, as `invalid_usage_record` with field `quantity`,
  so the fixed-point ledger can store valid quantities exactly. Decimal shape rules
  are unchanged.
- Ship the stricter validation as a patch because no producer emits media usage
  yet, so no deployed value is affected. Add valid bound-edge and refused
  just-over vectors.

## 0.1.0

- Publish the media-provider/v1 contract, typed wire values, describe and payload
  checks, bias trimming and voice-front helpers with shared test vectors.
