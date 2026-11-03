# step-transform-provider/v2 vectors

Canonical requests and answers round-trip through the public wire types.
Unknown fields are tolerated. Unknown values round-trip through the open wire
types. Permission-bearing unknown values are refused semantically; discovery
and stale-difference additions use the safe fallbacks in CONTRACT.md §8.
Malformed known variants do not decode.

| File | Coverage |
|---|---|
| `role-describe.json` | required v2 major/ops, discovery extensions and runner groups |
| `declare.json` | preset/params preflight, declaration bounds and malformed policies |
| `subscriptions.json` | frozen bounds, admission precedence, reduction ownership and provider ordering |
| `hook-requests.json` | all four hooks, phases, text blocks, prompt mark/delivery, trace and malformed subjects |
| `hook-answers.json` | operations, phase authorization, tool limits and unsupported answers |
| `subject-fields.json` | opaque whole messages, subject identity pairing and 1-based ordinals |
| `grants.json` | user-tier post_tool replace grants only in the dedicated plan field |
| `routes.json` | session route hooks, non-session per-call refusal and session-independent preflight |
| `errors.json` | provider codes and shared runner result reasons |

Entry ordinals follow compaction W4.1/W4.2 and start at 1; text-block indexes
start at 0. A message id is independent of its ordinal. Hook bodies carry no
session or caller identity; route vectors describe daemon metadata separately.
`mark` and `delivery` retain prompt provenance; `runner_trace` is opaque on any
hook. Absent optional fields are omitted. Present empty values remain where
their meaning requires them; `message` and `runner_trace` additionally
distinguish explicit JSON null from absence.

`SHA256SUMS` covers every corpus file, including this README. The feature-gated
`vectors` module embeds the same bytes shipped in the package.
