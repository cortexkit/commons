//! Error codes a runner answers with, carried as the `code` of an `ERROR`
//! frame's body (`{code, message, detail?}`), and the reasons a runner
//! writes into a tool result or a run's terminal.
//!
//! Codes are open strings on the wire: a consumer that meets a code it does
//! not know treats it as a terminal refusal of that one request. Only the
//! codes [`is_retryable`] names are retried with the same request; every
//! other refusal is an answer, and retrying it unchanged meets the same
//! refusal.
//!
//! This crate carries no error-body type of its own. The helpers here take
//! the body's `code` and `detail` as found in whatever frame type the
//! caller's transport decodes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A request field is malformed, unknown, or combined with a field it
/// excludes. `detail.field` names it.
pub const INVALID_PARAMS: &str = "invalid_params";

/// A session input-token cap stopped a tool call before dispatch. Not a
/// retryable request refusal; the run ends in the state of the same name.
pub const BUDGET_EXHAUSTED: &str = "budget_exhausted";

/// A read's `lineage_id` is not the session's current lineage. The caller
/// re-reads from the tail; it never retries the same cursor.
pub const LINEAGE_CHANGED: &str = "lineage_changed";

/// A read's `after_mid` names no message in the lineage the read named.
pub const UNKNOWN_MID: &str = "unknown_mid";

/// The complete encoded `session.read` reply exceeds the transport frame
/// limit. Terminal: retrying with a smaller `max_bytes` cannot shrink a
/// single message or atomic model-view group. `detail` is
/// [`ReplyTooLargeDetail`].
pub const REPLY_TOO_LARGE: &str = "reply_too_large";

/// Safe identity and size details for a `reply_too_large` refusal. The
/// subject names a message or a model-view group without carrying its content.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReplyTooLargeDetail {
    /// Length of the complete encoded reply frame, including its header.
    pub bytes: u64,
    /// Maximum encoded frame length accepted by the transport.
    pub limit: u64,
    pub subject: ReplyTooLargeSubject,
}

impl ReplyTooLargeDetail {
    pub fn new(bytes: u64, limit: u64, subject: ReplyTooLargeSubject) -> Self {
        Self {
            bytes,
            limit,
            subject,
        }
    }
}

/// The indivisible item that made a `session.read` reply too large.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReplyTooLargeSubject {
    /// A raw transcript message, identified by its session and ordinal.
    Message { session: String, ordinal: u64 },
    /// An atomic model-view group, identified by the existing source of its
    /// anchored entry.
    ModelGroup { source: crate::read::EntrySource },
    /// A `run.result` or `run.status` answer for this run, made too large by
    /// its structured-output `value`, which is never cut short.
    Run { run_id: String },
    /// One entry of a run's `run.tool_calls` list, identified by its run and
    /// the model's id for the call (with its `call_key` when it has one,
    /// since a model id need not be unique), made too large by its `args`,
    /// which are never cut short.
    ToolCall {
        run_id: String,
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_key: Option<String>,
    },
}

/// `run.result` or `run.tool_calls` names a run the session does not have.
pub const UNKNOWN_RUN: &str = "unknown_run";

/// A `session.resolve` names a well-formed tag the session never assigned.
/// Terminal: tags are never reused, so the same request meets the same
/// refusal. A malformed tag is `invalid_params` naming `tag` instead.
pub const UNKNOWN_TAG: &str = "unknown_tag";

/// The session's context is at or over its ceiling, so the runner will not
/// send the request. It refuses a `session.send` that would start or join a
/// run, writing nothing; a run already in progress that reaches the ceiling
/// at a step boundary ends in the run state of the same name instead
/// (`crate::run::states::CONTEXT_CEILING`). `detail` is
/// [`ContextCeilingDetail`].
///
/// It is not a model or provider failure: a caller that routes between
/// models must not charge backoff to the model or mark it unhealthy for it.
/// A provider's own overflow refusal is different: that run ends `error`
/// with the provider code `context_overflow`.
pub const CONTEXT_CEILING: &str = "context_ceiling";

