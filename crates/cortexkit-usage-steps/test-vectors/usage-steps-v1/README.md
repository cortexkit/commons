# `usage-steps-v1` test vectors

Normative examples of the live usage step feed, contract version 1. See
`../../CONTRACT.md` for the rules. Every value is invented: no real accounts,
sessions, paths or projects.

## Encoding

Each `.json` case except `cases.json` is one compact JSON value (no spaces, no
trailing newline), exactly as this crate encodes it. Member order is this
crate's encoding order; it is not part of the contract, and a reader must not
depend on it. `SHA256SUMS` pins every file in this directory, this README
included, with paths relative to it.

## Index

`cases.json` lists every case by file and type, so a test suite in any
language can drive them:

- `round_trip`: decodes as `type` and re-encodes to the same bytes.
- `normalize`: decodes as `type` and re-encodes to the bytes of the file in
  `encodes_as`. These are inputs a reader accepts but never produces: an
  unknown member on a row, reply or notice (ignored), `null` for an optional
  request member, and a reply without `next`.
- `reject`: must not decode as `type`. `provider_code` names the code the
  provider refuses it with, where the contract names one. An undecodable
  request is refused `malformed`.

## Cases

- `subscribe/`: the subscribe request `{}` and one notice.
- `read/`: three read requests, three replies and two standalone rows.
  `reply-page.json` has eight rows over five sessions; its fourth row
  (`c0000104`) has the identity of its first row (`opencode` plus
  `msg_ex_0001`) and replaces it because its finish reason arrived late, so a
  reader keeps seven rows. `next` is set because more pages remain.
  `reply-last-page.json` holds a row with every member and a row with only
  the required members, and `next` is null. `reply-empty.json` has no rows.
- `values/`: each verdict, request size and source value, plus one value
  per set that this version does not know, which a reader keeps as `Other`.
- `refusals/`: one error body per refusal code of either operation.
  `cursor_expired` carries `detail.through`.
- `endings/`: the `lagged` ending of a subscription, with `detail.through`, and the `cancelled` ending a provider may send after the caller's own Cancel.
  A StreamEnd and the caller's Cancel carry no body.
- `tolerant/` and `rejected/`: the `normalize` and `reject` cases above.

The rows show the wire shape. Their numbers are plausible, but they are not a
fixture for recomputing verdicts: the feed may hold earlier steps of a session
that a vector does not show. Each `retention` is the exact quotient, never
rounded, and the runner row `c0000102` has one above 1, from a step that read
more cache than expected.

## Producers

A row whose producer is a harness store names one of the contract's listed
harnesses (`opencode`, `codex`, `gemini`, `kimi` here). A row from a runner
uses the neutral module id `example-runner`: a runner's module id is open,
and public test data does not name real runner modules. These runner rows
stand for steps without a host record, so their identity is the runner's
module id plus `<run_id>#<step_id>#<attempt>` (see "Runner steps" in `CONTRACT.md`).
