//! The `run_tool_calls` group: `run.tool_calls`, every tool call a run
//! made, as a list a consumer reads without decoding any message body.
//!
//! A runner that declares `run_tool_calls`
//! ([`crate::capabilities::RUN_TOOL_CALLS`]) answers `run.tool_calls
//! {run_id, after?, limit?}` with the run's calls in dispatch order: the
//! order the model asked for them across the run's steps, which is the
//! order the runner dispatches them, with a call never dispatched keeping
//! its place. The list pages the way `session.read` does: a page stops by
//! count and by bytes, and carries `next`, an opaque cursor the caller
//! passes back as `after`, whenever more calls follow. A single call larger
//! than the runner's byte cap comes alone on its page; one too large for a
//! transport frame on its own is refused `reply_too_large`, naming the run
//! and the call. `args` is never cut short.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A call's outcome, decoded as an open string: a consumer treats one it
/// does not know as not known to have succeeded.
pub mod outcomes {
    /// The call executed and its result is not an error.
    pub const SUCCEEDED: &str = "succeeded";
    /// The call executed and its result is an error.
    pub const FAILED: &str = "failed";
    /// The call was refused before it executed: a hook denied it, a person
    /// declined it or the approval expired, its route was drained, or its
    /// tool provider refused it. No dispatch happened, or it was refused at
    /// the provider without running.
    pub const REFUSED: &str = "refused";
    /// A hard stop gave the call the runner's never-executed result before
    /// dispatch (CONTRACT §6): `max_steps`, `stop_when`, `budget_exhausted`,
    /// `context_ceiling`, `skipped_after_submit`, or a run sealed
    /// `interrupted` with the call never sent. No dispatch intent exists.
    pub const NOT_EXECUTED: &str = "not_executed";
    /// The call has a durable dispatch intent and no result (`session.read`'s
    /// `indeterminate: true`), or it was closed `outcome_unknown` after a
    /// restart: it may or may not have run.
    pub const INDETERMINATE: &str = "indeterminate";
    /// Every outcome this role names.
    pub const ALL: &[&str] = &[SUCCEEDED, FAILED, REFUSED, NOT_EXECUTED, INDETERMINATE];
}

/// The `field` an `invalid_params` refusal names for a cursor the runner
/// never gave for this run.
pub const AFTER_FIELD: &str = "after";

/// The `field` an `invalid_params` refusal names for a `limit` of zero.
pub const LIMIT_FIELD: &str = "limit";

/// The `run.tool_calls` request: `{run_id, after?, limit?}`.
///
/// Strict: an unknown field is refused `invalid_params` naming it, because a
/// misspelled `after` would otherwise answer the first page again.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`ToolCallsRequest::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ToolCallsRequest {
    pub run_id: String,
    /// The cursor a page's `next` gave: answer the calls after it. Opaque to
    /// the caller, which only passes back a value the runner returned for
    /// this run. Absent means from the run's first call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// The most calls the page may hold. Absent means the runner's default;
    /// a value above the runner's maximum is capped to it; zero is refused
    /// naming `limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

impl ToolCallsRequest {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            after: None,
            limit: None,
        }
    }

    pub fn with_after(mut self, after: impl Into<String>) -> Self {
        self.after = Some(after.into());
        self
    }

    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// [`LIMIT_FIELD`] for a `limit` of zero, which asks for no call.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.limit == Some(0) {
            Err(LIMIT_FIELD)
        } else {
            Ok(())
        }
    }
}

/// One tool call of a run.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`RunToolCall::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunToolCall {
    /// The model's id for the call. Display only: it is not unique.
    pub tool_call_id: String,
    /// The call's key, as `session.read` exposes it (CONTRACT §5, §11.3):
    /// present once the call was prepared or dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    /// The model-facing tool name.
    pub name: String,
    /// The arguments exactly as dispatched; for a call never dispatched,
    /// exactly as the model asked for them.
    pub args: Value,
    /// One of [`outcomes`]; any other value decodes as itself.
    pub outcome: String,
    /// The length in bytes of the result the runner stored for the call, in
    /// its own stored encoding. Absent when no result is stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_bytes: Option<u64>,
    /// The module the call was dispatched to. Absent for a call never
    /// dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_to: Option<String>,
    /// The ordinal of the transcript message carrying the call.
    pub ordinal: u64,
}

impl RunToolCall {
    pub fn new(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        args: Value,
        outcome: impl Into<String>,
        ordinal: u64,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            call_key: None,
            name: name.into(),
            args,
            outcome: outcome.into(),
            result_bytes: None,
            dispatched_to: None,
            ordinal,
        }
    }

    pub fn with_call_key(mut self, call_key: impl Into<String>) -> Self {
        self.call_key = Some(call_key.into());
        self
    }

    pub fn with_result_bytes(mut self, result_bytes: u64) -> Self {
        self.result_bytes = Some(result_bytes);
        self
    }

    pub fn with_dispatched_to(mut self, module: impl Into<String>) -> Self {
        self.dispatched_to = Some(module.into());
        self
    }
}

