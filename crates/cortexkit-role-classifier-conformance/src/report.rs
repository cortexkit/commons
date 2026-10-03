//! The suite's checks and what a run reports.

/// One check: its stable name and what it asserts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckSpec {
    pub name: &'static str,
    pub checks: &'static str,
}

const fn check(name: &'static str, checks: &'static str) -> CheckSpec {
    CheckSpec { name, checks }
}

/// Every check, in the order the suite runs them.
pub const CHECKS: &[CheckSpec] = &[
    check(
        "describe_complete",
        "role.describe decodes, passes check_describe, names classifier/v1, and lists exactly the served models, each with the catalog's context_tokens, image limit (null for none), a price exactly when the catalog prices it, and its egress host",
    ),
    check(
        "concurrent_call_refused_batch_in_progress",
        "while one call holds a batch (the stand-in holds its first item), a second call with the same batch_id and body is refused batch_in_progress, class transient, with retry_after_ms, at once; the stand-in is called once per item; a later re-send returns every answer and calls nothing",
    ),
    check(
        "retry_calls_only_unanswered_items",
        "a re-send with the same body returns the stored answer of an answered item unchanged, calls the stand-in for that item no more, and calls it once more for the item that had failed transiently",
    ),
    check(
        "stored_permanent_error_replayed_transient_retried",
        "on a re-send, an item whose stored error is permanent (stand-in 400) is returned with that error as stored and no call; an item whose stored error is transient (stand-in 503) is called again and its new answer replaces the error",
    ),
    check(
        "cost_usd_stable_on_replay",
        "on a priced model, cost_usd is present on the first reply and has the same spelling on a full replay that calls nothing",
    ),
    check(
        "looser_max_cost_does_not_raise_ceiling",
        "after the first call's spend reached its max_cost_usd, a re-send with a higher max_cost_usd is not batch_id_reuse, and the item left unanswered gets cost_exceeded (as an item error or an admission refusal) without a stand-in call",
    ),
    check(
        "tighter_max_cost_stops_crossing_items",
        "a re-send with a max_cost_usd below the batch's recorded spend is not batch_id_reuse, and the item left unanswered gets cost_exceeded (as an item error or an admission refusal) without a stand-in call",
    ),
    check(
        "tightened_ceiling_recorded_across_resends",
        "three calls of one batch carry a high max_cost_usd, then a lower one, then the original high one again; both items fail transiently (stand-in 503) on the first two, and on the third item 0 is answered at a cost over the lower ceiling: item 1 gets cost_exceeded without a stand-in call, because the lower ceiling was recorded",
    ),
    check(
        "reuse_refused_naming_field",
        "re-sends of a recorded batch_id with a changed item, an added item, changed questions or another model are refused batch_id_reuse, class permanent, naming items[i], items, questions and model; they call nothing, and the original body still replays",
    ),
    check(
        "reordered_object_state_is_replay",
        "a re-send whose object state has its keys (nested ones too) in another order is a replay: the same answers, no batch_id_reuse, no new call",
    ),
    check(
        "validation_refusals_reach_no_provider",
        "a bad batch_id, an unknown model, a non-classifier model, too many questions, a bad question id, a one-option choice, an eleven-level score, too many items, a number state, unsupported images and an oversize request are refused with their code and detail.field before any stand-in call, and write nothing: a valid body with the same batch_id is then admitted",
    ),
    check(
        "null_state_refused_naming_field",
        "an item with a null state is refused invalid_params naming items[1].state, calls nothing, and writes nothing",
    ),
    check(
        "unknown_question_type_refused_naming_field",
        "a request with a question of an unknown type (rank) is refused invalid_params naming questions.<id>.type with the question's key, calls nothing, and writes nothing",
    ),
    check(
        "answers_in_request_order",
        "five items come back in request order, each at its own index with its own scripted answer",
    ),
    check(
        "provider_numbers_byte_for_byte",
        "every number of the stand-in's answers (noul, choice probabilities and confidence, score, score probabilities and confidence), spelled with extra digits, trailing zeros, exponents and integers, reaches the reply with the same spelling, on the first reply and on a replay",
    ),
    check(
        "one_failing_item_does_not_fail_batch",
        "a batch whose middle item the stand-in rejects (400) is answered: the other items carry their answers and the middle one a permanent item error",
    ),
    check(
        "auth_failure_stops_the_call_unstored",
        "in a batch of four whose item 1 the stand-in answers 401 after item 0 was answered, items 1 to 3 carry auth_failed, class permanent, and items 2 and 3 are never sent; once the stand-in accepts, a re-send keeps item 0's answer without a call and asks the stand-in again for exactly items 1 to 3",
    ),
    check(
        "auth_failure_on_first_call_refused",
        "a call whose first provider call the stand-in answers 401 is refused auth_failed at admission, class permanent, and sends no further item; once the stand-in accepts, the same body is admitted and both items are asked, so nothing was recorded",
    ),
    check(
        "model_unavailable_stops_the_call_unstored",
        "a stand-in 404 behaves as a 401 does, with model_unavailable: mid-batch it stops the call and is not stored, so a re-send asks again for exactly the unanswered items; on the first provider call it refuses the call at admission naming the model, and records nothing",
    ),
    check(
        "rate_limit_stops_the_rest_of_the_call",
        "in a batch of four whose item 1 the stand-in answers 429 after item 0 was answered, items 1 to 3 carry rate_limited, class transient, and items 2 and 3 are never sent; once the stand-in accepts, a re-send keeps item 0's answer without a call and asks again for items 1 to 3",
    ),
    check(
        "unreported_usage_stays_absent",
        "an item whose provider response has no usage carries no token counts, an item reporting only input_tokens carries no output_tokens, and the batch usage sums input_tokens and leaves output_tokens absent, never zero",
    ),
];

