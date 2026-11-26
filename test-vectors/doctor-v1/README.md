# Doctor report vectors (v1)

`manifest.json` is canonical manifest JSON with one trailing LF. `inputs.json`
contains the fixed report context, the check and refused fix documents, and
the synthetic canaries. The context's two keys are for fixtures only.

For `report.json`, use the first key and supply the fixes in this order:
`store.note` times out; `store.backlog` returns `refused_fix`. Pass the tails
in order: empty, `tail-a.log`, `tail-b.log`, then `cr_tail` from `inputs.json`.
Tail A ends with LF and tail B ends without one. The CR-terminated line is
encoded in JSON so text checkout does not normalize it away.

`evicted-report.json` uses the same inputs and `evicted_body_bytes` as the
budget. The earliest line from tail A and the line from tail B have the same
timestamp. The first uses a path and the second a ref with the same first
HMAC window. The ref keeps its second window even after the path line is
evicted, and the drop on the evicted line still counts.

`timed-out-check.json` uses the same context and user text, but the check
times out and there are no fixes or tails. Expected reports are canonical
JSON without a trailing LF. `SHA256SUMS` pins every fixture's bytes.
