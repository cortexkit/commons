# `llm-runner/v1` test vectors

The shared bytes every `llm-runner/v1` implementation and consumer checks.
The role document is `crates/cortexkit-role-llm-runner/CONTRACT.md`. This
role is a draft: these vectors change with the contract until it is
reviewed.

| File | What it pins | Checked by |
|---|---|---|
| `role-describe.json` | `role.describe` answers a consumer accepts (with the capabilities each declares), canonical answers (one stating the `max_bytes` cap), a missing `stability` read as `alpha`, and answers it refuses with the problem: a missing major or required op, a declared group missing one of its ops (including `interrupt`), no session-capability source, `transcript_reads` without `max_bytes` | the wire crate's `check_describe` test |
| `read-requests.json` | `session.read` requests for each mode (tail, range, after), with the mode each selects, and refused requests with the `invalid_params` field: `after_mid` without `lineage_id`, `after_mid` with `from_ordinal`, a misspelled cursor, an unknown view | the wire crate's request round-trip and mode tests |
| `read-pages.json` | pages: a tail page, a page stopped by its byte cap with `next_from_ordinal`, a single message over the cap alone and whole on its page, run and dispatch attribution, originals, a denied call with no dispatch target, an empty transcript, a never-written session's empty page with no `lineage_id`; runner extras a decoder ignores; pages a decoder refuses (no `mid`, a non-object `head`); pages without a lineage that still carry messages or a next ordinal; the `lineage_changed` and `unknown_mid` refusals with the requests that get them | the wire crate's page tests |
| `head.json` | the `session.head` request, a request it refuses, and answers with and without a last run | the wire crate's head test |
| `run-result.json` | the `run.result` request, requests it refuses, an answer per run state with whether it is terminal, and the `unknown_run` refusal | the wire crate's run-result test |
| `subscribe.json` | `session.subscribe` requests (absent, `start`, `live`, after a page's `head`) with their attach point, refused requests with the field, and stream events (durable, best-effort, unknown kind) | the wire crate's subscribe test |
| `send.json` | owner `session.send` requests per delivery mode, with a mark, and with a plan and runner parameters; refused requests (unknown mode, no `send_id`); delivery checks against the modes a runner declares, with the `delivery_unsupported` refusal for an undeclared one; replies, including the admission reply's baseline; refusals with their retryability and named field | the wire crate's send and delivery-check tests |
| `baseline.json` | the `session.baseline` request, a request it refuses, the `not_yet` answer, a fresh and a changed baseline, and the non-owner refusal | the wire crate's baseline test |
| `compaction-ready.json` | the `compaction.ready` request, and extras a runner ignores | the wire crate's compaction test |
| `errors.json` | every refusal code the role defines, plus one it does not, with whether a caller retries it | the wire crate's retryability test |

Every pinned vector round-trips: decoding it and encoding the result gives
back the same JSON. Changing a vector changes the contract: bump the role
crate's version and say why in the commit.