/// Where the suite checks less than the contract says, and why. Every
/// report prints these.
pub const NARROWINGS: &[&str] = &[
    "The suite checks single flight with one held call; it cannot cut a module mid-batch, so a re-send after a crash is checked only as a re-send after a transient error.",
    "The ceiling checks accept cost_exceeded as an item error or as an admission refusal: the contract does not say whether a re-send whose recorded spend already meets the ceiling is admitted. What they require is that the unanswered item is not called and the re-send is not batch_id_reuse.",
    "The ceiling checks make an item's spend cross the ceiling by the usage the stand-in reports, computed at role.describe's price; a module that prices by something other than reported usage may need a larger margin than the suite gives.",
    "The 400 a stand-in answers is mapped to invalid_item by the contract; the suite checks only that the item's error is permanent, not its code.",
    "auth_failed is checked with a 401 only; the contract maps 403 the same way.",
    "A stand-in 429 is answered at once and every time, so the suite cannot tell a module's own retries apart; it checks only that the item ends rate_limited and that the items after it are not sent.",
    "Images are checked only for count and support, not for byte or pixel limits.",
];

/// How one check ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    Passed,
    Failed {
        reason: String,
    },
    /// The module does not do what the check asks about (for example, it
    /// serves no priced model, so it has no cost to compare). Never a pass.
    NotApplicable {
        reason: String,
    },
}

/// One check's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckReport {
    pub check: &'static str,
    pub outcome: CheckOutcome,
}

/// The run's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuiteVerdict {
    /// Every check ran and passed.
    Passed,
    /// No check failed, but these were not applicable. Conforming for what
    /// the module does, and never reported as a full pass.
    PassedExceptNotApplicable {
        not_applicable: Vec<&'static str>,
    },
    Failed {
        reasons: Vec<String>,
    },
}

/// What a run found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuiteReport {
    pub checks: Vec<CheckReport>,
    pub verdict: SuiteVerdict,
}

impl SuiteReport {
    /// The outcome of the check named `name`.
    pub fn outcome(&self, name: &str) -> Option<&CheckOutcome> {
        self.checks
            .iter()
            .find(|report| report.check == name)
            .map(|report| &report.outcome)
    }

    /// The report as text, one line per check, then the narrowings and the
    /// verdict.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for report in &self.checks {
            let line = match &report.outcome {
                CheckOutcome::Passed => format!("PASS {}", report.check),
                CheckOutcome::Failed { reason } => format!("FAIL {}: {reason}", report.check),
                CheckOutcome::NotApplicable { reason } => {
                    format!("N/A  {}: {reason}", report.check)
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
        for narrowing in NARROWINGS {
            out.push_str(&format!("NARROWED {narrowing}\n"));
        }
        let verdict = match &self.verdict {
            SuiteVerdict::Passed => "passed".to_owned(),
            SuiteVerdict::PassedExceptNotApplicable { not_applicable } => format!(
                "passed for what the module does; not applicable: {}",
                not_applicable.join(", ")
            ),
            SuiteVerdict::Failed { reasons } => format!("failed: {}", reasons.join("; ")),
        };
        out.push_str(&format!("VERDICT {verdict}\n"));
        out
    }
}