/// Why the runner stopped at the ceiling: the `cause` of a
/// [`ContextCeilingDetail`]. Decoded as an open string; a consumer treats a
/// cause it does not know as a ceiling stop all the same.
pub mod context_ceiling_causes {
    /// The runner waited the compaction provider's bound at the ceiling and
    /// no view came.
    pub const VIEW_TIMEOUT: &str = "view_timeout";
    /// The session has no compaction provider (none at setup), or its
    /// provider is unreachable at the ceiling.
    pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";
    /// The request cannot fit the window even with the current view applied.
    pub const OVER_WINDOW: &str = "over_window";
    /// Every cause this role documents.
    pub const ALL: &[&str] = &[VIEW_TIMEOUT, PROVIDER_UNAVAILABLE, OVER_WINDOW];
}

/// The detail of a `context_ceiling` refusal, and of a run that ended in
/// the `context_ceiling` state: sizes in tokens, and why the runner stopped.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ContextCeilingDetail {
    /// The model's context window, in tokens.
    pub window: u64,
    /// The runner's estimate of the request's size, in tokens, when it
    /// stopped.
    pub size: u64,
    /// One of [`context_ceiling_causes`]; any other value decodes as itself.
    pub cause: String,
}

impl ContextCeilingDetail {
    pub fn new(window: u64, size: u64, cause: impl Into<String>) -> Self {
        Self {
            window,
            size,
            cause: cause.into(),
        }
    }
}

/// The session's transcript has expired. Terminal, never retried. The
/// detail is [`ExpiredDetail`], not an empty never-written-session reply.
pub const EXPIRED: &str = "expired";

/// Detail of the named `expired` answer to a read of an expired session.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ExpiredDetail {
    /// Wall-clock expiry in milliseconds since the Unix epoch, not deletion time.
    pub expired_at_ms: u64,
}

impl ExpiredDetail {
    pub fn new(expired_at_ms: u64) -> Self {
        Self { expired_at_ms }
    }
}

/// Detail of the `invalid_params` refusal a runner returns when the `retention` field of a
/// `session.send` request is out of range.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RetentionInvalidDetail {
    pub field: String,
    /// Present for an above-maximum request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_seconds: Option<u64>,
}

impl RetentionInvalidDetail {
    pub fn new() -> Self {
        Self {
            field: "retention".into(),
            max_seconds: None,
        }
    }
    pub fn with_max_seconds(mut self, max_seconds: u64) -> Self {
        self.max_seconds = Some(max_seconds);
        self
    }
}

impl Default for RetentionInvalidDetail {
    fn default() -> Self {
        Self::new()
    }
}

/// A send reused a `send_id` with a different payload or delivery mode, or
/// a `session.adopt` reused one with another plan. `detail.field` names the
/// first field that differs, for example `delivery` or `plan`.
pub const SEND_ID_REUSE: &str = "send_id_reuse";

/// A retry of a durably retracted send. Nothing is written and it never starts.
pub const SEND_RETRACTED: &str = "send_retracted";

/// A send whose delivery mode (`queue` when absent) the runner does not
/// declare as a capability group. `detail.delivery` names the mode. Nothing
/// was written, and the send is never delivered in another mode instead.
pub const DELIVERY_UNSUPPORTED: &str = "delivery_unsupported";

/// A send into a session whose last run is paused, other than the paused
/// run's own retry or an interrupt. Nothing was written.
pub const RUN_PAUSED: &str = "run_paused";

/// The caller is not the identity allowed to drive the session: a send, a
/// `session.baseline` request or a `session.adopt`, from anyone but the
/// session's owner.
pub const SCOPE_OWNER_MISMATCH: &str = "scope_owner_mismatch";

/// A scoped send into a session recorded without a scope that cannot adopt
/// the stamped one. `detail` is `no_recorded_principal` or
/// `principal_mismatch {recorded}`.
pub const SCOPE_ADOPTION_REFUSED: &str = "scope_adoption_refused";

/// A send under a scope on a daemon that lacks scopes. Nothing was written.
pub const SCOPE_UNSUPPORTED: &str = "scope_unsupported";

