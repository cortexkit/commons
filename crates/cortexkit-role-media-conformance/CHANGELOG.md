# Changelog

## 0.1.1

- Pin media `=0.1.1`, which newly refuses usage quantities with more than
  9 fractional digits or a whole part above 9223372036854775807. Use bounded
  exact decimals in fixtures and check that a provider emitting 10 fractional
  digits fails the named `usage_quantity_bounds` case.
- Ship the stricter validation as a patch because no producer emits media usage
  yet, so no deployed value is affected.

## 0.1.0

- Initial subject interface, generated audio fixtures, and job, object, usage and
  job-admission conformance cases for media-provider/v1.
- Add stream transport hooks and stream, bias/pack, realtime token, call lifecycle
  and shared stream/call admission cases, each checked against focused faulty fakes.
- Complete the rule map and exemption checks for every pinned media rule, with
  wire-extension and realtime-watch cases defended by focused faulty fakes.
