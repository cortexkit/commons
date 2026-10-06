# Step-transform provider draft decisions

This is a review ledger, not a wire contract. The owner of this contract
(Magic Context) must obtain the agreements listed below before pinning an
unresolved encoding. Pinning means removing its `[open: Qn]` marker and
freezing the wire spelling. Partial settlements do not settle the rest of
a question: its marker and implemented draft remain.

Agreements name the owner of each affected contract: Broca is the CortexKit
module that runs model sessions and owns `llm-runner/v1`; prefrontal owns the
fetch-plan composer that writes each session's plan.

References to numbered design sections below are to
`ck-extensibility-design-r7.3.md`; its corrections are in
`ck-extensibility-r7.3-errata.md`. Fetch-plan references name sections or
files in prefrontal's `test-vectors/fetch-plan-v1/`.

| Question | Disposition | Options and recommendation | Agreement needed |
|---|---|---|---|
| Q1: op envelope | Open in part. Routes are module-level and unscoped; design §4.7.2, §14.1. | `{method, params}` or `{name, arguments}`. Recommend the former to match runner management ops. `role.describe` takes empty params in this draft. | Owner of `llm-runner/v1` (Broca) |
| Q2: op and answer names | Open in part. Text hooks return `prepend`, `append` and `replace` operations; design §6.2. Neither the design nor errata selects `transform.declare`, `transform.hook` or the JSON `answer` tags. | Keep namespaced ops and `pass`, `ops`, `mutate`, `deny`, `ask`, or adopt other dispatch/tag names. Recommend retaining the draft spellings for consistency with hook/phase snake_case. | Owner of the fetch-plan composer (prefrontal) for declaration dispatch; owner of `llm-runner/v1` (Broca) for hook dispatch and answers |
| Q3: declaration location | Settled in substance: per-preset/params declaration, read while composing the plan and checked again at admission, the plan check before accepting a session. It is not build-level discovery or HELLO. | The declaration exchange encodes `{provider, preset, params, answer: {subscriptions}}`; fetch-plan README, StepItem section, and `admission/refuse-subscription-loosened-tools.json`. The declaration op's name (drafted as `transform.declare`) is still open under Q2; the test vectors carry the declaration's content, not its op name. | No further agreement on location; Q2 still needs the owner of the fetch-plan composer (prefrontal) and owner of `llm-runner/v1` (Broca) |
| Q6: text subjects and multiple text blocks | Open in part. PostAssistant acts on existing text only, never blocks, reasoning, signatures or tool calls; design §6.1, §6.3. Flattening and replace mapping are unspecified. | Flat text with first/last-block mapping, or per-block text subjects. Recommend preserving block boundaries (prepend first, append last; define per-block replacement before allowing multi-block replace), because changing block structure may conflict with the text-only restriction (design §6.1). Keep draft flat strings until the owner of `llm-runner/v1` (Broca) agrees on a concrete encoding. | Owner of `llm-runner/v1` (Broca) |
| Q7: disallowed answer | Open. Disallowed operations must be refused before application, but whether that refusal uses the unavailable policy is unspecified; design §6.2. | Treat as unavailable or drop as pass. Recommend unavailable: a broken enforcing hook must not fail open. Existing draft checks and policy are retained. | Owner of `llm-runner/v1` (Broca) |
| Q8: approve answer and ask encoding | Open in part. An approve hook passes or asks a human; design §6.4 and fetch-plan README, StepItem section. The question includes prompt, options, expiry, policies and material-damage information, but `expiry_ms` and option encoding are unspecified; design §6.4, §11.3. | Draft `{prompt, options?: [string], expiry_ms, on_expiry, material_damage, late_execution}` or reuse an elicitation request shape. Recommend the draft projection, with `on_expiry` and `late_execution` open strings; the runner adds target, canonical args and scope when filing. Do not invent a second approval digest in this role. | Owner of the fetch-plan composer (prefrontal) for elicitation-facing fields; owner of `llm-runner/v1` (Broca) for runner conversion |
| Q9: reduction-owner order | Contested; left open. The reduction owner is the session's compaction provider, which alone may remove or rewrite history (CONTRACT.md §5). Providers run in plan order according to design §4.5, §6.2, §6.3 and fetch-plan README, Order section; the draft and local vectors instead put the reduction owner first in a separate list. | Owner first regardless of plan, or exact plan order with the composer placing the owner first. Recommend composer ordering: it protects preserving prepends without secretly changing plan precedence. Keep the existing owner-first helper pending agreement, rather than deleting the existing vector's claim. | Owner of the fetch-plan composer (prefrontal) and owner of `llm-runner/v1` (Broca) |
| Q10: user-tier post_tool replace grant transport | Open. Only the user tier grants replacement permission, using `{module, hook, tools}`, but its transport is unspecified; design §6.2. Current plan vectors carry no grants. | Starter sends grants in the plan or runner reads user-tier policy. Recommend runner reads user-tier policy unless a separately authenticated grant field is defined; ordinary plan data must not manufacture user permission. | Owner of the fetch-plan composer (prefrontal) and owner of `llm-runner/v1` (Broca) |
| Q12: `runner_groups` | Open in part. A provider without an independent transcript reader needs runner history reads, but discovery of that requirement is unspecified; design §6.3, §10.4. | Discovery field or starter configuration. Recommend `role.describe.runner_groups` for discoverable third-party requirements. | Owner of the fetch-plan composer (prefrontal) and owner of `llm-runner/v1` (Broca) |

## Already settled questions

| Question | Decision and authority |
|---|---|
| Q4 | The declaration supplies frozen availability policy and budget; equal or tighter planned values admit. Fetch-plan README, StepItem section, and admission vectors. |
| Q5 | `ops` is empty on pre_tool and answers depend on phase. Fetch-plan `subscriptions.json`, README StepItem section and `admission/refuse-pre-tool-append.json`. |
| Q11 | Wider tools/ops are stale, not malformed; tagged subscription differences require nullable phase. Fetch-plan README, Admission outcomes section, and loosened tools/ops vectors. |
| Q13 | Hook and phase names use snake_case; a human decline is `pre_tool_declined`. Fetch-plan `subscriptions.json`; design §6.4–§6.5. |
| Q14 | Ordered plan items carry subscriptions bounded by declarations. Fetch-plan README, StepItem section; design §4.5, §6.2. |
| Q15 | The compaction provider is the one reduction owner; preserving transforms are separate (CONTRACT.md §5). Whether the reduction owner's hooks run before preserving transforms' hooks, or every hook runs in plan order, remains contested under Q9. |
| Q16 | Fetched bytes and declarations depend on composition, preset, params and configuration, never scope, agent or session identity; design §4.2, §4.5. The errata confirms these permitted inputs and the identity exclusion. |

The fetch-plan sources above are Git objects from prefrontal origin/main commit
`473401d615547a8d5ad8c14b8e260a9c71c1ff4e`. They require a runner to declare
hook support before accepting a non-empty step-transform list (README, Plan
section); that check belongs to runner admission, not a new provider request
field. The existing commons admission vectors retain their historical source
revision and encode subscriptions the same way as those Git objects.
