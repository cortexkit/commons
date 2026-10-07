# Step-transform provider draft decisions

This is a review ledger, not a wire contract. Every question below is
settled, and CONTRACT.md states each answer as a pinned rule; this ledger
only points to the section that holds it.

Agreements name the owner of each affected contract: Broca is the CortexKit
module that runs model sessions and owns `llm-runner/v1`; prefrontal owns the
fetch-plan composer that writes each session's plan; Magic Context is the
provider that owns this contract.

References to numbered design sections below are to
`ck-extensibility-design-r7.3.md`; its corrections are in
`ck-extensibility-r7.3-errata.md`. Fetch-plan references name sections or
files in prefrontal's `test-vectors/fetch-plan-v1/`.

## Open questions

None.

## Settled questions

| Question | Where CONTRACT.md answers it |
|---|---|
| Q1: op envelope | §1: every request is `{method, params}`. Ruled by Magic Context and Broca. |
| Q2: op and answer names | §1, §4 and §7: `transform.declare`, `transform.hook`; `pass`, `ops`, `mutate`, `deny`, `ask`. Ruled by Magic Context and Broca. |
| Q3: declaration location | §4: per preset and params, read at composition and rechecked at admission; the op is `transform.declare`. |
| Q6: text subjects and multiple text blocks | §5 and §7: `blocks` lists the subject's text blocks only; each operation names one by required `block` index and changes that block alone. Ruled by Magic Context and Broca. |
| Q7: disallowed answer | §5: the hook is unavailable for that call, and `on_unavailable` decides. Ruled by Magic Context and Broca. |
| Q8: approve answer and ask encoding | §6: `{prompt, options?, expires_at_ms, on_expiry, material_damage, late_execution}`, absolute expiry, closed `on_expiry`/`late_execution`. Ruled by Magic Context and Broca, refined by ALF. |
| Q9: reduction-owner order | §5: hooks run in exact frozen plan order; placing the reduction owner first is the composer's obligation. Ruled by ALF as the plan composer. |
| Q10: user-tier post_tool replace grant transport | §5: the plan's dedicated `user_grants` field, filled only from the user tier and frozen at admission; a grant anywhere else is ignored. Ruled by ALF as the plan composer. |
| Q12: `runner_groups` | §2: declared in `role.describe.runner_groups`, checked by the plan composer at composition; an unmet group fails the launch by name. Ruled by ALF as the plan composer. |
| Caller's harness (not numbered) | §1, §7: every hook request carries a required `harness` from the session's key; providers key a runner conversation on `(project_root, session, harness)`. Ruled by Magic Context and Broca. |

## Settled earlier

| Question | Decision and authority |
|---|---|
| Q4 | The declaration supplies frozen availability policy and budget; equal or tighter planned values admit. Fetch-plan README, StepItem section, and admission vectors. |
| Q5 | `ops` is empty on pre_tool and answers depend on phase. Fetch-plan `subscriptions.json`, README StepItem section and `admission/refuse-pre-tool-append.json`. |
| Q11 | Wider tools/ops are stale, not malformed; tagged subscription differences require nullable phase. Fetch-plan README, Admission outcomes section, and loosened tools/ops vectors. |
| Q13 | Hook and phase names use snake_case; a human decline is `pre_tool_declined`. Fetch-plan `subscriptions.json`; design §6.4–§6.5. |
| Q14 | Ordered plan items carry subscriptions bounded by declarations. Fetch-plan README, StepItem section; design §4.5, §6.2. |
| Q15 | The compaction provider is the one reduction owner; every other transform is preserving (CONTRACT.md §5). Order is settled under Q9. |
| Q16 | Fetched bytes and declarations depend on composition, preset, params and configuration, never scope, agent or session identity; design §4.2, §4.5. The errata confirms these permitted inputs and the identity exclusion. |

The fetch-plan sources above are Git objects from prefrontal origin/main commit
`473401d615547a8d5ad8c14b8e260a9c71c1ff4e`. They require a runner to declare
hook support before accepting a non-empty step-transform list (README, Plan
section); that check belongs to runner admission, not a new provider request
field. The existing commons admission vectors retain their historical source
revision and encode subscriptions the same way as those Git objects.
