//! The suite's cases and what a run reports.

use cortexkit_role_harness::KillReport;
use cortexkit_role_llm_runner::points;

use crate::subject::Capability;

/// One conformance case: its stable name, what it requires, and what it
/// checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaseSpec {
    pub name: &'static str,
    /// Every one of these must be declared.
    pub requires: &'static [Capability],
    /// At least one of these must be declared, when not empty.
    pub requires_any: &'static [Capability],
    /// At least one of these must be left undeclared, when not empty. A
    /// subject that declares all of them cannot be asked the question, so
    /// the case is [`CaseOutcome::Inapplicable`].
    pub requires_undeclared_any: &'static [Capability],
    pub checks: &'static str,
}

use Capability::{
    DispatchAttribution, HoldToolCalls, Interrupt, KillAt, Queue, RunOps, Steer, Streaming,
    TranscriptReads,
};

const fn case(
    name: &'static str,
    requires: &'static [Capability],
    checks: &'static str,
) -> CaseSpec {
    CaseSpec {
        name,
        requires,
        requires_any: &[],
        requires_undeclared_any: &[],
        checks,
    }
}

const READS: &[Capability] = &[TranscriptReads, Queue];
const RUNS: &[Capability] = &[RunOps, Queue];
const DISPATCH: &[Capability] = &[DispatchAttribution, Queue, TranscriptReads];
const SENDS: &[Capability] = &[Queue, TranscriptReads];

