# Changelog

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
