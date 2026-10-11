# `llm-runner/v1` amendment: notes for review

`CONTRACT.md` in this folder is the amended role contract. It starts from
`crates/cortexkit-role-llm-runner/CONTRACT.md` on commons master, the copy
read for this draft. Each entry below covers one changed section: what it
said, what it says now, the input that justifies the change, and who checks
it. Reviewers: **core** (prefrontal, the session owner), **MC** (Magic
Context, the compaction provider and context manager) and **Thalamus** (the
second runner). The "Rulings" section below records the agreed answers to
seventeen questions.

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
- **rulings 1 to 6**: the earlier decisions settled for this amendment;
- **settled rulings**: the answers agreed by SUBC, Thalamus and broca to the
  seventeen questions below, including core's additions to the provider
  codes.

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

### Preamble (status of the rulings)

- **Old:** Q22 was left open after Q1 to Q21 were settled.
- **New:** every numbered question is settled, including Q22.
- **Input:** settled ruling 1 chooses the unknown-run answer and leaves the
  session-level status form optional.
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
  groups must not ride inside either. The group names are agreed, and no
  `session_warm` group is defined until a second consumer exists (settled
  rulings 14 and 17).
- **Checks:** core (uses them), Thalamus (declares or omits them).

### §4.3 Head metadata

- **Old:** `session.head.last_run_state` is pinned; pages may add runner
  members such as broca's `lineage_state`.
- **New:** makes the boundary explicit: `last_run_state` is the pinned run
  summary, while a page's `lineage_state` is not pinned.
- **Input:** settled ruling 9; §4.2 stays unchanged (settled ruling 8).
- **Checks:** core, Thalamus.

### §6 `run_ops`: run errors

- **Old:** one clause: `provider_code` carries the provider's code; for a
  compaction `REFUSE` it is the role `code`, and the answer's finer
  `provider_code` maps to `provider_detail_code`.
- **New:** a typed error with required `class`, `message` and
  `provider_code`, and optional `status`, `retry_after_secs`, `resets_at_ms`,
  `account_ref`, `provider`, `reason` and `provider_detail_code`. An absent
  optional member means not measured, never zero. `provider_code` is the
  code callers branch on. For a compaction `REFUSE` it is a `RefuseCode`
  value (the list is pointed at,
  not repeated) or an unknown code as received, and retry follows from it
  as the provider contract fixes it. The finer code is
  `provider_detail_code`, diagnostics only, never branched on. The text says
  in so many words that the provider wire's `provider_code` maps to the
  runner's `provider_detail_code`, so the two same-named members are not
  read as one field. A run error without a finer code omits
  `provider_detail_code`; `compaction_unavailable` never carries one.
- **Input:** ruling 1 and settled ruling 10. Provider contract §11 (the
  mapping, retryability fixed by `RefuseCode`) and its "Differences from `llm-runner/v1`
  wording", third item. Suite: `compaction_unavailable_distinct_from_refuse`
  checks `provider_code`, `provider`, `reason` and `provider_detail_code`
  for Setup and step refusals (`refusal_error` in `src/compaction.rs`), and
  fails a Setup-unavailable run error that carries `provider_detail_code`.
- **Checks:** MC (provider side of the mapping), core (branches on
  `provider_code`).

### §6 `run_ops`: run attribution

