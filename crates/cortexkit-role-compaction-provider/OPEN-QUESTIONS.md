# Compaction provider draft decisions

This is a review ledger, not a wire contract. Unresolved choices retain their
markers in CONTRACT.md and their existing draft encodings in Rust. The
authoritative sources do not select an option unless the disposition below
says settled. MC must obtain the listed agreements before pinning an open item.

| Question | Disposition | Options and recommendation | Agreement needed |
|---|---|---|---|
| Q1: op envelope and ready reply | Open in part. Routing is settled: module-level, unscoped (design §4.7.2, §14.1). Neither the design nor errata specifies these provider envelopes or the ready reply. | `{method, params}` or `{name, arguments}`; ready reply `{}` or an acknowledgement carrying an outcome. Keep `{method, params}` and `{}` to match runner management ops without making a ready hint into a query. `role.describe` uses empty params in this draft. | BROCA |
| Q2: op and answer spellings | Open. §5.1 and §5.3 name semantic operations and answers, not their wire tags. | Keep `compaction.setup`, `compaction.step`, `ready`, `noop`, `compaction_message`, `wait`, `refuse`; alternatives include `setup`/`status` and the design's uppercase labels. Recommend the existing namespaced ops and snake_case tags for consistency with the runner. | BROCA |
| Q4: who mints `compaction_id` | Open. §5.3 says the status carries an id; §5.4 assigns the version counter, not the id's minter. | Provider-minted or runner-minted. Recommend provider-minted: the initial answer can name its content before recording, with no second allocation protocol. | BROCA |
| Q6: capped status messages | Open. §5.3 requires messages since the cursor but supplies no cap or pagination shape. | Uncapped messages or a byte cap with `more`. Recommend the draft cap, a single oversized message sent alone, and advancement only to the last sent message, to bound lineage-reset requests without losing history. | BROCA |
| Q7: `last_not_applied` | Open. §5.3 requires `last_applied`, not this additional field. | Include the newest rejected message and reason, or infer from `last_applied`. Recommend including it: absence from the applied counter cannot distinguish rejection from outstanding work. | BROCA |
| Q8: `runner_groups` | Open in part. The transcript-read dependency is settled (§10.4); its discovery field is not. | Provider discovery data or starter configuration. Recommend `role.describe.runner_groups`: build-level requirements remain discoverable for third-party providers. The starter checks them before pairing. | ALF and BROCA |
| Q9: unknown conditions and model patterns | Open. §5.2 allows additive kinds and patterns without defining fallback, matching or overlap precedence. | Unknown kinds: always call, ignore, or refuse. Patterns: exact only, trailing `*`, or a broader grammar. Recommend always call for unknown kinds; exact match first, then longest trailing-star prefix, then default. This avoids skipping required work and keeps matching deterministic. No matcher is implemented until agreement. | BROCA |
| Q13: stability encoding | Open. §5.1, §7 and §13.2 settle output-relative indexes and higher-is-stabler ranks, not a JSON encoding. | Array of `{index, rank}` or object keyed by index. Recommend the draft array, with unsigned 32-bit indexes/ranks, to avoid special JSON-key parsing. Empty declarations are omitted on serialization. | BROCA |
| Q14: role-named refusal codes | Open. §5.6 settles `code`, `reason`, `retryable` and their consequences, but no provider-owned vocabulary. | The four draft codes or entirely provider-defined codes. Recommend `window_too_small`/false, `provider_busy`/true, `misconfigured`/false and `history_unreadable`/true, while keeping unknown codes open. They distinguish automatic retry from user action. | ALF and BROCA |

## Already settled questions

| Question | Decision and authority |
|---|---|
| Q3 | Half-open ordinal ranges, including empty insertions: existing runner contract §4.2 and compaction source types. The design §5.4 still describes message-id endpoints; preserve the already shared ordinal contract rather than changing runner types. |
| Q5 | Opaque string request ids and unsigned 64-bit versions: existing runner types; request fence semantics design §5.4. |
| Q10 | Empty replacement ranges are insertions using the shared model-view source: existing runner §4.2. |
| Q11 | Late answers never apply: design §5.3–§5.4 and errata BROCA timeout/fence correction, superseded 2026-10-01. |
| Q12 | `compaction_unavailable` for Setup without an answer: existing runner provider-code vocabulary; design §5.1 supplies the failure behavior. |
| Q15 | Optional runner `compaction` group and `invalid_params` admission refusal: existing runner §3, §10.1 and pinned fetch-plan README, Plan. |
| Q16 | Ready's session/request fields, caller check and hint semantics: existing runner §11.1; design §5.5. |
| Q17 | Shared model-view page and source encodings: existing runner §4.2. |
| Former provisional plan field | `plan.compaction_item.provider`: pinned fetch-plan README, Plan/Item, and `plans/pre-tool-two-phase.json` on prefrontal origin/main `473401d615547a8d5ad8c14b8e260a9c71c1ff4e`. The existing generic runner helper already reads that field; its prose still says provisional. No shared type change is needed. |

## Remaining interoperability wording

The runner's provider-code documentation still says `compaction_unavailable`
for Setup **or a compaction call**, whereas design §5.3 says a failed step call
continues with the last applied view. Recommend narrowing the runner wording
to Setup, with BROCA agreement. Its code string and this draft's behavior are
unchanged; no runner file is changed here.
