# Changelog

## 0.1.0 (unpublished)

- First version of the `extractor/v1` role contract, request side:
  `role.describe`, the `extract.run` request with its nine field types and
  their closed option table, admission in a pinned order (`request::admit`),
  the reply-size check against the daemon's 4 MiB frame, the pre-call spend
  check with the shared conservative estimate, batch identity (model,
  fields, options in force, items with canonical images), and the Clairic
  request mapping.
- The batch rules it shares with `classifier/v1` come from
  `cortexkit-model-batch` 0.1.0: the error vocabulary and the Workers AI
  code table, images, identity hashing, the estimate, absent-is-not-zero
  and number text.
- Answers keep each named member as the provider's text (`RawValue`) and
  every other member as given. The answer shapes not yet measured are the
  contract's §12 open readings.
- `publish = false` until the answer side is measured.
