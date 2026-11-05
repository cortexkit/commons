//! Capacity-only budget arithmetic for `route.budget_quote` (group
//! `budget_quote`). Capacities come from the retained selection answers,
//! never a fresh catalog lookup. Expiry does not prevent quoting a retained
//! decision. Routers enforce selecting-principal/consumer ownership and answer
//! `unknown_decision` alike for unknown, forgotten, foreign and non-selection
//! ids, including account-admission reasons and continuation offers.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::Refusal;
use crate::members::Members;
use crate::select::SelectAnswer;

open_set! {
    /// Whether the caller sends an output limit.
    pub enum OutputLimitStatus {
        Send => "send",
        Unknown => "unknown",
    }
}

open_set! {
    /// The first applicable reason no output limit can be supplied.
    pub enum OutputLimitReason {
        NoOutputLimit => "no_output_limit",
        NoContextWindow => "no_context_window",
        Placeholder => "placeholder",
        NoRoom => "no_room",
    }
}

/// A request in decision order. Only `select`/`select_panel` decisions,
/// including replacement seats, may be quoted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetQuoteRequest {
    pub decisions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
}

impl BudgetQuoteRequest {
    pub fn new(decisions: Vec<String>) -> Self {
        Self {
            decisions,
            prompt_tokens: None,
        }
    }

    pub fn with_prompt_tokens(mut self, prompt_tokens: u64) -> Self {
        self.prompt_tokens = Some(prompt_tokens);
        self
    }
}

/// Answered capacities for one decision, in the caller's request order.
/// Zero is the only unknown capacity representation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BudgetSeat {
    pub decision_id: String,
    pub route_key: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
}

impl BudgetSeat {
    /// Copy exactly the capacities and route recorded in a selection answer.
    pub fn from_select_answer(answer: &SelectAnswer) -> Self {
        Self::new(
            &answer.decision_id,
            answer.route.route_key(),
            answer.context_window,
            answer.max_output_tokens,
        )
    }

    pub fn new(
        decision_id: impl Into<String>,
        route_key: impl Into<String>,
        context_window: u64,
        max_output_tokens: u64,
    ) -> Self {
        Self {
            decision_id: decision_id.into(),
            route_key: route_key.into(),
            context_window,
            max_output_tokens,
        }
    }
}

/// Identity of the seat constraining the package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LimitingSeat {
    pub decision_id: String,
    pub route_key: String,
}

impl LimitingSeat {
    pub fn new(decision_id: impl Into<String>, route_key: impl Into<String>) -> Self {
        Self {
            decision_id: decision_id.into(),
            route_key: route_key.into(),
        }
    }
}

/// The smallest seat ceiling, with its limiting seat's effective window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PackageBudget {
    pub tokens: u64,
    pub limiting: LimitingSeat,
    pub context_window: u64,
    pub context_window_known: bool,
}

impl PackageBudget {
    pub fn new(
        tokens: u64,
        limiting: LimitingSeat,
        context_window: u64,
        context_window_known: bool,
    ) -> Self {
        Self {
            tokens,
            limiting,
            context_window,
            context_window_known,
        }
    }
}

/// A send limit carries `tokens`, `catalog` and `suspect`; an unknown limit
/// carries `reason`. Optional fields keep unfamiliar statuses decodable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OutputLimit {
    pub status: OutputLimitStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspect: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<OutputLimitReason>,
}

impl OutputLimit {
    pub fn send(tokens: u64, catalog: u64, suspect: bool) -> Self {
        Self {
            status: OutputLimitStatus::Send,
            tokens: Some(tokens),
            catalog: Some(catalog),
            suspect: Some(suspect),
            reason: None,
        }
    }

    pub fn unknown(reason: OutputLimitReason) -> Self {
        Self {
            status: OutputLimitStatus::Unknown,
            tokens: None,
            catalog: None,
            suspect: None,
            reason: Some(reason),
        }
    }
}

