# `step-transform-provider/v1` test vectors

The shared bytes every `step-transform-provider/v1` implementation and
runner checks. The role document is
`crates/cortexkit-role-step-transform-provider/CONTRACT.md`. This role is a
draft: these vectors change with the contract until it is reviewed.

| File | What it pins | Checked by |
|---|---|---|
| `role-describe.json` | canonical answers, answers a runner accepts, and answers it refuses with the problem | the wire crate's describe test |
| `declare.json` | `transform.declare` requests; declarations (tags, recall and cleanup with the reduction owner's `replace`; a guard in every `pre_tool` phase; an empty one) with the `on_unavailable` each subscription takes in effect; malformed declarations with the problem; declarations that do not decode (unknown or camel-case hook, unknown phase, op or `on_unavailable`, no budget) | the wire crate's declaration test |
| `subscriptions.json` | one declaration and planned subscriptions with the declared entry that bounds each, or the problem (not declared, tools not covered, op not declared, `ops` on `pre_tool` as the fetch-plan vectors write it, no phase); subscriptions that do not decode; the reduction rule for `replace`; the order the runner calls providers in | the wire crate's subscription test |
| `hook-requests.json` | a `transform.hook` request per hook (with and without a lineage, a steered prompt's mark, a deferred call's key), extras a provider ignores, and requests that do not decode (unknown hook, no hook, `pre_tool` without or with an unknown phase, `post_tool` without `is_error`, no session) | the wire crate's hook-request test |
| `hook-answers.json` | each answer, extras tolerated, answers that do not decode (unknown answer or op, `replace` carrying `text`, `deny` without text, `ask` without expiry), and the runner's check of an answer against its hook, phase, subscription and the tool's accepted operations | the wire crate's answer test |
| `errors.json` | every refusal code a provider answers with, plus one it does not, with whether it is retried, and the tool-result reasons the runner writes | the wire crate's error test |

Every canonical vector round-trips: decoding it and encoding the result gives
back the same JSON. Changing a vector changes the contract: bump the role
crate's version and say why in the commit.
