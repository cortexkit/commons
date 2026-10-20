# `agent-inbox/v1` test vectors

These package-local bytes are normative examples, exposed by the
`test-vectors` feature. `FILES` lists every file except SHA256SUMS; that file
pins their SHA-256 bytes in FILES order, including this README.

- `posts.json`: provider text, stamped-flow template with every optional field,
  reserved carrier owner notice, first/replayed/future replies, structural and
  semantic invalid requests. `post_shapes` checks them.
- `withdraw.json`: both selectors, all three answers, future state and invalid
  selector combinations. `withdraw_shapes` checks them.
- `pull.json`: own-inbox list/read params, empty and populated replies, every
  lifecycle state and a future state. `pull_shapes` checks them.
- `refusals.json`: every refusal with explicit terminal/retryable classification
  and one unknown refusal. `refusal_finality` checks completeness and finality.
- `neutralize.json`: exact input/output pairs for ASCII case, whitespace,
  nested/attribute/incomplete/prefix forms, split names, Unicode look-alikes
  and non-marker whitespace. `neutralize_pairs` also checks idempotence.

Additional unit/integration tests defend boundary-sized serialized notices,
all rendered field families in the live suite, open reply members and builders.