/// Every case, in the order the runner runs them.
pub const CASES: &[CaseSpec] = &[
    case(
        "role_describe_shape",
        &[],
        "role.describe passes check_describe, lists the llm-runner/v1 major with a known stability, and names its session-capability source as admission or baseline",
    ),
    case(
        "role_describe_cacheable",
        &[],
        "role.describe answers identically twice on one route and on the routes of two different sessions",
    ),
    case(
        "role_describe_groups_complete",
        &[],
        "every group role.describe declares lists all its ops, and the groups it declares are exactly the groups the subject declares, so no group is claimed on the wire while its cases are skipped",
    ),
    case(
        "extra_op_still_admitted",
        &[],
        "the live role.describe answer, with an extra op, an extra major, an unknown capability and an unknown member added, is still admitted by check_describe with the same groups",
    ),
    case(
        "describe_states_max_bytes",
        &[TranscriptReads],
        "role.describe states max_bytes with a non-zero default no larger than its maximum",
    ),
    case(
        "baseline_owner_only",
        &[Queue],
        "after the owner's first send, session.baseline answers the owner ready with a baseline and refuses another caller scope_owner_mismatch",
    ),
    case(
        "baseline_matches_admission_reply",
        &[Queue],
        "on a runner whose session capabilities come from admission, the first send's reply carries a baseline equal to what session.baseline answers",
    ),
    case(
        "tail_read_is_newest_page",
        READS,
        "on a session longer than one page, a read with no cursor answers the newest messages, ending at session.head's last_ordinal, the same messages a range read of them answers",
    ),
    case(
        "range_read_stops_by_count",
        READS,
        "a range read with a limit answers that many messages and a next_from_ordinal from which the following message is read",
    ),
    case(
        "range_read_stops_by_bytes",
        READS,
        "a range read with a one-byte cap and a limit above the transcript stops after one message, untruncated, with next_from_ordinal set",
    ),
    case(
        "oversize_message_alone_on_its_page",
        READS,
        "a message larger than the read's byte cap comes alone on its page, whole, with next_from_ordinal set because more follow",
    ),
    case(
        "after_mid_reads_strictly_after",
        READS,
        "a read after a message's mid in the session's lineage answers the messages that follow it, starting with the next one",
    ),
    case(
        "after_mid_unknown_mid_refused",
        READS,
        "a read after a mid the lineage does not have is refused unknown_mid, never answered from the tail",
    ),
    case(
        "read_other_lineage_refused",
        READS,
        "a tail, range or after read naming another lineage is refused lineage_changed",
    ),
    case(
        "lineage_id_on_every_page",
        READS,
        "every page of a written session, in every read mode, carries the session's lineage_id, the one session.head reports",
    ),
    case(
        "unknown_read_field_refused",
        &[TranscriptReads],
        "session.read and session.head refuse an unknown request field as invalid_params naming it",
    ),
    case(
        "never_written_session_empty_page",
        &[TranscriptReads],
        "a read of a session never written answers an empty page with no lineage_id and no next_from_ordinal",
    ),
    case(
        "never_written_session_empty_head",
        &[TranscriptReads],
        "session.head of a session never written answers with every member absent",
    ),
    case(
        "head_has_no_bodies",
        READS,
        "session.head of a written session decodes as consistent head metadata matching the transcript and carries no message body",
    ),
    case(
        "run_result_completed_final_text",
        RUNS,
        "a completed run answers run.result completed with the final assistant message's text, not an earlier turn's",
    ),
    case(
        "run_result_text_parts_joined_unchanged",
        RUNS,
        "a final message's text parts, a JSON value split across them with reasoning between, are joined with nothing inserted and the reasoning left out",
    ),
    case(
        "run_result_completed_no_text_is_empty",
        RUNS,
        "a completed run whose final message has only reasoning answers final_message with text \"\", never without final_message",
    ),
    case(
        "run_result_interrupted_not_cancelled",
        &[RunOps, Queue, TranscriptReads, KillAt(points::DISPATCH_INTENT)],
        "a run cut by a kill at DispatchIntent answers run.result interrupted, never cancelled",
    ),
    case(
        "dispatched_to_per_call",
        DISPATCH,
        "every scripted tool call has one tool_calls entry naming the scripted tool provider as dispatched_to, with a well-formed call_key, not indeterminate once the run ended",
    ),
    case(
        "indeterminate_until_closed",
        &[DispatchAttribution, Queue, TranscriptReads, HoldToolCalls],
        "a call the tool provider holds is read as indeterminate with its call_key and dispatched_to, and is no longer indeterminate once its result is in",
    ),
    case(
        "sibling_calls_distinct_call_keys",
        DISPATCH,
        "two tool calls from one assistant step get distinct call_keys",
    ),
    case(
        "recurring_model_id_distinct_call_keys",
        DISPATCH,
        "one model tool_call_id used in two steps gets a distinct call_key each time",
    ),
    case(
        "subscribe_from_head_no_gap_no_duplicate",
        &[Streaming, Queue, TranscriptReads],
        "a subscription from a page's head answers exactly the durable events a replay from the start holds after that page's snapshot, every one with a cursor",
    ),
    case(
        "send_id_retry_same_answer",
        &[],
        "on a runner that declares queue or steer, and without reading the transcript or waiting: a send retried twice at once with the same send_id and payload names the run_id and submission_id the send was answered with, and its delivered receipt moves only forward (absent or pending to step, turn or unknown, which are final, ref included); the send's state may still move between the retries, so they are not compared whole",
    ),
    case(
        "send_id_retry_settled_same_answer",
        &[RunOps],
        "on a runner that declares queue or steer, and without reading the transcript: once the send's run has ended, seen through run.result, a send retried twice with the same send_id and payload gets the same answer twice apart from delivered, names the run_id and submission_id the send was answered with, and its delivered receipt moves only forward; inapplicable when the send's reply names no run_id",
    ),
    case(
        "send_id_retry_written_once",
        &[Queue],
        "on a runner that declares transcript_reads, a send retried twice with the same send_id and payload is answered with the run it started, starts no other run, and its prompt is written once",
    ),
    case(
        "send_id_reuse_refused_naming_field",
        &[],
        "on a runner that declares queue or steer, and without reading the transcript: a send_id reused with another prompt is refused send_id_reuse naming prompt",
    ),
    case(
        "send_id_reuse_writes_nothing",
        &[Queue],
        "on a runner that declares transcript_reads, a send_id reused with another prompt writes nothing, however it is answered",
    ),
    CaseSpec {
        name: "delivery_change_refused",
        requires: &[],
        requires_any: &[Steer, Interrupt],
        requires_undeclared_any: &[],
        checks: "on a runner that declares queue or steer, and without reading the transcript: a send_id reused with another declared delivery mode is refused send_id_reuse naming delivery",
    },
    CaseSpec {
        name: "delivery_change_writes_nothing",
        requires: &[Queue],
        requires_any: &[Steer, Interrupt],
        requires_undeclared_any: &[],
        checks: "on a runner that declares transcript_reads, a send_id reused with another declared delivery mode writes nothing, however it is answered",
    },
    case(
        "unknown_delivery_refused",
        SENDS,
        "a send with a delivery mode the role does not define is refused invalid_params naming delivery, and nothing is written",
    ),
    CaseSpec {
        name: "undeclared_delivery_refused",
        requires: SENDS,
        requires_any: &[],
        requires_undeclared_any: &[Steer, Interrupt],
        checks: "a send with a delivery mode the runner does not declare is refused delivery_unsupported naming the mode, and nothing is written",
    },
    case(
        "guaranteed_steer_never_pending_or_unknown",
        SENDS,
        "on a guaranteed runner with hold_tool_calls, a steer and its same-send_id re-send are answered while the held turn is still active, naming that run; any first receipt and the required re-send receipt are step or turn and name the same delivery",
    ),
    case(
        "resend_steer_delivered_stable",
        SENDS,
        "a steer re-sent twice with the same send_id carries a delivered receipt that moves only forward: absent or pending to step, turn or unknown, which are final, ref included",
    ),
    case(
        "crash_at_Admitted",
        &[Queue, TranscriptReads, KillAt(points::ADMITTED)],
        "killed at Admitted and restarted: another session's messages keep their ordinals, mids and bodies; the retried first send is answered and its prompt written once; the run is not cancelled",
    ),
    case(
        "crash_at_SendRecorded",
        &[Queue, TranscriptReads, KillAt(points::SEND_RECORDED)],
        "killed at SendRecorded and restarted: the retried send is answered with the existing state, nothing is written twice, and the run is not cancelled",
    ),
    case(
        "crash_at_StepRecorded",
        &[Queue, TranscriptReads, DispatchAttribution, KillAt(points::STEP_RECORDED)],
        "killed at StepRecorded and restarted: observations show either one dispatch and a continued run with a result before the later model turn, or zero dispatches and an interrupted run followed by an owner follow-up with a result before its assistant turn; the call is not indeterminate and the step is not generated again",
    ),
    case(
        "crash_at_DispatchIntent",
        &[
            Queue,
            TranscriptReads,
            DispatchAttribution,
            KillAt(points::DISPATCH_INTENT),
        ],
        "killed at DispatchIntent and restarted: the call is never re-dispatched, is closed outcome_unknown and no longer indeterminate, the run ends interrupted without another model call, and call_keys read before the kill are unchanged",
    ),
    case(
        "crash_at_ToolResultRecorded",
        &[Queue, TranscriptReads, KillAt(points::TOOL_RESULT_RECORDED)],
        "killed at ToolResultRecorded and restarted: the tool is not called again and its result is written once",
    ),
    case(
        "crash_at_Terminal",
        &[Queue, TranscriptReads, KillAt(points::TERMINAL)],
        "killed at Terminal and restarted: the run keeps its one terminal state, completed, and nothing is written twice",
    ),
];

