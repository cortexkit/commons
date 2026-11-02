# Changelog

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