- **Old:** each message records which run and episode produced it and
  whether it is that run's final message, as `run: {run_id, episode,
  final}`; what `final` means for a run that ended on a non-assistant
  message, for a run still running, and for a message the runner cannot
  attribute was left implicit.
- **New:** three sentences. `final` is the last message the run produced,
  whatever its role, so an `interrupted` run ending on a tool result marks
  that tool result final. A run whose terminal is not durable has no final
  message. A message the runner cannot attribute omits `run`; the runner
  never invents one, and absence never means "the last run".
- **Input:** a clarification of the existing pinned text, not a change.
  broca's gap "no per-message run attribution despite `run_ops`" below
  will close after merge when broca implements §6 as written: its WAL
  records every episode boundary, so a reprojection can attribute every message,
  including archived sessions. broca will use the episode's `RunStarted`
  WAL sequence as the opaque `episode`.
- **Checks:** core and Thalamus (they read pages), MC (it reads transcripts
  through the runner route).

### §6.1 `run_status`: `run.status` (new)

- **Old:** none. `run.status` appeared only in §9.1 as a runner-specific op.
- **New:** request `{run_id?}`, strict; `{}` is an optional session-level
  form within the group. Answer `{state, reason?, error?,
  undelivered_steers?}`, lenient, with runner members beside it. `state`
  uses `run::states`, decoded open; an unknown run answers
  `{state: "unknown"}`, never an error. `reason` is an open string with
  known pause reasons `auth_required` and `restart`. `error` follows the §6
  run-error rules.
  `undelivered_steers` on an ended run is present whenever delivery could
  be determined, `[]` when checked and clean, and absent only when
  delivery could not be determined; on a run that has not ended it is
  absent and means nothing. The list covers this run only; a later
  delivery shows in the send's `delivered` receipt.
- **Input:** ruling 4 (the group) and ruling 5 (`undelivered_steers`).
  broca's served shape: annotations rows `run.status`,
  `run.status.undelivered_steers` ("contract is wrong: define terminal
  run.status undelivered_steers, including omission and episode scope"),
  `run.status.error`, `pause_reason`. Current broca emits `[]` when its
  terminal WAL shows no undelivered steers
  (`crates/broca-core/src/replay.rs:1932-1935,1977-1980`), so that old gap is
  removed. The "this run only" clause follows the annotations' Change item
  "Distinguish
  durable steer admission from model consumption". Core polls `run.status`
  (diff §1). Settled rulings 1, 2 and 13 fix the unknown-run answer,
  optional session-level form, `reason` and the runner-only status members.
- **Checks:** core (reads it), Thalamus (whether it can determine
  delivery).

### §6.2 `run_cancel`: `run.cancel` (new)

- **Old:** none. `run.cancel` appeared only in §9.1 as a runner-specific op.
- **New:** request `{run_id}`; answer a JSON string decoded open, `ack` or
  `not_active`. `ack` accepts the cancellation and does not say which
  terminal state the run reaches; the caller reads that through
  `run.result` or `run.status`. A cancelled run ends `cancelled`, never
  `interrupted`. A paused run is cancellable with `ack`; the op is
  owner-only under the session's frozen scope. An expired session answers
  `expired`.
- **Input:** ruling 4 and settled ruling 11. broca's reply strings:
  annotations row `run.cancel`. The cancel-versus-completion race: annotations, Add item 1 ("cancel-
  versus-completion and retract-versus-start races"); the answer follows
  from the existing one-terminal-per-run rule (§6, `RunState`). The
  `expired` clause moves here from §9.1.
- **Checks:** core.

### §7 `streaming`

- **Old:** event kinds and cursors are pinned, but pause reasons are not
  named.
- **New:** a stream event naming why a run is paused uses `reason`, an open
  string with known values `auth_required` and `restart`, as `run.status`
  does.
- **Input:** settled ruling 2.
- **Checks:** core, Thalamus.

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

### §9 Owner sends, settings and replies

- **Old:** owner sends require `send_id` and `prompt`; `mark` is recorded;
  replies name `active`, `finished` and `pending`. Reuse must name the
  differing field, and a paused run is listed as a send refusal.
- **New:** `send_id` and `prompt` stay required of owner sends; a runner may
  admit unkeyed non-owner sends without retry idempotency. `mark` is
  dropped. The steer-settings rule stands: a joining steer whose settings
  differ is refused `invalid_params` naming the first differing field.
  `paused` is named in `send::states`, decoded open, and its `pause_reason`
  stays. A send behind a paused run may be queued with `pending` or refused
  `run_paused`; `send_id_reuse` names `detail.field` when identifiable.
- **Input:** settled rulings 3 to 7 and 16. broca now refuses differing
  joining-steer settings in stable field order
  (`crates/broca-module/src/actor.rs:586-619,2908-2915`;
  `crates/broca-module-serve/src/serve.rs:2695-2698`). §14's crash-idempotency
  guarantee is scoped to keyed sends for the permitted unkeyed form.
- **Checks:** core, Thalamus.

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
  `send_id_reuse` as before. The op is owner-only under the session's
  frozen scope. An expired session answers `expired`.
- **Input:** ruling 4 and settled ruling 11. broca's reply strings:
  annotations row `session.retract`; `send_retracted`: annotations row `send_retracted`
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
- **New:** `unknown_run` remains only for `run.result`; `run.status`
  answers `{state: "unknown"}`. `expired` names the three ops where their
  groups are declared; new row `send_retracted`, not retryable. The
  `run_paused` row permits queueing instead of refusal, `send_id_reuse`
  names the field when identifiable, and `scope_owner_mismatch` includes
  `run.cancel` and `session.retract`. A clause says `send_retracted` is not yet
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
  retryability to that contract. The table also pins `context_overflow`,
  `output_ceiling`, `response_deadline`, `account_limit_exhausted`,
  `billing_exhausted`, `account_changed`, `cyber_policy`, `bio_policy`,
  `misalignment_policy_violation`, `content_filter`,
  `access_terminated_error` and the read-only legacy alias
  `insufficient_quota`. The content-policy codes are provider policy
  refusals, `content_filter` is the normalized safety finish, genuine
  access termination makes the seat unable to run, and consumers must
  interpret recorded `insufficient_quota` as `billing_exhausted` while
  runners stop emitting it. broca's Kimi usage-limit normalization and
  billing-code emission from 0.3.193 on are recorded; plan-fetch refusals
  stay outside the run-error list.
- **Input:** rulings 1 and 2, and settled ruling 10 with core's additions.
  The commons master contract used for this amendment already lacked the
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

### Q22 ruling

- **Old:** unknown-run and session-level status forms were left open.
- **New:** an unknown `run_id` answers `{state: "unknown"}`, not an error;
  request `{}` is an optional session-level form within `run_status`.
- **Input:** settled ruling 1, agreed by SUBC, Thalamus and broca.
- **Checks:** core, Thalamus.

### Gaps in broca today

- **Old:** sixteen gaps describing an older broca: no `role.describe`, no
  `session.baseline`, no `session.head`, no `run.result`, count-only pages,
  build-plan slice references.
- **New:** five remaining gaps checked at broca 0.3.194 (`549f678`): the
  undeclared `run_status`, `run_cancel` and `retract` groups; no per-message
  run attribution; no `session_capabilities` in the baseline; legacy
  lineages without `lineage_id` until a write; and owner sends whose wire
  shape does not require both `send_id` and `prompt`. Every remaining item
  has current file:line citations in the contract. Per-message run
  attribution remains open for broca to implement after merge.
- **Input:** current broca source: role declarations and baseline
  (`crates/broca-wire/src/role.rs:192-282`), read messages and legacy IDs
  (`crates/broca-wire/src/lib.rs:662-717`), owner-send shape
  (`crates/broca-wire/src/lib.rs:126-140`;
  `crates/broca-module-serve/src/wire.rs:112-154`), and terminal steer
  delivery (`crates/broca-core/src/replay.rs:1932-1935,1977-1980`). Empty
  `undelivered_steers` is now present when known, frozen settings inherit,
  and differing joining-steer settings are refused. The rulings remove the
  old unknown-run, `mark`, pause-queueing, `send_id_reuse` and paused-reply
  differences. Retention, hooks, compaction, the approval gate,
  `billing_exhausted` and `output_ceiling` are live, not missing features;
  an optional group a broca build does not declare is not a gap.
- **Checks:** core (it reads broca), MC and Thalamus for awareness only.

### Unchanged on purpose

- **§4.2 `lineage_id` on every page (settled ruling 8).** Kept unchanged;
  legacy lineages remain a broca gap until they have a write.
- **Rulings 12 and 15 below.** The explanation of guaranteed steer
  delivery into the next episode and the pinned
  `plan.compaction_item.provider` decision are kept as written.
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

## Rulings

SUBC, Thalamus and broca have agreed each answer. The contract records these
rulings in the sections they govern.

1. **Q22, `run.status` unknown run and session-level mode.** An unknown run
   answers `{state: "unknown"}`, not an error. The session-level form,
   request `{}`, is optional within `run_status`.
2. **Pause reasons and `pause_reason`.** The role names `reason` on
   `run.status` and on the stream, an open string with known values
   `auth_required` and `restart`. The send reply's `pause_reason` stays.
3. **`paused` as a `session.send` state.** Name `paused` in `send::states`,
   decoded open.
4. **`run_paused` versus queueing behind a pause.** A runner may queue a
   send behind a paused run, answering `pending`, or refuse it with
   `run_paused`; both conform.
5. **Steer inheritance: settled in §9.** The steer-settings rule stands:
   a joining steer whose settings differ is refused `invalid_params`,
   naming the first differing field. broca now meets the rule
   (`crates/broca-module/src/actor.rs:586-619,2908-2915`;
   `crates/broca-module-serve/src/serve.rs:2695-2698`).
6. **`send_id` required and `prompt` required.** Both stay required of
   owner sends. A runner may admit unkeyed non-owner sends, without retry
   idempotency.
7. **`mark`.** Drop `mark` from the role.
8. **`lineage_id` on legacy lineages.** Keep §4.2 unchanged: every page of
   a written session carries `lineage_id`.
9. **`lineage_state` on a page.** It is a runner member, not pinned;
   `session.head.last_run_state` is the pinned surface.
10. **Provider codes and the run error's schema.** Alongside the pinned
    compaction codes, pin `context_overflow`, `output_ceiling`,
    `response_deadline`, `account_limit_exhausted`, `billing_exhausted`,
    `account_changed`, `cyber_policy`, `bio_policy`,
    `misalignment_policy_violation`, `content_filter` (the normalized safety
    finish), `access_terminated_error` (the seat cannot run) and
    `insufficient_quota` (a legacy alias consumers MUST treat as
    `billing_exhausted` and runners MUST stop emitting); the content-policy
    codes are provider policy refusals, broca maps a Kimi usage-limit
    message under `access_terminated_error` to `account_limit_exhausted`
    instead, and broca emits `billing_exhausted` from 0.3.193 on.
    The run error requires `class`, `message` and `provider_code`, with
    optional `status`, `retry_after_secs`, `resets_at_ms`, `account_ref`,
    `provider`, `reason` and `provider_detail_code`; an absent member means
    not measured, never zero, and plan-fetch refusals such as
    `plan_param_refused` stay send refusals, outside the run-error list.
11. **`run.cancel` and `session.retract` details.** A paused run is
    cancellable with `ack`. Both ops are owner-only, like `session.send`.
12. **`undelivered_steers` on a `guaranteed` runner.** Answered by broca, no
    contract change. A steer a run never rendered is not dropped: broca
    appends it to the conversation when the run ends and carries it into the
    next episode's prefix, so the session's next model step includes it and
    its receipt becomes `turn` naming that run (broca `replay.rs`
    RunFinished and RunStarted handling, `run.rs` lineage prefix,
    `actor.rs` receipt search). `undelivered_steers` means "not yet seen by
    a model step of this run", which lets the owner prompt a turn sooner.
    §2's "may be absent until the steer is rendered" already covers the
    window, so `guaranteed` stands.
13. **Other `run.status` members.** `indeterminate_tool_calls`,
    `final_step_finish_reason`, `usage` and `retries_used` stay runner
    members, unpinned.
14. **`session.warm`.** No `session_warm` group until a second consumer
    exists.
15. **`plan.compaction_item.provider`: promoted to pinned in §10.1.** MC's
    review: the field gates `compaction.ready` (§11.1), so it can't stay
    provisional; Prefrontal's fetch plans already carry it and ck-mc passes
    all 27 vectors that use it.
16. **`send_id_reuse` `detail.field`.** Name the differing field in
    `detail.field` when the runner can identify it.
17. **Group names.** Keep `run_status`, `run_cancel` and `retract`.

## Crate and suite follow-ups

Not contract text; listed so the merge carries them.

- `cortexkit-role-llm-runner`: add op constants for `run.status`,
  `run.cancel` and `session.retract`, their groups `run_status`,
  `run_cancel` and `retract` in `capabilities::GROUPS`, request and answer
  types for them, the typed
  run error with required `class`, `message` and `provider_code` and the
  optional members in §6, the newly pinned provider codes in §12.2,
  `paused` in `send::states` and `pause_reason` in `SendReply`; remove
  `mark` from `SendRequest`, add
  `send_retracted` in `errors::CODES`, and change the doc comment on
  `errors::provider_codes::COMPACTION_UNAVAILABLE` to Setup-only. The
  `run-result.json` case named "compaction call unavailable with no answer"
  should be renamed to say Setup.
- `cortexkit-role-llm-runner-conformance`: implement
  `run_attribution_per_message` (today in the README's "not yet
  implemented" list) in the same change as this amendment. It checks the
  three §6 rules: every attributable message carries `run`; `final` sits on
  exactly the last message of each run with a durable terminal and on none
  of a running run's messages; an unattributable message omits `run`
  rather than inventing one. Each rule has one deliberately broken fake
  that must fail the case by name, so a runner pins against a case that
  can fail.
- `cortexkit-role-llm-runner-conformance`: the narrowing
  "retention_run_status_expired is inapplicable if run.status is not
  advertised; run.status remains runner-specific" should drop "remains
  runner-specific"; the planned case
  `run_status_undelivered_steers_present_when_clean` does not exist yet.
- `cortexkit-role-compaction-provider/CONTRACT.md`: its "Differences from
  `llm-runner/v1` wording" section's second and third items are resolved by
  this amendment once it merges.
