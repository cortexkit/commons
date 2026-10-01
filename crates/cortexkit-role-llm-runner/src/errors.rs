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

/// Admission found two tools with the same model-facing name.
/// `detail.tools` names both. Nothing was written.
pub const TOOL_NAME_COLLISION: &str = "tool_name_collision";

/// A later send carried a plan that differs from the session's frozen plan.
/// Nothing was applied.
pub const PLAN_CHANGED: &str = "plan_changed";

/// A steered or queued send whose PreUser hook was unavailable under
/// `on_unavailable: refuse`. Nothing was written.
pub const PRE_USER_UNAVAILABLE: &str = "pre_user_unavailable";

/// The runner could not answer now for a reason that may clear by itself
/// (a provider or store briefly unavailable). Retryable.
pub const TRANSIENT: &str = "transient";

/// The scope's owner has not yet synced the scope after a daemon restart.
/// Retryable, with backoff, within the caller's hold bound.
pub const SCOPE_NOT_SYNCED: &str = "scope_not_synced";

/// Every refusal code this role defines.
pub const CODES: &[&str] = &[
    INVALID_PARAMS,
    LINEAGE_CHANGED,
    UNKNOWN_MID,
    UNKNOWN_RUN,
    SEND_ID_REUSE,
    RUN_PAUSED,
    SCOPE_OWNER_MISMATCH,
    SCOPE_ADOPTION_REFUSED,
    SCOPE_UNSUPPORTED,
    FETCH_UNAVAILABLE,
    PLAN_STALE,
    TOOL_NAME_COLLISION,
    PLAN_CHANGED,
    PRE_USER_UNAVAILABLE,
    TRANSIENT,
    SCOPE_NOT_SYNCED,
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
    /// A compaction WAIT reached its cap and the request could not be shown
    /// to fit.
    pub const COMPACTION_WAIT_EXCEEDED: &str = "compaction_wait_exceeded";
    /// A PreUser hook under `on_unavailable: refuse` was unavailable for a
    /// user turn.
    pub const PRE_USER_UNAVAILABLE: &str = "pre_user_unavailable";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

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
