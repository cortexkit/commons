# `llm-runner/v1` amendment: notes for review

`CONTRACT.md` in this folder is the amended role contract. It starts from
`crates/cortexkit-role-llm-runner/CONTRACT.md` on commons master, the copy
read for this draft. Each entry below covers one changed section: what it
said, what it says now, the input that justifies the change, and who checks
it. Reviewers: **core** (prefrontal, the session owner), **MC** (Magic
Context, the compaction provider and context manager) and **Thalamus** (the
second runner). Open questions follow the entries.

Inputs, by short name:

- **contract**: the current `CONTRACT.md` and the role crate's source
  (`cortexkit-role-llm-runner/src/`);
- **suite**: the conformance crate on commons master
  (`cortexkit-role-llm-runner-conformance`: `src/report.rs` `CASES` and
  `NARROWINGS`, `src/compaction.rs`, `README.md`);
- **provider contract**: `cortexkit-role-compaction-provider/CONTRACT.md`;
- **diff**: `llm-runner-contract-diff.md`, broca's served surface against the
  contract;
- **annotations**: `llm-runner-v1-broca-annotations.md`, broca source
  citations per row at broca `f06a487`, with Thalamus's corrections;
- **rulings 1 to 6**: the decisions already settled for this amendment.

Every type or helper the amended text names was checked against the copied
crate source: `RunResult` (`error`), `run::states`, `RunState::parse`,
`RoleDescribe::declares`, `Major::serves`, `capabilities::GROUPS`,
`capabilities::session::MID_SESSION_APPENDS`, `ModelPage::check`,
`ModelPageProblem::InsertionAfterEntry`, `errors::CODES`,
`errors::is_retryable`, `errors::provider_codes::CODES`. `RefuseCode` is the
compaction-provider crate's (`cortexkit_role_compaction_provider::errors`,
imported by the suite). The new groups, their ops, `undelivered_steers`,
`provider_detail_code` and `send_retracted` have no type or constant in the
role crate yet; the text says so where it matters, and "Crate and suite
follow-ups" lists them.

## Changed sections

### Preamble (status of the open questions)

- **Old:** every numbered question is settled.
- **New:** every numbered question is settled except Q22, the parts of the
  `run.status` answer one runner serves and the role has not chosen.
- **Input:** the new §6.1 needs an `[open: Q22]` item (see that entry).
- **Checks:** core.

### §1 Role identity and addressing

- **Old:** required ops are `role.describe` and `session.baseline`;
  everything else is a declared group.
- **New:** adds that `session.baseline` is required whether or not a given
  consumer calls it, and names `run.status`, `run.cancel` and
  `session.retract` as operations that are groups, not required.
- **Input:** ruling 4 (non-universal operations become declared groups;
  `session.baseline` stays required). The diff notes core never calls
  `session.baseline` (diff §1, row `session.baseline`); the annotations
  keep it required (annotations, "Contract amendment proposals", Add).
- **Checks:** core, Thalamus.

### §3 Capability groups

- **Old:** twelve groups; no group for run status, cancellation or
  retraction.
- **New:** three more groups, `run_status` (`run.status`), `run_cancel`
  (`run.cancel`) and `retract` (`session.retract`), each declared on its
  own and independent of `run_ops` and `transcript_reads`. Because
  `capabilities::GROUPS` does not list them yet, `check_describe` cannot
  check their ops, so a consumer checks `Major::serves` as well as
  `RoleDescribe::declares` until the crate catches up.
- **Input:** ruling 4. broca serves all three and its `role.describe` omits
  them because the contract had no group (diff §1 and the paragraph under
  its table; annotations rows `run.status`, `run.cancel`,
  `session.retract`, marked "contract is wrong"). Core calls all three
  (diff §1 notes). Thalamus declares neither `run_ops` nor
  `transcript_reads` (annotations, "Keep for other producers"), so the new
  groups must not ride inside either. The group names are this draft's
  proposal.
- **Checks:** core (uses them), Thalamus (declares or omits them).