/// One quoted seat, preserving request order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetSeatQuote {
    pub decision_id: String,
    pub route_key: String,
    pub package_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_limit: Option<OutputLimit>,
}

impl BudgetSeatQuote {
    pub fn new(
        decision_id: impl Into<String>,
        route_key: impl Into<String>,
        package_tokens: u64,
    ) -> Self {
        Self {
            decision_id: decision_id.into(),
            route_key: route_key.into(),
            package_tokens,
            output_limit: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetQuote {
    pub package: PackageBudget,
    pub seats: Vec<BudgetSeatQuote>,
}

impl BudgetQuote {
    pub fn new(package: PackageBudget, seats: Vec<BudgetSeatQuote>) -> Self {
        Self { package, seats }
    }
}

/// Decode a request after discovery has declared `budget_quote`. Ownership,
/// selection provenance and retention are the router's stateful checks.
pub fn parse_budget_quote_request(params: &Value) -> Result<BudgetQuoteRequest, Refusal> {
    let members = Members::of(params)?;
    let decisions: Vec<String> = members.required("decisions")?;
    check_decisions(decisions.iter().map(String::as_str))?;
    Ok(BudgetQuoteRequest {
        decisions,
        prompt_tokens: members.optional("prompt_tokens")?,
    })
}

fn check_decisions<'a>(decisions: impl IntoIterator<Item = &'a str>) -> Result<(), Refusal> {
    let decisions: Vec<_> = decisions.into_iter().collect();
    if !(1..=16).contains(&decisions.len())
        || decisions.iter().copied().collect::<BTreeSet<_>>().len() != decisions.len()
    {
        return Err(Refusal::invalid_request(
            "decisions",
            "decisions must contain 1 to 16 distinct decision ids",
        ));
    }
    Ok(())
}

/// Quote capacities without I/O or catalog reads. Empty, duplicate or over-16
/// decision lists return `invalid_request {field: "decisions"}`, just like the
/// request parser. All subtraction saturates before division or limiting.
/// Equal ceilings use bytewise route order, then the earlier input decision.
pub fn quote(seats: &[BudgetSeat], prompt_tokens: Option<u64>) -> Result<BudgetQuote, Refusal> {
    check_decisions(seats.iter().map(|seat| seat.decision_id.as_str()))?;
    let mut quoted = Vec::with_capacity(seats.len());
    let mut limiting: Option<PackageBudget> = None;
    for seat in seats {
        let known = seat.context_window != 0;
        let window = if known { seat.context_window } else { 128_000 };
        let output = if seat.max_output_tokens == 0 {
            32_000
        } else {
            seat.max_output_tokens
        };
        let ceiling = window
            .saturating_sub(output.min(64_000))
            .saturating_sub(16_000)
            / 2;
        if limiting.as_ref().is_none_or(|previous| {
            (ceiling, &seat.route_key) < (previous.tokens, &previous.limiting.route_key)
        }) {
            limiting = Some(PackageBudget::new(
                ceiling,
                LimitingSeat::new(&seat.decision_id, &seat.route_key),
                window,
                known,
            ));
        }
        let mut result = BudgetSeatQuote::new(&seat.decision_id, &seat.route_key, ceiling);
        result.output_limit = prompt_tokens.map(|prompt| output_limit(seat, window, prompt));
        quoted.push(result);
    }
    // The decision-list check guarantees a first seat.
    Ok(BudgetQuote::new(limiting.expect("non-empty seats"), quoted))
}

fn output_limit(seat: &BudgetSeat, window: u64, prompt: u64) -> OutputLimit {
    let room = window.saturating_sub(prompt).saturating_sub(4_096);
    let reason = if seat.max_output_tokens == 0 {
        Some(OutputLimitReason::NoOutputLimit)
    } else if seat.context_window == 0 {
        Some(OutputLimitReason::NoContextWindow)
    } else if seat.max_output_tokens >= window {
        Some(OutputLimitReason::Placeholder)
    } else if room == 0 {
        Some(OutputLimitReason::NoRoom)
    } else {
        None
    };
    match reason {
        Some(reason) => OutputLimit::unknown(reason),
        None => OutputLimit::send(
            seat.max_output_tokens.min(room),
            seat.max_output_tokens,
            seat.max_output_tokens > 131_072,
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BudgetQuoteProblem {
    InvalidDecisions,
    Mismatch,
}

impl std::fmt::Display for BudgetQuoteProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidDecisions => "capacities must name 1 to 16 distinct decisions",
            Self::Mismatch => "answer differs from the exact capacity quote",
        })
    }
}

/// Recompute with [`quote`] from the selection answers' capacities and compare
/// the whole answer exactly, including order and output-limit presence.
pub fn check_budget_quote(
    seats: &[BudgetSeat],
    prompt_tokens: Option<u64>,
    answer: &BudgetQuote,
) -> Result<(), BudgetQuoteProblem> {
    let expected = quote(seats, prompt_tokens).map_err(|_| BudgetQuoteProblem::InvalidDecisions)?;
    if answer != &expected {
        return Err(BudgetQuoteProblem::Mismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(window: u64, output: u64) -> BudgetSeat {
        BudgetSeat::new("d", "p/m", window, output)
    }

    fn one(window: u64, output: u64, prompt: Option<u64>) -> BudgetQuote {
        quote(&[seat(window, output)], prompt).unwrap()
    }

    #[test]
    fn budget_quote_known_capacities_and_request_order() {
        let seats = [
            seat(200_000, 20_000),
            BudgetSeat::new("e", "q/n", 100_000, 10_000),
        ];
        let answer = quote(&seats, Some(90_000)).unwrap();
        let mut first = BudgetSeatQuote::new("d", "p/m", 82_000);
        first.output_limit = Some(OutputLimit::send(20_000, 20_000, false));
        let mut second = BudgetSeatQuote::new("e", "q/n", 37_000);
        second.output_limit = Some(OutputLimit::send(5_904, 10_000, false));
        let expected = BudgetQuote::new(
            PackageBudget::new(37_000, LimitingSeat::new("e", "q/n"), 100_000, true),
            vec![first, second],
        );
        assert_eq!(answer, expected);
        assert_eq!(check_budget_quote(&seats, Some(90_000), &expected), Ok(()));
    }

    #[test]
    fn budget_quote_unknown_window_uses_fallback_but_stays_unknown() {
        let answer = one(0, 10_000, Some(0));
        assert_eq!(answer.package.tokens, 51_000);
        assert_eq!(answer.package.context_window, 128_000);
        assert!(!answer.package.context_window_known);
        assert_eq!(
            answer.seats[0].output_limit,
            Some(OutputLimit::unknown(OutputLimitReason::NoContextWindow))
        );
    }

    #[test]
    fn budget_quote_zero_capacities_use_reserves_and_reason_precedence() {
        let answer = one(0, 0, Some(u64::MAX));
        assert_eq!(answer.package.tokens, 40_000);
        assert_eq!(answer.package.context_window, 128_000);
        assert!(!answer.package.context_window_known);
        assert_eq!(
            answer.seats[0].output_limit,
            Some(OutputLimit::unknown(OutputLimitReason::NoOutputLimit))
        );
        let answer = one(100_000, 0, Some(100_000));
        assert_eq!(answer.package.tokens, 26_000);
        assert_eq!(
            answer.seats[0].output_limit,
            Some(OutputLimit::unknown(OutputLimitReason::NoOutputLimit))
        );
    }

    #[test]
    fn budget_quote_reason_precedence_is_first_matching() {
        for (window, output, prompt, reason) in [
            (0, 0, u64::MAX, OutputLimitReason::NoOutputLimit),
            (0, 128_000, u64::MAX, OutputLimitReason::NoContextWindow),
            (8_000, 8_000, 8_000, OutputLimitReason::Placeholder),
            (8_000, 8_001, 8_000, OutputLimitReason::Placeholder),
            (8_000, 1_000, 8_000, OutputLimitReason::NoRoom),
        ] {
            assert_eq!(
                one(window, output, Some(prompt)).seats[0].output_limit,
                Some(OutputLimit::unknown(reason))
            );
        }
    }

    #[test]
    fn budget_quote_room_zero_and_one() {
        assert_eq!(
            one(10_000, 1_000, Some(5_904)).seats[0].output_limit,
            Some(OutputLimit::unknown(OutputLimitReason::NoRoom))
        );
        assert_eq!(
            one(10_000, 1_000, Some(5_903)).seats[0].output_limit,
            Some(OutputLimit::send(1, 1_000, false))
        );
        assert_eq!(
            one(4_096, 1_000, Some(0)).seats[0].output_limit,
            Some(OutputLimit::unknown(OutputLimitReason::NoRoom))
        );
        assert_eq!(
            one(4_097, 1_000, Some(0)).seats[0].output_limit,
            Some(OutputLimit::send(1, 1_000, false))
        );
        assert_eq!(
            one(10_000, 1_000, Some(u64::MAX)).seats[0].output_limit,
            Some(OutputLimit::unknown(OutputLimitReason::NoRoom))
        );
    }

    #[test]
    fn budget_quote_suspect_threshold_is_strict_and_still_sends() {
        assert_eq!(
            one(500_000, 131_072, Some(0)).seats[0].output_limit,
            Some(OutputLimit::send(131_072, 131_072, false))
        );
        assert_eq!(
            one(500_000, 131_073, Some(0)).seats[0].output_limit,
            Some(OutputLimit::send(131_073, 131_073, true))
        );
        assert_eq!(
            one(500_000, 131_073, Some(495_903)).seats[0].output_limit,
            Some(OutputLimit::send(1, 131_073, true))
        );
    }

    #[test]
    fn budget_quote_package_reserve_cap_floor_and_saturation() {
        for (window, output, expected) in [
            (100_000, 63_999, 10_000),
            (100_000, 64_000, 10_000),
            (100_000, 64_001, 10_000),
            (100_001, 64_000, 10_000),
            (100_002, 64_000, 10_001),
            (80_000, 64_000, 0),
            (79_999, 64_000, 0),
            (1, u64::MAX, 0),
            (16_001, 1, 0),
            (16_003, 1, 1),
        ] {
            assert_eq!(
                one(window, output, None).package.tokens,
                expected,
                "window {window}, output {output}"
            );
        }
        assert_eq!(
            one(u64::MAX, u64::MAX, None).package.tokens,
            9_223_372_036_854_735_807
        );
    }

    #[test]
    fn budget_quote_absent_prompt_omits_output_limits_and_does_not_change_package() {
        let without = one(200_000, 20_000, None);
        assert!(without.seats[0].output_limit.is_none());
        for prompt in [0, 1, u64::MAX] {
            let with = one(200_000, 20_000, Some(prompt));
            assert_eq!(without.package, with.package);
            assert!(with.seats[0].output_limit.is_some());
        }
        let wire = serde_json::to_string(&without).unwrap();
        assert!(!wire.contains("output_limit"));
        assert_eq!(
            serde_json::to_string(&BudgetQuoteRequest::new(vec!["d".into()])).unwrap(),
            r#"{"decisions":["d"]}"#
        );
    }

    #[test]
    fn budget_quote_limiting_tie_uses_bytewise_route_then_request_order() {
        let seats = [
            BudgetSeat::new("z-first", "z/m", 100_000, 64_000),
            BudgetSeat::new("z-id", "a/m", 80_000, 44_000),
            BudgetSeat::new("a-id", "a/m", 100_000, 64_000),
        ];
        let answer = quote(&seats, None).unwrap();
        assert_eq!(
            answer.package,
            PackageBudget::new(10_000, LimitingSeat::new("z-id", "a/m"), 80_000, true)
        );
        let tied = [
            BudgetSeat::new("known", "p/m", 128_000, 32_000),
            BudgetSeat::new("unknown", "p/m", 0, 0),
        ];
        assert_eq!(
            quote(&tied, None).unwrap().package.limiting.decision_id,
            "known"
        );
        assert!(quote(&tied, None).unwrap().package.context_window_known);
        assert_eq!(
            quote(&[tied[1].clone(), tied[0].clone()], None)
                .unwrap()
                .package
                .limiting
                .decision_id,
            "unknown"
        );
        assert!(
            !quote(&[tied[1].clone(), tied[0].clone()], None)
                .unwrap()
                .package
                .context_window_known
        );
    }

    #[test]
    fn budget_quote_rejects_empty_decisions() {
        assert_eq!(
            quote(&[], None),
            Err(Refusal::invalid_request(
                "decisions",
                "decisions must contain 1 to 16 distinct decision ids"
            ))
        );
    }

    #[test]
    fn budget_quote_rejects_duplicate_decisions() {
        let duplicate = [seat(100_000, 10_000), seat(200_000, 20_000)];
        assert_eq!(
            quote(&duplicate, None),
            Err(Refusal::invalid_request(
                "decisions",
                "decisions must contain 1 to 16 distinct decision ids"
            ))
        );
    }

    #[test]
    fn budget_quote_rejects_over_sixteen_decisions() {
        let mut seats: Vec<_> = (0..16)
            .map(|n| BudgetSeat::new(n.to_string(), "p/m", 100_000, 10_000))
            .collect();
        assert_eq!(quote(&seats, None).unwrap().seats.len(), 16);
        seats.push(BudgetSeat::new("16", "p/m", 100_000, 10_000));
        assert_eq!(
            quote(&seats, None),
            Err(Refusal::invalid_request(
                "decisions",
                "decisions must contain 1 to 16 distinct decision ids"
            ))
        );
    }

    #[test]
    fn budget_quote_checker_compares_every_member_and_order() {
        let seats = [
            seat(200_000, 20_000),
            BudgetSeat::new("e", "q/n", 100_000, 10_000),
        ];
        let good = quote(&seats, Some(90_000)).unwrap();
        let mut changes = vec![];
        let mut changed = good.clone();
        changed.package.tokens += 1;
        changes.push(changed);
        let mut changed = good.clone();
        changed.package.context_window += 1;
        changes.push(changed);
        let mut changed = good.clone();
        changed.package.context_window_known = false;
        changes.push(changed);
        let mut changed = good.clone();
        changed.package.limiting.decision_id = "other".into();
        changes.push(changed);
        let mut changed = good.clone();
        changed.package.limiting.route_key = "other/m".into();
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats.reverse();
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats.pop();
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].decision_id = "other".into();
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].route_key = "other/m".into();
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].package_tokens += 1;
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].output_limit = None;
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].output_limit.as_mut().unwrap().tokens = Some(1);
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].output_limit.as_mut().unwrap().catalog = Some(1);
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].output_limit.as_mut().unwrap().suspect = None;
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].output_limit.as_mut().unwrap().reason = Some(OutputLimitReason::NoRoom);
        changes.push(changed);
        let mut changed = good.clone();
        changed.seats[0].output_limit.as_mut().unwrap().status =
            OutputLimitStatus::Other("future".into());
        changes.push(changed);
        for answer in changes {
            assert_eq!(
                check_budget_quote(&seats, Some(90_000), &answer),
                Err(BudgetQuoteProblem::Mismatch)
            );
        }
        assert_eq!(
            check_budget_quote(&[], None, &good),
            Err(BudgetQuoteProblem::InvalidDecisions)
        );
        assert_eq!(
            check_budget_quote(&seats, None, &good),
            Err(BudgetQuoteProblem::Mismatch)
        );
    }

    #[test]
    fn budget_quote_parser_and_helper_share_decision_list_refusals() {
        use serde_json::json;
        for decisions in [
            vec![],
            vec!["d".to_owned(), "d".to_owned()],
            (0..17).map(|n| n.to_string()).collect(),
        ] {
            let seats: Vec<_> = decisions
                .iter()
                .map(|id| BudgetSeat::new(id, "p/m", 100_000, 10_000))
                .collect();
            assert_eq!(
                parse_budget_quote_request(&json!({"decisions": decisions})).unwrap_err(),
                quote(&seats, None).unwrap_err()
            );
        }
        for body in [
            json!({}),
            json!({"decisions": null}),
            json!({"decisions": "d"}),
            json!({"decisions": [1]}),
        ] {
            assert_eq!(
                parse_budget_quote_request(&body).unwrap_err().detail["field"],
                "decisions"
            );
        }
        let decisions: Vec<_> = (0..16).map(|n| n.to_string()).collect();
        assert_eq!(
            parse_budget_quote_request(&json!({"decisions": decisions}))
                .unwrap()
                .decisions
                .len(),
            16
        );
    }

    #[test]
    fn budget_quote_parser_requires_unsigned_prompt_or_absence() {
        use serde_json::json;
        for prompt in [json!(-1), json!(1.5), json!("1"), json!(true)] {
            assert_eq!(
                parse_budget_quote_request(&json!({"decisions": ["d"], "prompt_tokens": prompt}))
                    .unwrap_err()
                    .detail["field"],
                "prompt_tokens"
            );
        }
        for prompt in [0, 1, u64::MAX] {
            assert_eq!(
                parse_budget_quote_request(&json!({"decisions": ["d"], "prompt_tokens": prompt}))
                    .unwrap(),
                BudgetQuoteRequest::new(vec!["d".into()]).with_prompt_tokens(prompt)
            );
        }
        assert_eq!(
            parse_budget_quote_request(&json!({"decisions": ["d"], "prompt_tokens": null}))
                .unwrap(),
            BudgetQuoteRequest::new(vec!["d".into()])
        );
    }

    #[test]
    fn budget_quote_copies_recorded_selection_capacities() {
        use crate::select::{RouteRef, RunnerRef};
        let mut route = RouteRef::new("p", "m");
        route.variant = Some("variant".into());
        let selected = SelectAnswer::new(
            "d",
            route.clone(),
            RunnerRef::new("runner", route),
            200_000,
            20_000,
            0,
            "Selected.",
        );
        let capacity = BudgetSeat::from_select_answer(&selected);
        assert_eq!(capacity, seat(200_000, 20_000));
        let answer = quote(&[capacity], None).unwrap();
        let wire = serde_json::to_string(&answer).unwrap();
        assert_eq!(serde_json::from_str::<BudgetQuote>(&wire).unwrap(), answer);
    }

    #[test]
    fn budget_quote_output_open_sets_round_trip_unknown_strings() {
        for wire in [
            r#"{"status":"future","reason":"new_reason"}"#,
            r#"{"status":"unknown","reason":"new_reason"}"#,
        ] {
            let limit: OutputLimit = serde_json::from_str(wire).unwrap();
            assert_eq!(
                limit.reason,
                Some(OutputLimitReason::Other("new_reason".into()))
            );
            assert_eq!(serde_json::to_string(&limit).unwrap(), wire);
        }
        assert_eq!(
            serde_json::to_string(&OutputLimit::send(1, 2, false)).unwrap(),
            r#"{"status":"send","tokens":1,"catalog":2,"suspect":false}"#
        );
        assert_eq!(
            serde_json::to_string(&OutputLimit::unknown(OutputLimitReason::NoRoom)).unwrap(),
            r#"{"status":"unknown","reason":"no_room"}"#
        );
    }
}