/// A page of the `run.tool_calls` answer. Decoded leniently.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`ToolCallsPage::new`] and [`ToolCallsPage::with_next`].
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolCallsPage {
    pub run_id: String,
    /// The calls after the request's cursor, in dispatch order.
    pub calls: Vec<RunToolCall>,
    /// The cursor of the next page, passed back as `after`. Present exactly
    /// when the page stops before the end of the run's list as it stands;
    /// a run still in progress may add calls after a last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

impl ToolCallsPage {
    pub fn new(run_id: impl Into<String>, calls: Vec<RunToolCall>) -> Self {
        Self {
            run_id: run_id.into(),
            calls,
            next: None,
        }
    }

    pub fn with_next(mut self, next: impl Into<String>) -> Self {
        self.next = Some(next.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};
    use serde_json::json;

    #[test]
    fn tool_call_requests_round_trip_and_refuse_by_field() {
        let file = vectors::load("run-tool-calls.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<ToolCallsRequest>(
                case["name"].as_str().unwrap(),
                &case["request"],
            );
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            let error = serde_json::from_value::<ToolCallsRequest>(case["request"].clone())
                .expect_err(name);
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
    }

    #[test]
    fn pages_list_calls_in_order_and_carry_next() {
        let file = vectors::load("run-tool-calls.json");
        let mut outcomes_seen = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "pages") {
            let name = case["name"].as_str().unwrap();
            let page: ToolCallsPage = vectors::round_trip(name, &case["page"]);
            let request: ToolCallsRequest = vectors::round_trip(name, &case["request"]);
            assert_eq!(request.check(), Ok(()), "{name}");
            assert_eq!(page.run_id, request.run_id, "{name}");
            assert!(!page.calls.is_empty() || page.next.is_none(), "{name}");
            if let Some(limit) = request.limit {
                assert!(page.calls.len() as u64 <= limit, "{name}");
            }
            let mut last = None;
            for call in &page.calls {
                assert!(call.args.is_object(), "{name}: args must be an object");
                assert!(
                    last.is_none_or(|last| last <= call.ordinal),
                    "{name}: out of order"
                );
                last = Some(call.ordinal);
                outcomes_seen.insert(call.outcome.clone());
                let dispatched = call.dispatched_to.is_some();
                match call.outcome.as_str() {
                    outcomes::NOT_EXECUTED => assert!(!dispatched, "{name}"),
                    outcomes::SUCCEEDED | outcomes::FAILED => {
                        assert!(dispatched && call.result_bytes.is_some(), "{name}")
                    }
                    _ => {}
                }
            }
        }
        for outcome in outcomes::ALL {
            assert!(outcomes_seen.contains(*outcome), "no vector for {outcome}");
        }
        let lenient: ToolCallsPage =
            serde_json::from_value(json!({"run_id": "r", "calls": [], "future": 1})).unwrap();
        assert_eq!(lenient, ToolCallsPage::new("r", Vec::new()));
        let future: RunToolCall = serde_json::from_value(json!({
            "tool_call_id": "t", "name": "n", "args": {}, "outcome": "future", "ordinal": 1
        }))
        .unwrap();
        assert_eq!(future.outcome, "future");
    }

    #[test]
    fn refusals_are_unknown_run_and_reply_too_large_naming_the_entry() {
        let file = vectors::load("run-tool-calls.json");
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            let code = case["refusal"]["code"].as_str().unwrap();
            assert!(errors::CODES.contains(&code), "{name}");
            assert!(!errors::is_retryable(code), "{name}");
            if code == errors::INVALID_PARAMS {
                let field = case["refusal"]["detail"]["field"].as_str().unwrap();
                assert!([AFTER_FIELD, LIMIT_FIELD].contains(&field), "{name}");
                if field == LIMIT_FIELD {
                    let request: ToolCallsRequest =
                        serde_json::from_value(case["request"].clone()).unwrap();
                    assert_eq!(request.check(), Err(LIMIT_FIELD), "{name}");
                }
            } else if code == errors::REPLY_TOO_LARGE {
                let detail: errors::ReplyTooLargeDetail =
                    vectors::round_trip(name, &case["refusal"]["detail"]);
                assert!(matches!(
                    detail.subject,
                    errors::ReplyTooLargeSubject::ToolCall { .. }
                ));
            } else {
                assert_eq!(code, errors::UNKNOWN_RUN, "{name}");
            }
        }
    }
}
