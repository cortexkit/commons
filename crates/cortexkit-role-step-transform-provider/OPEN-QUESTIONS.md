# Step-transform provider draft decisions

This is a review ledger, not a wire contract. MC must obtain the agreements
listed below before pinning an unresolved encoding. Partial settlements do
not settle the rest of a question: its marker and implemented draft remain.

| Question | Disposition | Options and recommendation | Agreement needed |
|---|---|---|---|
| Q1: op envelope | Open in part. Routes are module-level and unscoped (design §4.7.2, §14.1). | `{method, params}` or `{name, arguments}`. Recommend the former to match runner management ops. `role.describe` takes empty params in this draft. | BROCA |
| Q2: op and answer names | Open in part. Operations `prepend`, `append`, `replace` are settled by §6.2. Neither authority selects `transform.declare`, `transform.hook` or the JSON `answer` tags. | Keep namespaced ops and `pass`, `ops`, `mutate`, `deny`, `ask`, or adopt other dispatch/tag names. Recommend retaining the draft spellings for consistency with hook/phase snake_case. | ALF for declaration dispatch; BROCA for hook dispatch and answers |
| Q3: declaration location | Settled in substance: per-preset/params declaration, read at composition and checked again at admission, not build-level discovery or HELLO. | Pinned fetch-plan README, StepItem and `admission/refuse-subscription-loosened-tools.json`, encode `{provider, preset, params, answer: {subscriptions}}`. Exact declaration op name is still Q2, not inferred from vectors. | No further agreement on the location; Q2 still needs ALF and BROCA |
| Q6: text subjects and multiple text blocks | Open in part. PostAssistant is existing text only, never blocks/reasoning/signatures/tool calls (§6.1, §6.3). The flattening and replace mapping are not specified. | Flat text with first/last-block mapping, or per-block text subjects. Recommend preserving block boundaries (prepend first, append last; define per-block replacement before allowing multi-block replace), because collapsing blocks may violate §6.1. Keep current draft flat strings until BROCA agrees on a concrete encoding. | BROCA |
| Q7: disallowed answer | Open. §6.2 requires refusal before application, but does not say whether that refusal uses the unavailable policy. | Treat as unavailable or drop as pass. Recommend unavailable: a broken enforcing hook must not fail open. Existing draft checks and policy are retained. | BROCA |
| Q8: approve answer and ask encoding | Open in part. Pass or a human question is settled by §6.4 and pinned fetch-plan README, StepItem. §6.4 and §11.3 settle prompt/options/expiry/policies/material damage in substance, not `expiry_ms` or option encoding. | Draft `{prompt, options?: [string], expiry_ms, on_expiry, material_damage, late_execution}` or reuse an elicitation request shape. Recommend the draft projection, with `on_expiry` and `late_execution` open strings; the runner adds target, canonical args and scope when filing. Do not invent a second approval digest in this role. | ALF for elicitation-facing fields; BROCA for runner conversion |
| Q9: reduction-owner order | Contested; left open. Design §4.5, §6.2 and §6.3 and pinned fetch-plan README, Order, say plan order; existing draft and local vectors say reduction owner first in a separate list. | Owner first regardless of plan, or exact plan order with the composer placing the owner first. Recommend composer ordering: it protects preserving prepends without secretly changing plan precedence. Keep the existing owner-first helper pending agreement, rather than deleting the existing vector's claim. | ALF and BROCA |
| Q10: user-tier post_tool replace grant transport | Open. §6.2 defines grant authority and `{module, hook, tools}`, but no transport. Current plan vectors carry no grants. | Starter sends grants in the plan or runner reads user-tier policy. Recommend runner reads user-tier policy unless a separately authenticated grant field is defined; ordinary plan data must not manufacture user permission. | ALF and BROCA |
| Q12: `runner_groups` | Open in part. §6.3 and §10.4 settle the need for history reads absent an independent reader, not how discovery reports it. | Discovery field or starter configuration. Recommend `role.describe.runner_groups` for discoverable third-party requirements. | ALF and BROCA |

## Already settled questions

| Question | Decision and authority |
|---|---|
| Q4 | Declaration is source of frozen availability policy and budget; equal or tighter planned values admit. Pinned fetch-plan README, StepItem, and admission vectors. |
| Q5 | `ops` empty on pre_tool; answers depend on phase. Pinned `subscriptions.json`, README StepItem and `admission/refuse-pre-tool-append.json`. |
| Q11 | Wider tools/ops are stale, not malformed; tagged subscription differences require nullable phase. Pinned fetch-plan README, Admission outcomes, and loosened tools/ops vectors. |
| Q13 | Hook/phase snake_case vocabulary and `pre_tool_declined`. Pinned `subscriptions.json`; design §6.4–§6.5. |
| Q14 | Ordered plan items carry subscriptions bounded by declarations. Pinned fetch-plan README, StepItem; design §4.5, §6.2. |
| Q15 | One reduction owner; preserving transforms are separate. Existing contract settlement. Its relative ordering remains contested under Q9. |
| Q16 | Identity-independent fetched bytes and declaration inputs. Design §4.2, §4.5 and errata Stage-2 plan wire, Purity rule. |

The current prefrontal pin reviewed is origin/main
`473401d615547a8d5ad8c14b8e260a9c71c1ff4e`. Existing commons admission vectors
remain pinned to their historical revision; their subscription encodings agree
with the current source. Production runner-hook capability gating in that
source is runner admission behavior, not a new provider request field.
