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

/// A read's `lineage_id` is not the session's current lineage. The caller
/// re-reads from the tail; it never retries the same cursor.
pub const LINEAGE_CHANGED: &str = "lineage_changed";

/// A read's `after_mid` names no message in the lineage the read named.
pub const UNKNOWN_MID: &str = "unknown_mid";

/// `run.result` names a run the session does not have.
pub const UNKNOWN_RUN: &str = "unknown_run";

/// A send reused a `send_id` with a different payload or delivery mode.
/// `detail.field` names the first field that differs, for example
/// `delivery`.
pub const SEND_ID_REUSE: &str = "send_id_reuse";

/// A send whose delivery mode (`queue` when absent) the runner does not
/// declare as a capability group. `detail.delivery` names the mode. Nothing
/// was written, and the send is never delivered in another mode instead.
pub const DELIVERY_UNSUPPORTED: &str = "delivery_unsupported";

/// A send into a session whose last run is paused, other than the paused
/// run's own retry or an interrupt. Nothing was written.
pub const RUN_PAUSED: &str = "run_paused";

/// The caller is not the identity allowed to drive the session: a send, or
/// a `session.baseline` request, from anyone but the session's owner.
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

/// A later send carried a plan that differs from the session's frozen plan.
/// `detail` is [`PlanDriftDetail`]. Nothing was applied.
pub const PLAN_DRIFT: &str = "plan_drift";

/// The identities of the frozen and sent plans in a `plan_drift` refusal.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PlanDriftDetail {
    /// Absent when the session's first episode had no plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<String>,
    pub sent: String,
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
    UNKNOWN_RUN,
    SEND_ID_REUSE,
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
    /// Setup or a compaction call failed or timed out with no answer.
    pub const COMPACTION_UNAVAILABLE: &str = "compaction_unavailable";
    /// A compaction WAIT reached its cap and the request could not be shown
    /// to fit.
    pub const COMPACTION_WAIT_EXCEEDED: &str = "compaction_wait_exceeded";
    /// A PreUser hook under `on_unavailable: refuse` was unavailable for a
    /// user turn.
    pub const PRE_USER_UNAVAILABLE: &str = "pre_user_unavailable";

    /// Every provider code this role names; providers may define others.
    pub const CODES: &[&str] = &[
        COMPACTION_UNAVAILABLE,
        COMPACTION_WAIT_EXCEEDED,
        PRE_USER_UNAVAILABLE,
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
            .find(|case| case["name"] == "compaction call unavailable with no answer")
            .unwrap();
        let result: crate::run::RunResult =
            vectors::round_trip("compaction unavailable", &case["answer"]);
        let error = result.error.unwrap();
        assert_eq!(
            error["provider_code"].as_str(),
            Some(provider_codes::COMPACTION_UNAVAILABLE)
        );
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