/// Admission could not fetch a required plan item within its deadline.
/// `detail.provider` names it. Nothing was written. Retryable.
pub const FETCH_UNAVAILABLE: &str = "fetch_unavailable";

/// Admission found fetched items that disagree with the plan's composition.
/// `detail.differences` names them. Nothing was written. The starter
/// preflights again and retries once with a new plan, never the same one.
pub const PLAN_STALE: &str = "plan_stale";

/// Admission found tools with the same model-facing name.
/// `detail` is [`ToolNameCollisionDetail`]. Nothing was written.
pub const TOOL_NAME_COLLISION: &str = "tool_name_collision";

/// A later send carried a plan that differs from the session's frozen plan,
/// or a `session.adopt` carried a plan other than the one the runner applied
/// when the host started the session. `detail` is [`PlanDriftDetail`].
/// Nothing was applied. Terminal: never retried with the same request.
pub const PLAN_DRIFT: &str = "plan_drift";

/// The identities of the frozen and sent plans in a `plan_drift` refusal.
/// Identities only, never plan content.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PlanDriftDetail {
    /// Absent when the session's first episode had no plan, or, on
    /// `session.adopt`, when the runner applied no plan at host start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<String>,
    pub sent: String,
}

impl PlanDriftDetail {
    pub fn new(frozen: Option<String>, sent: impl Into<String>) -> Self {
        Self {
            frozen,
            sent: sent.into(),
        }
    }
}

/// The model-facing name and every provider that offered it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolNameCollisionDetail {
    pub name: String,
    pub providers: Vec<String>,
}

/// A steered or queued send whose PreUser hook was unavailable under
/// `on_unavailable: refuse`. Nothing was written.
pub const PRE_USER_UNAVAILABLE: &str = "pre_user_unavailable";

/// The runner could not answer now for a reason that may clear by itself
/// (a provider or store briefly unavailable). Retryable.
pub const TRANSIENT: &str = "transient";

/// The scope's owner has not yet synced the scope after a daemon restart.
/// Retryable, with backoff, within the caller's hold bound.
pub const SCOPE_NOT_SYNCED: &str = "scope_not_synced";

/// A `compaction.ready` from a caller other than the session's frozen
/// compaction provider: the route's caller stamp is not the provider of the
/// plan's `compaction_item`, or the session has none. The session is not
/// woken.
pub const NOT_SESSION_COMPACTION_PROVIDER: &str = "not_session_compaction_provider";

/// A `session.refresh` or `session.refresh_policy` names a policy whose rung
/// the session does not support for that surface. Refused when the policy is
/// set; nothing was written.
pub const RUNG_UNSUPPORTED: &str = "rung_unsupported";

/// A `session.refresh_policy` names a generation that is no longer the
/// pending one, because the runner has since accepted a later refresh.
/// Nothing was written.
pub const PENDING_SUPERSEDED: &str = "pending_superseded";

/// A `session.refresh_policy` names a generation a prefix rebuild or an
/// append has already applied, so there is no pending change left for the
/// policy to govern. Nothing was written.
pub const ALREADY_APPLIED: &str = "already_applied";

/// The refusals of the `session_change` group's ops.
pub const SESSION_CHANGE_CODES: &[&str] = &[RUNG_UNSUPPORTED, PENDING_SUPERSEDED, ALREADY_APPLIED];

/// Every refusal code this role defines.
pub const CODES: &[&str] = &[
    INVALID_PARAMS,
    LINEAGE_CHANGED,
    UNKNOWN_MID,
    REPLY_TOO_LARGE,
    UNKNOWN_RUN,
    UNKNOWN_TAG,
    CONTEXT_CEILING,
    EXPIRED,
    SEND_ID_REUSE,
    SEND_RETRACTED,
    DELIVERY_UNSUPPORTED,
    RUN_PAUSED,
    SCOPE_OWNER_MISMATCH,
    SCOPE_ADOPTION_REFUSED,
    SCOPE_UNSUPPORTED,
    FETCH_UNAVAILABLE,
    PLAN_STALE,
    TOOL_NAME_COLLISION,
    PLAN_DRIFT,
    PRE_USER_UNAVAILABLE,
    TRANSIENT,
    SCOPE_NOT_SYNCED,
    NOT_SESSION_COMPACTION_PROVIDER,
    RUNG_UNSUPPORTED,
    PENDING_SUPERSEDED,
    ALREADY_APPLIED,
];

