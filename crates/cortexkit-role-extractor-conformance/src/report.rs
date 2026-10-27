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
        "role.describe decodes, passes check_describe, names extractor/v1, and lists exactly the served models, each with its pass_tokens, context_tokens (absent when the provider states none), image limits (null for none), an input price exactly when the catalog prices it, and its egress host",
    ),
    check(
        "bad_batch_id_refused",
        "a batch_id with a tab, and one of 129 characters, are refused invalid_params naming batch_id, class permanent, and call nothing",
    ),
    check(
        "unknown_model_refused",
        "a model the catalog does not have is refused model_unknown naming the model, calls nothing, and writes nothing",
    ),
    check(
        "non_extraction_model_refused",
        "a catalog row whose kind is not extraction is refused model_not_extractor with detail.model and detail.kind, calls nothing, and writes nothing",
    ),
    check(
        "kindless_model_refused",
        "a catalog row that states no kind is refused model_not_extractor with detail.model and no detail.kind, never model_unknown, calls nothing, and writes nothing",
    ),
    check(
        "field_id_refused_naming_field",
        "a field id with a dot, and one starting with a digit, are refused invalid_params naming fields.<id>, call nothing, and write nothing",
    ),
    check(
        "unknown_field_type_refused_before_options",
        "a field of unknown type rank that also carries a misspelt option is refused naming fields.<id>.type, not the option, calls nothing, and writes nothing",
    ),
    check(
        "misspelt_or_misplaced_option_refused",
        "a misspelt option (decimal) and an option on a type that does not take it (integer on a text-like field) are refused naming fields.<id>.<option>, never dropped, call nothing, and write nothing",
    ),
    check(
        "missing_required_option_refused",
        "a list without items, an estimate without max and a forecast without horizon are refused naming the missing option, for each of those types the model serves, call nothing, and write nothing",
    ),
    check(
        "option_value_out_of_range_refused",
        "null_threshold 0, and for the types the model serves decimals 7, horizon 65 and an estimate whose min is not below max, are refused naming the option, call nothing, and write nothing",
    ),
    check(
        "instructions_refused_empty_or_over_bound",
        "empty instructions, and instructions one byte over limits.max_instruction_bytes, are refused naming fields.<id>.instructions (the second with the limit), call nothing, and write nothing",
    ),
    check(
        "item_state_and_reference_date_refused",
        "a number state, an empty state without images, and reference_date 2026-02-30 are refused naming items[i].state or items[i].reference_date, call nothing, and write nothing",
    ),
    check(
        "unknown_member_refused",
        "an unknown request member, an unknown item member and an unknown options member are refused naming their path, never dropped, call nothing, and write nothing",
    ),
    check(
        "counts_refused_with_limit",
        "no fields, one field over max_fields, and one item over max_items are refused naming fields or items with the bound broken, call nothing, and write nothing",
    ),
    check(
        "images_refused_before_any_call",
        "on a model that takes images, a remote URL, malformed base64 and one image over images.max are refused naming the image or the list, with the limit; on a model that takes none, any image is refused with limit 0; none calls anything or writes anything",
    ),
    check(
        "oversize_request_refused",
        "a request over limits.max_request_bytes is refused invalid_params naming params with the limit, calls nothing, and writes nothing",
    ),
    check(
        "reply_over_frame_refused",
        "a batch whose estimated reply crosses the 4 MiB reply frame (by fields x max_answer_bytes, and echoed state text) is refused naming items with limit 4194304, calls nothing, and writes nothing",
    ),
    check(
        "clairic_request_mapped",
        "an admitted batch of two items sends the stand-in, for each item, the provider request the role maps it to: the provider's model value, the state, the reference_date, every field key by key in the caller's order, and both options under the provider's names",
    ),
    check(
        "reuse_refused_naming_part",
        "re-sends of a recorded batch_id with changed fields, other options, an added item and a changed item are refused batch_id_reuse naming fields, options, items and items[1]; they call nothing for the new items, and the original body still replays",
    ),
    check(
        "image_spellings_are_one_batch",
        "on a model that takes images, a re-send carrying the same image as {content_type, base64} instead of a data: URL is a replay, not batch_id_reuse",
    ),
    check(
        "zero_ceiling_sends_nothing",
        "on a priced model, a batch with max_cost_usd 0 is answered with cost_exceeded, class permanent, on every item, and the stand-in is never called",
    ),
    check(
        "provider_answers_forwarded",
        "with the stand-in replaying Clairic's reply to a request of all nine field types (probe 01, sent again with quantiles and state_text), the item's answers equal what provider::read_reply reads from that reply: every answer, of every type, with every member's text as the provider wrote it, and truncated and usage as given",
    ),
    check(
        "spans_forwarded_unconverted",
        "with the stand-in replaying Clairic's reply to probe 02 (a state with an accented letter, an emoji and CJK characters before the target), the answer's span is [53,61], in Unicode scalar values as the provider wrote it, never converted to bytes or UTF-16 units, and state_text is the state byte for byte",
    ),
    check(
        "provider_errors_mapped",
        "with the stand-in replaying Clairic's three error replies (HTTP 400 code 5006, and HTTP 422 code 3030 twice, each with errors first and result {}), each item gets invalid_item, class permanent, with the Workers AI code in its message, no answers and no reported input_tokens",
    ),
    check(
        "cost_usd_from_reported_tokens",
        "on a priced model that takes list fields, with the stand-in replaying Clairic's reply to probe 03, the reply's cost_usd is the reported input_tokens (304) at the model's input price, never the pre-call estimate",
    ),
    check(
        "image_reference_follows_declaration",
        "on a model that takes images, an item image given as a reference {handle} the module never issued is refused before any call naming items[0].images[0]: image_unavailable, class permanent, when role.describe declares image_reference, and invalid_params (an unsupported image) when it does not; the refusal writes nothing",
    ),
    check(
        "image_reference_resolved_and_answered",
        "with a module that declares image_reference, a reference the subject's hook issued for a one-pixel PNG is admitted, and the stand-in receives the item with the image's bytes as {content_type, base64}, never the handle; not applicable without the hook",
    ),
    check(
        "image_reference_oversized_refused_like_inline",
        "with a module that declares image_reference, a reference the hook issued for one byte over the model's images.max_bytes_each is refused invalid_params naming items[0].images[0], the refusal an inline image of that size gets, before any call and writing nothing; not applicable without the hook",
    ),
    check(
        "image_source_down_refused_transient",
        "with a module that declares image_reference, while the hook's image source is down a reference is refused image_source_unavailable, class transient, naming items[0].images[0] with detail.retry_after_ms, before any call; once the source is back the same body is admitted, so the refusal wrote nothing; not applicable without the hook",
    ),
    check(
        "image_reference_identity_is_its_handle",
        "with a module that declares image_reference, a re-send carrying the same handle is not batch_id_reuse, and a re-send carrying the same picture inline instead of the handle is refused batch_id_reuse naming items[0]; not applicable without the hook",
    ),
];

/// Where the suite checks less than the contract says, and why. Every
/// report prints these.
pub const NARROWINGS: &[&str] = &[
    "The reply side is checked with the captured Clairic replies of probes 01, 02, 03, 08, 09 and 11 only, each replayed byte for byte; answer_too_large is not checked, and cost_usd only for a batch of one answered item.",
    "Only the first served model listed is driven; the other served models are checked in role.describe only.",
    "The pre-call spend check is checked with a zero ceiling only; the estimate's exact figure depends on the module's own bounds (image tokens and overhead), which the suite does not know.",
    "Batch-level provider errors (auth_failed, model_unavailable, account_limit_exhausted, account_not_entitled on the first provider call) are not checked: the live probe captured none of them.",
    "Single flight and stored outcomes on a re-send are not checked here.",
    "The provider body limit is not checked: reaching it needs a request larger than most modules' max_request_bytes.",
    "Image references are checked on the first served model only, with one image per item; the pixel limit of a resolved image is not checked.",
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
