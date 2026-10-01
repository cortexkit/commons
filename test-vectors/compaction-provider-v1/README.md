# `compaction-provider/v1` test vectors

The shared bytes every `compaction-provider/v1` implementation and runner
checks. The role document is
`crates/cortexkit-role-compaction-provider/CONTRACT.md`. This role is a
draft: these vectors change with the contract until it is reviewed.

| File | What it pins | Checked by |
|---|---|---|
| `role-describe.json` | canonical answers, answers a runner accepts (a missing `stability` read as `alpha`, unknown fields and ops tolerated, the runner groups a provider needs met or unmet), and answers it refuses with the problem: not an object, no majors, no `compaction-provider/v1` major, a missing required op | the wire crate's describe test |
| `setup.json` | `compaction.setup` requests (fresh, and with a first message already written), `ready` answers (a two-message head before every message with stability ranks and `call_when`, an empty view, an unknown condition kind), a `refuse` answer, answers that do not decode (unknown answer, no initial message, no request id, no `retryable`, a negative rank), and `call_when` shares out of range | the wire crate's setup test |
| `status.json` | `compaction.step` statuses (first call with no cursor, a tool step with usage, after an overflow, a prefix rebuild, a refused last answer, a byte-capped message list) with the next version a provider may use; runner extras a provider ignores; statuses that do not decode; statuses whose messages break the cursor rules | the wire crate's status test |
| `answers.json` | each answer (`noop`, two CompactionMessages, `wait`, two `refuse`s, opaque ids and the largest `u64` version), unknown fields tolerated, answers that do not decode (unknown or uppercase `answer`, no request id, no version, a range named by message ids, a negative version, `wait` without a bound), an inverted range, and the model-view `source` each CompactionMessage maps to (none for an empty range) | the wire crate's answer and model-view tests |
| `fence.json` | the runner's decision for an answer given its newest request, last applied version and newest ordinal: act, `superseded_request` for every kind of delayed answer (including one with a higher version), `stale_version`, `range_beyond_newest`, `range_inverted` | the wire crate's fence test |
| `ready.json` | the `compaction.ready` request, extras the runner ignores, requests that do not decode, and the runner's check: call again, ignored, or refused `not_session_compaction_provider` by the provider at `plan.compaction_item.provider` | the wire crate's ready test, through `llm-runner/v1`'s check |
| `errors.json` | every refusal code a provider answers with, plus one it does not, with whether it is retried; the role-named `refuse` codes with the `retryable` each must carry, and a provider's own code, whose `retryable` is its own | the wire crate's error test |

Every canonical vector round-trips: decoding it and encoding the result gives
back the same JSON. Changing a vector changes the contract: bump the role
crate's version and say why in the commit.