/// Whether a runner's refusal is retryable with the same request. Route
/// refusals the daemon itself classifies as retryable on open (module
/// reloading or warming, target unavailable) are the transport's to judge
/// and are not listed here.
pub fn is_retryable(code: &str) -> bool {
    matches!(code, TRANSIENT | FETCH_UNAVAILABLE | SCOPE_NOT_SYNCED)
}

/// The field an `invalid_params` or `send_id_reuse` refusal names, or `None`
/// for any other code or a detail without `field`.
pub fn refused_field<'a>(code: &str, detail: Option<&'a Value>) -> Option<&'a str> {
    if code != INVALID_PARAMS && code != SEND_ID_REUSE {
        return None;
    }
    detail?.get("field")?.as_str()
}

/// Reasons a runner writes on a tool call's error result. Where the reason
/// sits inside a message body is the runner's message schema.
pub mod tool_result_reasons {
    /// The session input-token cap stopped the call before dispatch.
    pub const BUDGET_EXHAUSTED: &str = super::BUDGET_EXHAUSTED;
    /// The call shared a step with a `submit_result` call, which ended the
    /// run before any call of that step was dispatched.
    pub const SKIPPED_AFTER_SUBMIT: &str = "skipped_after_submit";
    /// The session's context reached its ceiling at a step boundary, which
    /// ended the run `context_ceiling` before the call was dispatched.
    pub const CONTEXT_CEILING: &str = super::CONTEXT_CEILING;
    /// The call had a durable dispatch intent and no result when the runner
    /// restarted: it may or may not have run. Never re-dispatched.
    pub const OUTCOME_UNKNOWN: &str = "outcome_unknown";
    /// A PreTool validate hook denied the call.
    pub const PRE_TOOL_DENIED: &str = "pre_tool_denied";
    /// A PreTool hook under `on_unavailable: refuse` was unavailable.
    pub const PRE_TOOL_UNAVAILABLE: &str = "pre_tool_unavailable";
    /// A human declined the call at the approve phase.
    pub const PRE_TOOL_DECLINED: &str = "pre_tool_declined";
    /// The approve-phase question expired under `on_expiry: deny`.
    pub const PRE_TOOL_EXPIRED: &str = "pre_tool_expired";
    /// The call executed, and a PostTool hook under `on_unavailable: refuse`
    /// was unavailable, so its output was withheld.
    pub const POST_TOOL_UNAVAILABLE: &str = "post_tool_unavailable";
    /// The call was never sent: its route closed under a drained scope.
    pub const ROUTE_DRAINED: &str = "route_drained";
}

/// `provider_code` values a runner writes on a run that ends `error`
/// because of a provider the session depends on.
pub mod provider_codes {
    /// Setup only: failed or timed out with no answer. A step-call failure
    /// uses the last view instead and never writes this code.
    pub const COMPACTION_UNAVAILABLE: &str = "compaction_unavailable";
    /// A compaction WAIT reached its cap and the request could not be shown
    /// to fit.
    pub const COMPACTION_WAIT_EXCEEDED: &str = "compaction_wait_exceeded";
    /// A PreUser hook under `on_unavailable: refuse` was unavailable for a
    /// user turn.
    pub const PRE_USER_UNAVAILABLE: &str = "pre_user_unavailable";

