# `late_result_ready.v1` bus notice vectors

Each case contains the subject in `subject.txt`, the `Nats-Msg-Id` header value
in `nats-msg-id.txt`, and the raw inline compact JSON body in `payload.json`.
Payload files have no trailing newline. The subject and header files end in LF;
that delimiter is not part of the subject or header value.

All cases use provider module id `provider` and custodian `reserved:reader`, so
the reader suffix is `reader`. Case identities and outcomes are:

- `ok`: call key `call:ok`, stable event id `settlement:1`, outcome `ok`.
- `error`: call key `call:error`, stable event id `settlement:2`, settled error
  code `tool_denied` (whether from protocol `code` or tool `errorCode`).
- `unknown`: call key `call:unknown`, stable event id `settlement:3`, outcome
  `unknown`.

These are hints only, not result bodies or claim checks. Readers fetch and ack
through `late_results`, reconcile at startup, accept open outcome strings and
ignore unknown members. A retry or restart replay uses the same message id and
never announces an entry the reader has already acked.

`SHA256SUMS` pins every fixture and this README. With the `test-vectors` feature,
`vectors::late_result_ready` exposes the same packaged bytes. The
`late_result_ready` tests check the manifest, subject/header construction and
exact serializer output; `scripts/gate.sh fast` (the repository's quick check) also verifies the checksums.