### §6 `run_ops`: run errors

- **Old:** one clause: `provider_code` carries the provider's code; for a
  compaction `REFUSE` it is the role `code`, and the answer's finer
  `provider_code` maps to `provider_detail_code`.
- **New:** four clauses. `provider_code` is the code callers branch on and
  sits in `RunResult`'s `error` with `provider` and `reason`. For a
  compaction `REFUSE` it is a `RefuseCode` value (the list is pointed at,
  not repeated) or an unknown code as received, and retry follows from it
  as the provider contract fixes it. The finer code is
  `provider_detail_code`, diagnostics only, never branched on. The text says
  in so many words that the provider wire's `provider_code` maps to the
  runner's `provider_detail_code`, so the two same-named members are not
  read as one field. A run error without a finer code omits
  `provider_detail_code`; `compaction_unavailable` never carries one.
- **Input:** ruling 1. Provider contract §11 (the mapping, retryability
  fixed by `RefuseCode`) and its "Differences from `llm-runner/v1`
  wording", third item. Suite: `compaction_unavailable_distinct_from_refuse`
  checks `provider_code`, `provider`, `reason` and `provider_detail_code`
  for Setup and step refusals (`refusal_error` in `src/compaction.rs`), and
  fails a Setup-unavailable run error that carries `provider_detail_code`.
- **Checks:** MC (provider side of the mapping), core (branches on
  `provider_code`).

### §6.1 `run_status`: `run.status` (new)

- **Old:** none. `run.status` appeared only in §9.1 as a runner-specific op.
- **New:** request `{run_id}`, strict. Answer `{state, reason?, error?,
  undelivered_steers?}`, lenient, with runner members beside it. `state`
  uses `run::states`, decoded open. `error` follows the §6 run-error rules.
  `undelivered_steers` on an ended run is present whenever delivery could
  be determined, `[]` when checked and clean, and absent only when
  delivery could not be determined; on a run that has not ended it is
  absent and means nothing. The list covers this run only; a later
  delivery shows in the send's `delivered` receipt. An unknown `run_id`,
  and a request without `run_id`, are `[open: Q22]`.
