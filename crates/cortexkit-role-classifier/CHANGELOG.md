# Changelog

## 0.1.0

- First published release of the `classifier/v1` role contract: the
  `role.describe` and `classify.run` wire types, the providers' shared
  question and answer shapes, the batch body identity, error codes and their
  classes, the request validators, and `CONTRACT.md`. Stability: alpha.
- The role's vectors ship inside the crate package under
  `test-vectors/classifier-v1/`, pinned by `SHA256SUMS`. The opt-in
  `test-vectors` feature compiles them into the `vectors` module
  (`vectors::FILES`, plus a constant per file), so a classifier module's
  tests can use the same bytes without a repository checkout.
- Catalog ids are models.dev's own provider keys and pin a model version:
  `cloudflare-workers-ai/@cf/cloudflare/clef`,
  `cloudflare-workers-ai/@cf/cloudflare/clef-flash` and `typesafe/jev-1.13.0`.
- A `role.describe` model entry's `context_tokens` is optional: it is absent
  when the catalog states no exact figure, and absent is not zero.
- `ErrorClass` is non-exhaustive. Error codes are open strings, so later
  codes are additive.