/// Where this suite skips or narrows a check because the role leaves the
/// rule open, or because nothing on the wire can observe it. Printed with
/// every report. Section numbers refer to `CONTRACT.md` in
/// `cortexkit-role-llm-runner`.
pub const NARROWINGS: &[&str] = &[
    "the fetch plan's shape is not pinned (CONTRACT §10), so the subject builds each session's first-send fields and the suite never inspects a plan or a baseline's items",
    "the suite writes sessions with session.send (delivery absent, which means queue), so every case that needs a written session requires queue; a runner that admits sessions declares it (§9)",
    "the suite waits for a run to end through session.head's last_run_state, or run.result where only run_ops is required, so cases that wait require transcript_reads",
    "a read's byte measure is not pinned, so byte-cap cases use a cap below any message (one byte) or a message far above the cap, never a cap between message sizes",
    "message bodies are in the runner's own schema (§4.2), so the suite finds a scripted prompt, text part or tool result by a unique marker string inside a message body, and counts messages holding it",
    "where a tool result's reason sits is the runner's schema (§12.3), so crash_at_DispatchIntent only checks that some message holds the string outcome_unknown",
    "subscribe_from_head_no_gap_no_duplicate compares a replay from the head with a replay from start on an idle session; the live handoff race is not exercised",
    "the crash guarantee that a read naming the old lineage after a lineage change is refused (§14, 2) is not checked: nothing in this subset changes a lineage",
    "the crash guarantee of replay, not re-invocation (§14, 5) is checked only for the model and the tool (a durable step is not generated again, a recorded result is not re-invoked); hooks and compaction are not in this subset",
    "the crash guarantee that each resume writes one informational record (§14, 8) is not checked: where that record sits is the runner's schema",
    "a call's indeterminate window between a restart and its outcome_unknown close is not observable reliably, so crash_at_DispatchIntent checks the state after the close",
    "a run cut at DispatchIntent is expected to end interrupted, as the role's points list states for that point; StepRecorded accepts either one dispatch with continuation or zero dispatches with interrupted state; ToolResultRecorded checks only that the run has one terminal state that is not cancelled",
    "the subject interface does not expose model requests, so StepRecorded checks transcript ordering on resume or after an owner follow-up to a sealed run: one result precedes the later assistant turn; result bodies are joined by the model call id or attributed call key, without inspecting content, since the runner's message schema is not pinned; it cannot inspect the history actually sent to the model",
    "the crash guarantee that messages read before a kill read the same after it (§14, 1) is checked on a second session written before the kill, because the cut session has nothing readable before its trigger",
    "run_result_interrupted_not_cancelled reads the run cut in the DispatchIntent crash scenario, because a run cuts each point only once",
    "extra_op_still_admitted reads its case as the consumer's lenient decoding (§2) applied to the live answer",
    "role_describe_groups_complete also fails a runner whose role.describe declares a group the subject does not, because the suite would skip that group's cases while the wire claims it",
    "baseline_matches_admission_reply is inapplicable to a runner whose session capabilities come from session.baseline, which has no admission reply",
    "undeclared_delivery_refused is inapplicable to a runner that declares every delivery mode",
    "the suite fails a run with no kill at all, because §14 fails every run in which no kill ended a real process",
    "each send_id case is split in two: send_id_retry_same_answer, send_id_retry_settled_same_answer, send_id_reuse_refused_naming_field and delivery_change_refused check only the reply, never read the transcript, and run on a runner that declares queue or steer (their first send is steer when queue is not declared); send_id_retry_written_once, send_id_reuse_writes_nothing and delivery_change_writes_nothing check what was written and are inapplicable without transcript_reads",
    "send_id_retry_same_answer never waits, so it does not compare its two retries whole: before the send's run ends its state may move on between them; send_id_retry_settled_same_answer makes that comparison after seeing the run end through run.result, so it requires run_ops (skipped without it) and is inapplicable when the send's reply names no run_id",
    "a delivered receipt may also move from absent to pending, because on a confirm runner an absent receipt already means pending (§9)",
];

