# Compaction provider draft decisions

This is a review ledger, not a wire contract. Unresolved choices retain their
markers in CONTRACT.md and their existing draft encodings in Rust. The
authoritative sources do not select an option unless the disposition below
says settled. The owner of this contract (Magic Context) must obtain the
listed agreements before pinning an open item. Pinning means removing its
`[open: Qn]` marker and freezing the wire spelling.

Agreements name the owner of each affected contract: Broca is the CortexKit
module that runs model sessions and owns `llm-runner/v1`; prefrontal owns the
fetch-plan composer that writes each session's plan.

References to numbered design sections below are to
`ck-extensibility-design-r7.3.md`; its corrections are in
`ck-extensibility-r7.3-errata.md`. Runner-contract sections refer to
`cortexkit-role-llm-runner/CONTRACT.md`. Fetch-plan references name sections
or files in prefrontal's `test-vectors/fetch-plan-v1/`.

| Question | Disposition | Options and recommendation | Agreement needed |
|---|---|---|---|
| Q1: op envelope and ready reply | Open in part. Routes are module-level and unscoped; design §4.7.2, §14.1. Neither the design nor errata specifies these provider envelopes or the ready reply. | `{method, params}` or `{name, arguments}`; ready reply `{}` or an acknowledgement carrying an outcome. Keep `{method, params}` and `{}` to match runner management ops without making a ready hint into a query. `role.describe` uses empty params in this draft. | Owner of `llm-runner/v1` (Broca) |
| Q2: op and answer spellings | Open. Setup and the per-step call have defined meanings, but not wire tags; design §5.1, §5.3. | Keep `compaction.setup`, `compaction.step`, `ready`, `noop`, `compaction_message`, `wait`, `refuse`; alternatives include `setup`/`status` and the design's uppercase labels. Recommend the existing namespaced ops and snake_case tags for consistency with the runner. | Owner of `llm-runner/v1` (Broca) |
| Q4: who mints `compaction_id` | Open. The status carries an id and the provider maintains the version counter, but no rule selects the id's allocator; design §5.3, §5.4. | Provider-minted or runner-minted. Recommend provider-minted: the initial answer can name its content before recording, with no second allocation protocol. | Owner of `llm-runner/v1` (Broca) |
| Q6: capped status messages | Open. The runner sends messages since the cursor, but no cap or pagination shape is specified; design §5.3. | Uncapped messages or a byte cap with `more`. Recommend the draft cap, a single oversized message sent alone, and advancement only to the last sent message, to bound lineage-reset requests without losing history. | Owner of `llm-runner/v1` (Broca) |
| Q7: `last_not_applied` | Open. The status must identify the last applied compaction, but no rule requires this additional field; design §5.3. | Include the newest rejected message and reason, or infer from `last_applied`. Recommend including it: absence from the applied counter cannot distinguish rejection from outstanding work. | Owner of `llm-runner/v1` (Broca) |
| Q8: `runner_groups` | Open in part. A provider without its own transcript reader needs runner transcript reads, but the discovery field is unspecified; design §10.4. | Provider discovery data or starter configuration. Recommend `role.describe.runner_groups`: build-level requirements remain discoverable for third-party providers. The starter checks them before pairing. | Owner of the fetch-plan composer (prefrontal) and owner of `llm-runner/v1` (Broca) |
| Q9: unknown conditions and model patterns | Open. Condition kinds may be added and model overrides may use patterns, but fallback, matching and overlap precedence are unspecified; design §5.2. | Unknown kinds: always call, ignore, or refuse. Patterns: exact only, trailing `*`, or a broader grammar. Recommend always call for unknown kinds; exact match first, then longest trailing-star prefix, then default. This avoids skipping required work and keeps matching deterministic. No matcher is implemented until agreement. | Owner of `llm-runner/v1` (Broca) |
| Q13: stability encoding | Open. Indexes refer to provider output and higher ranks mean more stable messages, but no JSON encoding is selected; design §5.1, §7, §13.2. | Array of `{index, rank}` or object keyed by index. Recommend the draft array, with unsigned 32-bit indexes/ranks, to avoid special JSON-key parsing. Empty declarations are omitted on serialization. | Owner of `llm-runner/v1` (Broca) |
| Q14: role-named refusal codes | Open. A refusal carries `code`, `reason` and `retryable` and ends the run, but no provider-owned code vocabulary is defined; design §5.6. | The four draft codes or entirely provider-defined codes. Recommend `window_too_small`/false, `provider_busy`/true, `misconfigured`/false and `history_unreadable`/true, while keeping unknown codes open. They distinguish automatic retry from user action. | Owner of the fetch-plan composer (prefrontal) and owner of `llm-runner/v1` (Broca) |

## Already settled questions

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

The runner's provider-code documentation still says `compaction_unavailable`
for Setup **or a compaction call**, whereas a failed step call continues with
the last applied view (design §5.3). Recommend narrowing the runner wording
to Setup, with agreement from the owner of `llm-runner/v1` (Broca). Its code
string and this draft's behavior are unchanged; no runner file is changed here.
