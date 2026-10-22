# Changelog

## 0.1.0 (unpublished)

- First version of the `extractor/v1` conformance suite, request side: 21
  checks of `role.describe`, admission refusals (each checked for its code,
  for the field and limit where the case expects them, and for reaching no
  provider and reserving no batch id), batch identity,
  the zero-ceiling spend check, and the Clairic request mapping, against a
  stand-in provider that records what it receives.
- `publish = false` until the answer side is measured.