/// How one case ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseOutcome {
    Passed,
    Failed {
        reason: String,
    },
    /// Not run: the subject does not declare these capabilities.
    Skipped {
        missing: Vec<Capability>,
    },
    /// Not run: the question cannot be asked of this runner at all (for
    /// example, an undeclared delivery mode of a runner that declares them
    /// all). Never a pass, and not a missing capability either.
    Inapplicable {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseReport {
    pub case: &'static str,
    pub requires: Vec<Capability>,
    pub outcome: CaseOutcome,
}

/// The run's overall result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuiteVerdict {
    /// Every case ran (or was inapplicable) and passed, and at least one
    /// kill ended a real process.
    Passed,
    /// Nothing failed, but the subject does not declare every capability,
    /// so the cases requiring `skipped` were not run. The runner conforms
    /// for the capabilities in `declared`, and for no others. Never a plain
    /// pass.
    ConformingForDeclaredCapabilities {
        declared: Vec<Capability>,
        skipped: Vec<Capability>,
    },
    Failed {
        reasons: Vec<String>,
    },
}

/// Everything a run found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuiteReport {
    pub cases: Vec<CaseReport>,
    /// Every kill the run made, with the mechanism the harness reported.
    pub kills: Vec<KillReport>,
    pub verdict: SuiteVerdict,
}

impl SuiteReport {
    pub fn outcome(&self, case: &str) -> Option<&CaseOutcome> {
        self.cases
            .iter()
            .find(|report| report.case == case)
            .map(|report| &report.outcome)
    }

    /// A human-readable summary, one line per case, then the kills, the
    /// narrowings and the verdict, for a CI log.
    pub fn render(&self) -> String {
        let names = |capabilities: &[Capability]| -> String {
            capabilities
                .iter()
                .map(|c| c.name())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut out = String::new();
        for case in &self.cases {
            let line = match &case.outcome {
                CaseOutcome::Passed => format!("PASS {}", case.case),
                CaseOutcome::Failed { reason } => format!("FAIL {}: {reason}", case.case),
                CaseOutcome::Skipped { missing } => format!(
                    "SKIP {} (requires {}; missing {})",
                    case.case,
                    names(&case.requires),
                    names(missing)
                ),
                CaseOutcome::Inapplicable { reason } => {
                    format!("N/A  {}: {reason}", case.case)
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
        for kill in &self.kills {
            out.push_str(&format!("KILL at {} by {}\n", kill.point, kill.mechanism));
        }
        for narrowing in NARROWINGS {
            out.push_str(&format!("NARROWED {narrowing}\n"));
        }
        let verdict = match &self.verdict {
            SuiteVerdict::Passed => "passed".to_owned(),
            SuiteVerdict::ConformingForDeclaredCapabilities { declared, skipped } => format!(
                "conforming for declared capabilities: {} (not run: {})",
                names(declared),
                names(skipped)
            ),
            SuiteVerdict::Failed { reasons } => format!("failed: {}", reasons.join("; ")),
        };
        out.push_str(&format!("VERDICT {verdict}\n"));
        out
    }
}
