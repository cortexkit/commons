# Changelog

## 0.2.1

- Pin media `=0.1.4`, whose engine identity is `(kind, provider, model)` and whose
  engine set may change at runtime with describe and the declaration kept equal.
- Compare describe/declaration sets by the full identity. Exercise the same
  kind and model under two providers, declaration order independence, missing
  providers and duplicated provider identities with focused faulty subjects.
- Routers must run `cortexkit-role-routing` 0.7.12 or later before providers
  declare the same `(kind, model)` under two providers; older parsers reject the
  pair and refuse the whole declaration.

## 0.2.0

Breaking: `call_as` now returns the raw RESPONSE body; adapters stop unwrapping.
`StreamObservation::answer` now returns the raw terminal RESPONSE body too.
Both include `{"result": <answer>}`; STREAM_DATA event bodies and referenced
output-object contents stay unwrapped. Refusals still carry the complete ERROR
body through `SubjectError::refused`.

- Pin media `=0.1.3` and check success bodies centrally with `wire::read_reply`.
- Add `reply_envelope` and `stream_reply_envelope` cases with independent bare
  and extra-key faulty providers, mapped to the pinned reply-envelope rule.
- Bump the kit's minor version because existing adapters returning decoded
  answers must change to preserve the provider's RESPONSE body.

## 0.1.2

- Pin media `=0.1.2`, which newly refuses quantities whose coefficient (all
  digits with the decimal point removed) exceeds 9223372036854775807, including
  `9223372036.854775808` and `9223372036854775807.999999999`, even with at most
  9 fractional digits. The refusal remains `invalid_usage_record`, field `quantity`.
- Correct 0.1.1's wrong stated reason: bounding only the whole part and fraction
  length did not ensure exact ledger storage as one signed 64-bit coefficient
  and a scale of at most 9. Use valid exact decimals in the kit fixtures.
- Keep the 10-fractional-digit faulty provider in `usage_quantity_bounds` and
  add `usage_quantity_coefficient_bounds` with an independent coefficient-overflow
  faulty provider. Ship as a patch because no producer emits media usage yet.

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