    /// The request exceeds the model's context window.
    pub const CONTEXT_OVERFLOW: &str = "context_overflow";
    /// The provider's output-token ceiling prevents a complete response.
    pub const OUTPUT_CEILING: &str = "output_ceiling";
    /// The provider's response deadline ended the request.
    pub const RESPONSE_DEADLINE: &str = "response_deadline";
    /// The account's usage limit is exhausted.
    pub const ACCOUNT_LIMIT_EXHAUSTED: &str = "account_limit_exhausted";
    /// The account's billing or spending allowance is exhausted.
    pub const BILLING_EXHAUSTED: &str = "billing_exhausted";
    /// The provider served an account other than the run's frozen account.
    pub const ACCOUNT_CHANGED: &str = "account_changed";
    /// A provider content-policy refusal.
    pub const CYBER_POLICY: &str = "cyber_policy";
    /// A provider content-policy refusal.
    pub const BIO_POLICY: &str = "bio_policy";
    /// A provider content-policy refusal.
    pub const MISALIGNMENT_POLICY_VIOLATION: &str = "misalignment_policy_violation";
    /// The normalized safety finish, a provider policy refusal.
    pub const CONTENT_FILTER: &str = "content_filter";
    /// Access terminated: the seat cannot run.
    pub const ACCESS_TERMINATED_ERROR: &str = "access_terminated_error";
    /// Read-only legacy alias. Runners must emit `billing_exhausted` instead.
    pub const INSUFFICIENT_QUOTA: &str = "insufficient_quota";

    /// Normalize recorded legacy codes for consumer decisions, preserving
    /// unknown codes verbatim. This does not rewrite stored run errors.
    pub fn normalize(code: &str) -> &str {
        if code == INSUFFICIENT_QUOTA {
            BILLING_EXHAUSTED
        } else {
            code
        }
    }

