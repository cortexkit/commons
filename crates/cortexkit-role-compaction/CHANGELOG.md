# Changelog

## 0.2.1

- Depend on llm-runner 0.8 (the one-based entry contract).
- Add inserted-image wire types: PNG images on user-role inserted messages,
  rendered after their texts without tag numbers. Images count toward the
  view's size and checksum; empty image arrays are omitted and hash as absent.
  Specify the runner's required `unsupported_edit` (with the insert's edit
  index) for images on other roles, non-PNG media types and invalid standard
  padded base64. Wire decoding validates neither; the view library adds the
  checks.
- Add optional `accepts_images` to Setup and conditions: absent means unknown,
  and a provider inserts images only when the runner reports `true`.
- Specify that a message id is never given to a different message within a
  session, even after a revert or branch switch, and that providers key durable
  part state on both the message id and the part id (P6).
- Note that `step-transform-provider/v2`'s `pre_user` hook carries an optional
  `at_ms` taken on the runner's machine, so gaps against write times from that
  machine have no clock skew; without it the provider uses its own clock (U3).
- Add image-insert, image-capability Setup and image-refusal vectors (assistant
  role, non-PNG); existing vector bytes are unchanged. Add tests for image
  checksums, empty-image normalization and absent/true/false capability.
- Vector regeneration skips writing a file whose bytes already match.

## 0.2.0

- **Breaking:** a notice's `newest` is now optional and absent only for an
  empty transcript, including after a revert. Construct notices with
  `NoticeRequest::new(seq, conditions)` and add a cursor with `with_newest`.
- Allow signed parts to be omitted as an oldest run, a newest run, or both,
  while still refusing a gap between kept signed parts (PR2).
- Specify that empty tool `attachments` and `delivered` and notice `events`
  and `entries` may be omitted, decode as empty, and are omitted for checksums
  (W1.6). Add an entry checksum vector with both attachment forms.
- Restrict the contract's own integer fields to 0 through 9007199254740991
  for exact JavaScript round trips, with named `invalid_params` refusals;
  opaque JSON payloads keep their numbers unchanged (W1.2).
- Refuse a read's `limit: 0`; a supplied limit must be at least 1 (W5.4).
- Qualify byte-identical reply rendering: cleaning must change nothing else,
  including surrounding whitespace (T7).
- Explain that a view may hide the newest number and the runner replaces a
  lower number the model then writes with the assigned one (W6.4).
- Adopt step-transform-provider/v2: answer `role.describe` and
  `transform.declare` on any route of that role, and refuse an unbound
  `transform.hook` per call with `route_unbound`, without changing compaction
  route requirements (W2.1, W2.4). Retire the transitional rule for
  step-transform v1 hook routes, along with the `session_mismatch` refusal
  code, its constructor and its vector.
- Add vectors for an empty-transcript notice, accepted and refused integer
  boundaries, and a refused zero read limit; refresh vector checksums.

## 0.1.0

- Introduce the `compaction-provider/v2` and `compaction-runner/v1` wire contract.
- Provide forward-compatible wire enums, extensible construction APIs, and
  JSON vectors for requests, answers and refusals, round-trip tested, with a
  SHA256SUMS manifest.
- Use the published runner contract's canonical content digest and existing
  `session.send` / `run.result` protocol.
