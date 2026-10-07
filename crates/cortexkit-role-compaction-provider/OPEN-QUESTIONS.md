# Compaction provider draft decisions

This is a review ledger, not a wire contract. Every question below is
settled, and CONTRACT.md states each answer as a pinned rule; this ledger
only points to the section that holds it.

Agreements name the owner of each affected contract: Broca is the CortexKit
module that runs model sessions and owns `llm-runner/v1`; prefrontal owns the
fetch-plan composer that writes each session's plan; Magic Context is the
provider that owns this contract.

References to numbered design sections below are to
`ck-extensibility-design-r7.3.md`; its corrections are in
`ck-extensibility-r7.3-errata.md`. Runner-contract sections refer to
`cortexkit-role-llm-runner/CONTRACT.md`. Fetch-plan references name sections
or files in prefrontal's `test-vectors/fetch-plan-v1/`.

## Open questions

None.

## Settled questions

| Question | Where CONTRACT.md answers it |
|---|---|
| Q1: op envelope and ready reply | §1: every request is `{method, params}`; §10: `compaction.ready` is answered `{}`. Ruled by Magic Context and Broca. |
| Q2: op and answer spellings | §1 and §7: `compaction.setup`, `compaction.step`; `ready`, `noop`, `compaction_message`, `wait`, `refuse`. Ruled by Magic Context and Broca. |
| Q4: who mints `compaction_id` | §8: the provider; opaque, stable for one logical compaction lineage. Ruled by Magic Context and Broca. |
| Q6: capped status messages | §6: a byte cap (default 4 MiB) with `more`; a single oversized message is sent alone; the cursor advances to the last message sent. Ruled by Magic Context and Broca. |
| Q7: `last_not_applied` | §6: in the status, with its reason. Ruled by Magic Context and Broca. |
| Q8: `runner_groups` | §2: declared in `role.describe.runner_groups`, checked by the plan composer at composition; an unmet group fails the launch by name. Ruled by ALF as the plan composer. |
| Q9: unknown conditions and model patterns | §5: an unknown condition kind means every step; exact id, then the longest trailing-`*` pattern, then `default`. Ruled by Magic Context and Broca. |
| Q13: stability encoding | §4: an array of `{index, rank}`. Ruled by Magic Context and Broca. |
| Q14: role-named refusal codes | §11 and §14.3: four role codes with fixed retryability, the provider's own reason in `provider_code`, no `retryable` on the wire; `compaction_unavailable` only after Setup. Ruled by Magic Context and Broca. |

## Settled earlier

| Question | Decision and authority |
|---|---|
| Q3 | Half-open ordinal ranges, including empty insertions: existing runner contract §4.2 and compaction source types. The design §5.4 still describes message-id endpoints; preserve the already shared ordinal contract rather than changing runner types. |
| Q5 | Request ids are opaque strings and versions unsigned 64-bit integers in the existing runner types. Answers must name the newest issued request; design §5.4. |
| Q10 | Empty replacement ranges are insertions using the shared model-view source: existing runner §4.2. |
| Q11 | A late answer is recorded but never applied, regardless of version; only an answer to the newest issued request can apply. The errata removes the earlier allowance for applying late answers at a later boundary; design §5.3–§5.4. |
| Q12 | `compaction_unavailable` for Setup without an answer: existing runner provider-code vocabulary; design §5.1 supplies the failure behavior. |
| Q15 | A runner without the optional `compaction` group refuses a compaction plan item during admission, the plan check before accepting a session, with `invalid_params`; runner contract §3, §10.1 and fetch-plan README, Plan section. |
| Q16 | Ready names the session and request; the runner verifies the module that opened the route and treats ready as a hint to ask again, not as compaction output. Runner contract §11.1; design §5.5. |
| Q17 | Shared model-view page and source encodings: existing runner §4.2. |
| Former provisional plan field | The plan names its compaction provider at `plan.compaction_item.provider`; fetch-plan README, Plan and Item sections, and `plans/pre-tool-two-phase.json` at prefrontal origin/main commit `473401d615547a8d5ad8c14b8e260a9c71c1ff4e`. The existing generic runner helper already reads that field; the helper's doc comment in `cortexkit-role-llm-runner` still calls it provisional. No shared type change is needed. |

## Remaining interoperability wording

These sit in `cortexkit-role-llm-runner/CONTRACT.md`, which this crate does
not change; its owner (Broca) aligns them:

- §12.2 still says `compaction_unavailable` is written for Setup **or a
  compaction call**. This role uses it after Setup only (§14.3); a failed
  step call continues with the last applied view (design §5.3).
- §6 and §11 say a provider's refusal rides as the run error's
  `provider_code`. Under §11 of this contract the run records the role
  `code` and, separately, the refusal's own `provider_code`; the runner
  contract has to name the member that holds each.