    /// Every provider code this role names; providers may define others.
    pub const CODES: &[&str] = &[
        COMPACTION_UNAVAILABLE,
        COMPACTION_WAIT_EXCEEDED,
        PRE_USER_UNAVAILABLE,
        CONTEXT_OVERFLOW,
        OUTPUT_CEILING,
        RESPONSE_DEADLINE,
        ACCOUNT_LIMIT_EXHAUSTED,
        BILLING_EXHAUSTED,
        ACCOUNT_CHANGED,
        CYBER_POLICY,
        BIO_POLICY,
        MISALIGNMENT_POLICY_VIOLATION,
        CONTENT_FILTER,
        ACCESS_TERMINATED_ERROR,
        INSUFFICIENT_QUOTA,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn prefrontal_plan_drift_bytes_round_trip() {
        for name in [
            "refuse-later-send-with-different-plan.json",
            "refuse-later-plan-after-planless-first-episode.json",
        ] {
            let file = vectors::load(name);
            assert_eq!(file["expected"]["code"], PLAN_DRIFT);
            assert!(!is_retryable(PLAN_DRIFT));
            let detail: PlanDriftDetail = vectors::exact_object_round_trip(name, "\"detail\": ", 4);
            assert_eq!(
                detail.frozen.is_some(),
                name == "refuse-later-send-with-different-plan.json"
            );
        }
    }

    #[test]
    fn prefrontal_tool_name_collision_bytes_round_trip() {
        let name = "refuse-tool-name-collision.json";
        let file = vectors::load(name);
        assert_eq!(file["expected"]["code"], TOOL_NAME_COLLISION);
        let detail: ToolNameCollisionDetail =
            vectors::exact_object_round_trip(name, "\"detail\": ", 4);
        assert_eq!(detail.name, "web_search");
        assert_eq!(detail.providers, ["plexus", "prefrontal-core"]);
    }

    #[test]
    fn newly_pinned_provider_codes_and_legacy_alias_are_named() {
        for code in [
            "context_overflow",
            "output_ceiling",
            "response_deadline",
            "account_limit_exhausted",
            "billing_exhausted",
            "account_changed",
            "cyber_policy",
            "bio_policy",
            "misalignment_policy_violation",
            "content_filter",
            "access_terminated_error",
            "insufficient_quota",
        ] {
            assert!(provider_codes::CODES.contains(&code), "{code}");
        }
        assert_eq!(
            provider_codes::normalize("insufficient_quota"),
            "billing_exhausted"
        );
        assert_eq!(
            provider_codes::normalize("billing_exhausted"),
            "billing_exhausted"
        );
        assert_eq!(provider_codes::normalize("future-code"), "future-code");
        assert!(CODES.contains(&"send_retracted"));
        assert!(!is_retryable("send_retracted"));
    }

    #[test]
    fn reply_too_large_is_named_terminal_and_carries_only_safe_details() {
        assert!(CODES.contains(&REPLY_TOO_LARGE));
        assert!(!is_retryable(REPLY_TOO_LARGE));

        let file = vectors::load("reply-too-large.json");
        for case in vectors::cases(&file, "refusals") {
            let refusal = &case["refusal"];
            assert_eq!(refusal["code"], REPLY_TOO_LARGE);
            let detail: ReplyTooLargeDetail =
                serde_json::from_value(refusal["detail"].clone()).unwrap();
            assert_eq!(detail.limit, 4_194_304);
            assert!(detail.bytes > detail.limit);
            assert_eq!(
                serde_json::to_value(detail).unwrap(),
                refusal["detail"],
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn retryability_matches_the_vectors() {
        let file = vectors::load("errors.json");
        let listed: Vec<&str> = vectors::cases(&file, "codes")
            .iter()
            .map(|case| case["code"].as_str().unwrap())
            .collect();
        for code in CODES {
            assert!(listed.contains(code), "{code} has no vector");
        }
        for case in vectors::cases(&file, "codes") {
            let code = case["code"].as_str().unwrap();
            assert_eq!(
                is_retryable(code),
                case["retryable"].as_bool().unwrap(),
                "{code}"
            );
        }
    }

    #[test]
    fn compaction_unavailable_decodes_and_is_listed_as_a_provider_code() {
        let file = vectors::load("run-result.json");
        let case = vectors::cases(&file, "answers")
            .iter()
            .find(|case| case["name"] == "Setup unavailable with no answer")
            .unwrap();
        let result: crate::run::RunResult =
            vectors::round_trip("compaction unavailable", &case["answer"]);
        let error = result.error.unwrap();
        assert_eq!(error.provider_code, provider_codes::COMPACTION_UNAVAILABLE);
        assert!(provider_codes::CODES.contains(&"compaction_unavailable"));
        assert!(!CODES.contains(&"compaction_unavailable"));
    }

    #[test]
    fn refused_field_reads_only_field_naming_codes() {
        let detail = serde_json::json!({"field": "delivery"});
        assert_eq!(
            refused_field(SEND_ID_REUSE, Some(&detail)),
            Some("delivery")
        );
        assert_eq!(
            refused_field(INVALID_PARAMS, Some(&detail)),
            Some("delivery")
        );
        assert_eq!(refused_field(UNKNOWN_MID, Some(&detail)), None);
        assert_eq!(refused_field(INVALID_PARAMS, None), None);
    }
}

#[cfg(test)]
mod context_ceiling_tests {
    use super::*;
    use crate::{
        run::{RunResult, RunState, RunStatus},
        vectors,
    };

    #[test]
    fn context_ceiling_is_a_terminal_refusal_and_a_non_error_run_state() {
        assert!(CODES.contains(&CONTEXT_CEILING));
        assert!(!is_retryable(CONTEXT_CEILING));
        assert!(!provider_codes::CODES.contains(&CONTEXT_CEILING));
        assert_eq!(tool_result_reasons::CONTEXT_CEILING, CONTEXT_CEILING);
        let file = vectors::load("context-ceiling.json");
        let mut causes = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            assert_eq!(case["refusal"]["code"], CONTEXT_CEILING, "{name}");
            assert_eq!(case["writes"], "nothing", "{name}");
            let detail: ContextCeilingDetail =
                vectors::round_trip(name, &case["refusal"]["detail"]);
            causes.insert(detail.cause);
        }
        for cause in context_ceiling_causes::ALL {
            assert!(causes.contains(*cause), "no refusal vector for {cause}");
        }
        for case in vectors::cases(&file, "results") {
            let name = case["name"].as_str().unwrap();
            let (state, detail) = match case["kind"].as_str().unwrap() {
                "run.result" => {
                    let result: RunResult = vectors::round_trip(name, &case["answer"]);
                    assert!(result.error.is_none(), "{name}");
                    (result.run_state(), result.context_ceiling)
                }
                _ => {
                    let status: RunStatus = vectors::round_trip(name, &case["answer"]);
                    assert!(status.error.is_none(), "{name}");
                    (status.run_state(), status.context_ceiling)
                }
            };
            assert!(state.is_context_ceiling() && state.is_terminal(), "{name}");
            assert!(!state.is_budget_exhausted(), "{name}");
            assert!(detail.is_some(), "{name}");
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            let detail: ContextCeilingDetail = serde_json::from_value(case["detail"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(detail.cause, case["detail"]["cause"], "{name}");
        }
        // While held at the ceiling the run is active and waiting, with a
        // deadline, on run.status, session.head and the stream.
        let waiting = &file["waiting"];
        let status: RunStatus = vectors::round_trip("waiting status", &waiting["status"]);
        assert_eq!(status.run_state(), RunState::Active);
        assert_eq!(
            status.reason,
            Some(crate::run::RunReason::ContextCeilingWait)
        );
        assert_eq!(
            status,
            RunStatus::context_ceiling_wait(status.deadline_ms.expect("deadline_ms"))
        );
        let head: crate::read::HeadMeta = vectors::round_trip("waiting head", &waiting["head"]);
        let last = head.last_run_state.unwrap();
        assert_eq!(
            last.reason.as_deref(),
            Some(crate::RUN_REASON_CONTEXT_CEILING_WAIT)
        );
        assert!(!RunState::parse(&last.state).is_terminal());
        let event: crate::subscribe::SubscribeEvent =
            serde_json::from_value(waiting["event"].clone()).unwrap();
        assert_eq!(event.kind, crate::subscribe::kinds::CONTROL);
        assert_eq!(
            waiting["event"]["reason"],
            crate::RUN_REASON_CONTEXT_CEILING_WAIT
        );
        assert_eq!(
            waiting["event"]["deadline_ms"],
            waiting["status"]["deadline_ms"]
        );
        for end in vectors::cases(waiting, "ends") {
            let name = end["name"].as_str().unwrap();
            let status: RunStatus = vectors::round_trip(name, &end["status"]);
            assert!(status.deadline_ms.is_none(), "{name}");
            assert_ne!(
                status.reason,
                Some(crate::run::RunReason::ContextCeilingWait),
                "{name}"
            );
        }
        // A provider's own overflow refusal stays a provider failure.
        let overflow: RunResult =
            vectors::round_trip("provider overflow", &file["provider_overflow"]["answer"]);
        assert_eq!(overflow.run_state(), RunState::Error);
        assert!(overflow.context_ceiling.is_none());
        assert_eq!(
            overflow.error.unwrap().provider_code,
            provider_codes::CONTEXT_OVERFLOW
        );
        assert_eq!(
            ContextCeilingDetail::new(1, 2, context_ceiling_causes::OVER_WINDOW),
            serde_json::from_value(serde_json::json!({"window":1,"size":2,"cause":"over_window"}))
                .unwrap()
        );
    }
}

#[cfg(test)]
mod tag_code_tests {
    use super::*;

    #[test]
    fn unknown_tag_is_named_and_terminal() {
        assert!(CODES.contains(&UNKNOWN_TAG));
        assert!(!is_retryable(UNKNOWN_TAG));
    }
}

#[cfg(test)]
mod retention_tests {
    use super::{is_retryable, ExpiredDetail, CODES, EXPIRED};

    #[test]
    fn expired_is_named_terminal_and_carries_expiry_time() {
        assert!(CODES.contains(&EXPIRED));
        assert!(!is_retryable(EXPIRED));
        let detail = ExpiredDetail::new(1234);
        assert_eq!(
            serde_json::to_value(&detail).unwrap(),
            serde_json::json!({"expired_at_ms":1234})
        );
        assert_eq!(
            serde_json::from_str::<ExpiredDetail>(r#"{"expired_at_ms":1234,"future":true}"#)
                .unwrap(),
            detail
        );
        assert!(serde_json::from_str::<ExpiredDetail>("{}").is_err());
    }
}