- **Input:** ruling 4 (the group) and ruling 5 (`undelivered_steers`).
  broca's served shape: annotations rows `run.status`,
  `run.status.undelivered_steers` ("contract is wrong: define terminal
  run.status undelivered_steers, including omission and episode scope"),
  `run.status.error`, `pause_reason`. broca omits the list when empty,
  which ruling 5 rules out; that is gap 2 in "Gaps in broca today". The
  "this run only" clause follows the annotations' Change item "Distinguish
  durable steer admission from model consumption". Core polls `run.status`
  (diff §1). Members broca serves that no ruling covers (`pause_reason`,
  `usage`, `retries_used`, `indeterminate_tool_calls`,
  `final_step_finish_reason`) are left as runner members; see open
  questions.
- **Checks:** core (reads it), Thalamus (whether it can determine
  delivery).

### §6.2 `run_cancel`: `run.cancel` (new)

- **Old:** none. `run.cancel` appeared only in §9.1 as a runner-specific op.
- **New:** request `{run_id}`; answer a JSON string decoded open, `ack` or
  `not_active`. `ack` accepts the cancellation and does not say which
  terminal state the run reaches; the caller reads that through
  `run.result` or `run.status`. A cancelled run ends `cancelled`, never
  `interrupted`. An expired session answers `expired`.
- **Input:** ruling 4. broca's reply strings: annotations row `run.cancel`.
  The cancel-versus-completion race: annotations, Add item 1 ("cancel-
  versus-completion and retract-versus-start races"); the answer follows
  from the existing one-terminal-per-run rule (§6, `RunState`). The
  `expired` clause moves here from §9.1.
- **Checks:** core.

### §8 `model_view`

- **Old:** at an equal anchor, insertions come before any message or
  non-empty replacement; a separate unmarked paragraph at the end of §8
  said nothing produces an insertion and a non-empty replacement at one
  anchor.
- **New:** the ordering rule stays ("insertions come first",
  `ModelPageProblem::InsertionAfterEntry`). Its two halves are named: the
  insertion before a message has a v1 producer (Setup's `[0, 0)`); the
  insertion before a non-empty replacement has no v1 producer, with the
  reasons, and is still refused by `ModelPage::check`. The unmarked
  paragraph is folded into that clause and removed. A new clause: a runner
  applies at most one view per step, and a second view for a step never
  changes the model view, whatever its version.
- **Input:** ruling 3. Suite `NARROWINGS` ("insertion-before-nonempty-
  replacement ordering has no v1 producer ... only the insertion-before-
  message half is live conformance") and `README.md`; case
  `model_view_half_open_ranges_and_travel` (head insertions precede message
  zero); case `compaction_one_view_per_step` ("a second view for the same
  step is rejected even with a higher version and before the deadline").
  Provider contract §8 and §12. The crate's vectors already cover both
  halves (`model_page_refuses_insertion_after_its_replacement` in
  `read.rs`).
- **Checks:** MC.

### §9.1 `retention`, Reads paragraph

- **Old:** a runner that serves `run.status`, `run.cancel` or
  `session.retract` answers `expired`; "these ops are runner-specific, so
  the conformance suite does not probe them".
- **New:** the same refusals, keyed to declaring `run_status`, `run_cancel`
  and `retract`; the suite probes `run.status` only.
- **Input:** ruling 4. Suite case `retention_run_status_expired` runs when
  the major lists `run.status` (`src/retention.rs`, `m.serves(STATUS)`) and
  probes no other of the three.
- **Checks:** core.

### §9.2 `retract`: `session.retract` (new)

- **Old:** none. `session.retract` appeared only in §9.1.
- **New:** request `{submission_id}` (the id a `pending` reply named);
  answer a JSON string decoded open: `retracted`, `already_started` or
  `not_pending`. A submission is retracted or started, never both. A
  re-send of a retracted send with the same `send_id` and payload is
  refused `send_retracted` and writes nothing; a different payload is
  `send_id_reuse` as before. An expired session answers `expired`.
- **Input:** ruling 4. broca's reply strings: annotations row
  `session.retract`; `send_retracted`: annotations row `send_retracted`
  ("keyed submission already durably retracted") and diff §4. The
  retract-versus-start race: annotations, Add item 1.
- **Checks:** core.

### §10 Session-level capabilities

- **Old:** session-level capabilities are frozen for the session;
  `mid_session_appends` is re-evaluated only on a model switch.
- **New:** frozen, with one exception: `mid_session_appends` depends on the
  session's model (`capabilities::session::MID_SESSION_APPENDS`), so a
  runner whose sessions can switch model re-evaluates it on a switch and at
  no other time, and the owner reads the current value through
  `session.baseline` after a switch.
- **Input:** Thalamus's correction in the annotations ("Keep for other
  producers", third item): on Claude Code `mid_session_appends` is true on
  one model and false on another, a head can switch model with `/model`,
  and the baseline is where a consumer learns it. The crate's doc comment on
  `MID_SESSION_APPENDS` already says it depends on the model.
- **Checks:** Thalamus, core.

### §11.1 The compaction interface

- **Old:** `REFUSE` maps the role code and finer code in one clause; a
  failed or timed-out step call is not a refusal, and
  `compaction_unavailable` is reserved for Setup failure.
- **New:** `REFUSE` points to §6 for the mapping, repeats that the
  provider wire's `provider_code` becomes the run error's
  `provider_detail_code`, and says no model call is made for the refused
  step and the next send calls the provider again. The step-call clause
  says the run does not end `compaction_unavailable`. A separate clause
  makes `compaction_unavailable` Setup-only: no model call, no
  `provider_detail_code`, the session stays usable and the next send calls
  Setup again.
- **Input:** rulings 1 and 2. Suite `compaction_unavailable_distinct_from_refuse`
  (no model input after a Setup failure or a step `REFUSE`; a later send
  makes a second Setup call and one model call) and
  `compaction_step_timeout_uses_last_view` (a timed-out step proceeds with
  the last view and the run completes). Provider contract §4 ("the next
  send calls Setup again") and §11 ("the next send calls the provider
  again").
- **Checks:** MC.

### §12.1 Refusals

- **Old:** `unknown_run` only for `run.result`; `expired` mentions
  `run.status` "if served"; no `send_retracted`.
- **New:** `unknown_run` also for `run.status` (marked open: Q22);
  `expired` names the three ops where their groups are declared; new row
  `send_retracted`, not retryable. A clause says `send_retracted` is not yet
  in `errors::CODES` and `errors::is_retryable` already answers false for
  it.
- **Input:** ruling 4 and the §6.1 and §9.2 entries above. broca emits
  `send_retracted` (diff §4; annotations row `send_retracted`).
- **Checks:** core.

### §12.2 Run errors

- **Old:** a table of three provider codes; `compaction_unavailable`:
  "Setup failed or timed out with no answer; no model call is made".
- **New:** an opening sentence: `provider_code` is the code callers branch
  on, `provider_detail_code` never decides anything. The
  `compaction_unavailable` row says "Setup only", carries no
  `provider_detail_code`, and is never written after a step call. A
  paragraph points at the provider contract's `RefuseCode` list for
  `REFUSE` codes without repeating it, and leaves the compaction codes'
  retryability to that contract.
- **Input:** rulings 1 and 2. The copied `CONTRACT.md` already lacked the
  words "or a compaction call" in this table; they survive in the crate's
  doc comment on `errors::provider_codes::COMPACTION_UNAVAILABLE` ("Setup or
  a compaction call failed or timed out with no answer") and in the vector
  case name "compaction call unavailable with no answer" in
  `run-result.json`. Provider contract §14.2 and §14.3, and its
  "Differences" second item.
- **Checks:** MC, core.

### §13 Decoding

- **Old:** lenient replies and strict requests listed without the new ops.
- **New:** the `run.status`, `run.cancel` and `session.retract` answers are
  lenient (open strings for the two string answers); the `run.status`
  request is strict.
- **Input:** ruling 4. broca reads `run.status` strictly (annotations row
  `run.status`: "Reads strict optional `run_id`").
- **Checks:** core.

### §15 Conformance suite

- **Old:** no rows for `guaranteed_steer_never_pending_or_unknown` or
  `resend_steer_delivered_stable`; `retention_run_status_expired` required
  "`run.status` served"; the closing paragraph said no live case can check
  insertion-versus-replacement ordering.
- **New:** rows for the two steer-receipt cases, with the requirements the
  suite gives them; `retention_run_status_expired` is not applicable unless
  the major lists `run.status`; one planned case,
  `run_status_undelivered_steers_present_when_clean` (requires
  `run_status`, `queue`), for ruling 5; the closing paragraph says the
  insertion-before-message half is live
  (`model_view_half_open_ranges_and_travel`) and the
  insertion-before-replacement half is tested against vectors only.
- **Input:** suite `CASES` and `README.md` ("Cases in this subset": both
  steer-receipt cases require `queue`, `transcript_reads`, and are
  inapplicable without `steer`; the guaranteed case also on a `confirm`
  runner or without a held tool call). Ruling 6 (forward-only receipts) is
  what `resend_steer_delivered_stable` checks. Ruling 3 for the paragraph.
  The planned case is this draft's proposal; it is not in the suite.
- **Checks:** MC (ordering paragraph), core and Thalamus (steer receipts,
  `run_status`).

### Open questions: Q22 (new)

- **Old:** Q1 to Q21, all settled.
- **New:** Q22, `run.status` beyond its pinned members: how an unknown
  `run_id` is answered and whether a request without `run_id` (a
  session-level status) is part of the role. Options listed; option (a) is
  drafted.
- **Input:** annotations rows `run.status`, `state: unknown` / future
  states and `unknown_run` (broca answers `state: "unknown"` for an
  unknown run on `run.status` and refuses `unknown_run` on `run.result`).
  No ruling covers it, and Thalamus's behaviour is not in the inputs.
- **Checks:** core, Thalamus.

### Gaps in broca today

- **Old:** sixteen gaps describing an older broca: no `role.describe`, no
  `session.baseline`, no `session.head`, no `run.result`, count-only pages,
  build-plan slice references.
- **New:** eleven gaps at broca `f06a487`: undeclared `run.status`,
  `run.cancel` and `session.retract`; `run.status` against §6.1 (unknown
  run, omitted empty `undelivered_steers`); no per-message run attribution
  despite `run_ops`; no `session_capabilities` in the baseline; legacy
  lineages without `lineage_id`; optional `send_id` and `prompt_blocks`;
  `mark` not recorded; steer and interrupt inheriting a fixed parameter set;
  queue behind a restart pause answered `pending`; `send_id_reuse` without
  `detail.field` for aggregate mismatches; the `paused` send state. Optional
  groups broca does not declare are stated not to be gaps.
- **Input:** the annotations, row by row, with their broca citations
  (`role.describe`, `run.status`, `run.cancel`, `session.retract`, Message
  `run` attribution, `baseline.session_capabilities`, Page `lineage_id`,
  `prompt`, `send_id`, `mark`, Omitted params on steer, `run_paused`,
  `send_id_reuse`, Send `paused`). The old list contradicts the
  annotations (broca now serves `role.describe`, `session.baseline`,
  `session.head`, `run.result`, `max_bytes`, `after_mid` and
  `lineage_id`), so it is replaced rather than edited. Items the
  annotations do not evidence (subscribe strictness, build-plan slices) are
  dropped.
- **Checks:** core (it reads broca), MC and Thalamus for awareness only.

### Unchanged on purpose

- **§9 `delivered` receipts (ruling 6).** "A re-send's `delivered` moves
  forward only" is already in §9 and matches the suite
  (`resend_steer_delivered_stable`, `delivered_move_allowed` in
  `src/cases.rs`, and the narrowing that absent may move to `pending`). No
  text change.
- **§3 `interrupt` as its own group.** Thalamus advertises `steer` and
  `queue` and not `interrupt` (annotations, "Keep for other producers");
  §3 already lets a runner steer and queue without `interrupt`.
- **§2 `steer_receipt: confirm`.** Kept for Thalamus (annotations, same
  section).

## Open questions

Each is a difference between runners or a consumer need that no ruling
settles. The draft writes none of them into the contract as law.

1. **Q22, `run.status` unknown run and session-level mode.** In the
   contract. broca answers an unknown run `{state: "unknown"}` and serves a
   session-level status for `{}`. Core and Thalamus: which option?
2. **Pause reasons and `pause_reason`.** broca's paused run status carries
   `reason` (`auth_required` or `restart`) and, for `restart` only, a
   separate `pause_reason`; its stream uses `reason` only (annotations rows
   `pause_reason`, Pause `auth_required`, Pause `restart`). Does the role
   name pause reasons, and which member carries them? (diff question 6.)
3. **`paused` as a `session.send` state.** broca answers it to a keyed
   retry of a restart-paused run; core's decoder rejects it (annotations
   row Send `paused`). Name it in `send::states`, or keep it a runner value
   decoded open? (diff question 5.)
4. **`run_paused` versus queueing behind a pause.** broca refuses a send
   behind an authentication pause and queues one behind a restart pause
   (annotations row `run_paused`). Should §12.1 say "a runner may queue
   behind a paused run, answering `pending`"?
5. **Steer inheritance.** §9 says a steer or interrupt takes every omitted
   runner parameter from the frozen values. broca inherits a fixed set and
   keeps `keep_warm` and `on_restart` per send (annotations row Omitted
   params on steer). Narrow §9 to "every parameter the runner freezes for
   the session"? Thalamus's behaviour is not in the inputs.
6. **`send_id` required and `prompt` required.** broca accepts both absent
   (`prompt_blocks` replaces `prompt`; unkeyed sends are admitted). Core
   needs a stable `send_id` for recovery (annotations, Change item 2).
   Keep both required of owner sends, or allow unkeyed non-owner sends?
   (diff question 3.)
7. **`mark`.** broca ignores it; core never sends it (diff §2). Keep it in
   the role? (diff question 4.)
8. **`lineage_id` on legacy lineages.** broca omits it until the next write
   (annotations row Page `lineage_id`). The suite's
   `lineage_id_on_every_page` requires it on every page of a written
   session, so this draft does not relax §4.2; relaxing it needs the suite
   changed with it.
9. **`lineage_state` on a page.** broca puts a required latest-run summary
   on every page and marks the contract wrong for not defining it
   (annotations row `lineage_state`). Does a consumer rely on it, or is
   `session.head`'s `last_run_state` enough?
10. **Provider codes and the run error's schema.** broca writes
    `response_deadline`, `account_limit_exhausted`, `account_changed` and
    `restart_resume_refused`, and a typed error with `class`, `message`,
    `status`, `retry_after_secs` (fractional) and account reset fields
    (annotations "Reasons and provider codes"; diff §5). Core branches on
    some of them (diff question 6). Which does the role name, and does it
    type the run error beyond `provider_code`, `provider`, `reason` and
    `provider_detail_code`?
11. **`run.cancel` and `session.retract` details.** Is a paused run
    cancellable (`ack`)? Are both owner-only, like `session.send`? broca's
    behaviour on either is not in the inputs.
12. **`undelivered_steers` on a `guaranteed` runner.** §2 says a durably
    accepted steer on a `guaranteed` runner is delivered; broca, which
    declares `guaranteed`, lists steers a run never rendered (annotations,
    Change item 4). Is every listed steer delivered later by a turn, or can
    a run's end drop it? If it can, broca's `steer_receipt` should be
    `confirm`.
13. **Other `run.status` members.** `indeterminate_tool_calls` (absent
    versus `[]`), `final_step_finish_reason`, `usage`, `retries_used`
    (annotations rows of those names). Runner members for now; does core
    need any of them pinned?
14. **`session.warm`.** broca serves it; no consumer dependency is claimed
    (annotations row `session.warm`). Not made a group here. A
    `session_warm` group if a second consumer appears?
15. **`plan.compaction_item.provider` still provisional.** The provider
    contract says the field is settled and the crate's
    `plan_compaction_provider` already reads it ("Differences", first
    item). §10.1 still marks it provisional pending the fetch-plan section.
    No ruling covers it; promote it to pinned?
16. **`send_id_reuse` `detail.field`.** broca names the differing field
    when it can identify one and omits it for aggregate or legacy identity
    mismatches (annotations row `send_id_reuse`). Should §9 say "names the
    field when the runner can identify it"?
17. **Group names.** `run_status`, `run_cancel` and `retract` are this
    draft's spellings. SUBC may rename before the crate adds them.

## Crate and suite follow-ups

Not contract text; listed so the merge carries them.

- `cortexkit-role-llm-runner`: add op constants for `run.status`,
  `run.cancel` and `session.retract`, the three groups in
  `capabilities::GROUPS`, request and answer types for them, a typed
  `provider_detail_code` (today `RunResult::error` is untyped JSON),
  `send_retracted` in `errors::CODES`, and change the doc comment on
  `errors::provider_codes::COMPACTION_UNAVAILABLE` to Setup-only. The
  `run-result.json` case named "compaction call unavailable with no answer"
  should be renamed to say Setup.
- `cortexkit-role-llm-runner-conformance`: the narrowing
  "retention_run_status_expired is inapplicable if run.status is not
  advertised; run.status remains runner-specific" should drop "remains
  runner-specific"; the planned case
  `run_status_undelivered_steers_present_when_clean` does not exist yet.
- `cortexkit-role-compaction-provider/CONTRACT.md`: its "Differences from
  `llm-runner/v1` wording" section's second and third items are resolved by
  this amendment once it merges.
