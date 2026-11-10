//! Panel selection, replacement and caller-side validation for `select_panel`.
//! Ownership, retention, current availability and model modalities require router
//! state or catalog facts and cannot be verified from a panel answer alone.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    budget::{self, BudgetSeat},
    errors::Refusal,
    members::Members,
    select::{self, Modality, Requirements, SelectAnswer, SelectRequest, SelectSettings},
    service_tier::ServiceTier,
};

open_set! {
    /// Router policy class. Unknown classes are retained; only the router knows
    /// whether it has a policy for a class and refuses `class` when it does not.
    pub enum PanelClass {
        Audit => "audit",
        DesignReview => "design_review",
        Evaluate => "evaluate",
        Diagnose => "diagnose",
        Explain => "explain",
        Spec => "spec",
        Lab => "lab",
    }
}

open_set! {
    /// A slot's policy phase.
    pub enum PanelPhase {
        Top => "top",
        Extra => "extra",
    }
}

open_set! {
    /// Why fewer seats than the policy size were answered.
    pub enum ShortfallCode {
        PackageUnmeetable => "package_unmeetable",
        FloorUnmeetable => "floor_unmeetable",
        DiversityUnmeetable => "diversity_unmeetable",
        SizeReduced => "size_reduced",
    }
}

open_set! {
    /// Why a replacement has no seat. A cause leaves caller-held seats intact.
    pub enum ReplacementCause {
        NoCandidate => "no_candidate",
        PackageUnmeetable => "package_unmeetable",
    }
}

/// Panel demands; quality and ranking belong to the class policy, not the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<Modality>>,
}

impl PanelRequirements {
    pub fn new() -> Self {
        Self::default()
    }
}

/// New-panel request. Send only to a router declaring `select_panel`; the
/// shared select members also require their respective select groups.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectPanelRequest {
    pub class: PanelClass,
    pub consumer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements: Option<PanelRequirements>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runners: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_routes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::service_tier::deserialize_request"
    )]
    pub service_tier: Option<ServiceTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<SelectSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperatures: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
}

impl SelectPanelRequest {
    pub fn new(class: PanelClass, consumer: impl Into<String>) -> Self {
        Self {
            class,
            consumer: consumer.into(),
            requirements: None,
            runners: None,
            available_routes: None,
            exclude: None,
            harness: None,
            service_tier: None,
            correlation_id: None,
            task_ref: None,
            settings: None,
            temperatures: None,
            package_tokens: None,
            seed: None,
        }
    }

    /// Derive the select request used to check a seat. The inert `iq(0)` lets
    /// select's answer checks be shared without inventing a panel quality demand.
    pub fn seat_request(&self, extra_exclude: &[String]) -> SelectRequest {
        let mut requirements = Requirements::iq(0);
        if let Some(panel) = &self.requirements {
            requirements.context_window = panel.context_window;
            requirements.modalities = panel.modalities.clone();
        }
        let mut request = SelectRequest::new(&self.consumer, requirements);
        request.runners = self.runners.clone();
        request.available_routes = self.available_routes.clone();
        request.exclude = self.exclude.clone();
        if !extra_exclude.is_empty() {
            request
                .exclude
                .get_or_insert_with(Vec::new)
                .extend_from_slice(extra_exclude);
        }
        request.service_tier = self.service_tier;
        request.harness = self.harness.clone();
        request.settings = self.settings.clone();
        request.correlation_id = self.correlation_id.clone();
        request.task_ref = self.task_ref.clone();
        request
    }

    fn temperature(&self, slot: u64) -> Option<f64> {
        usize::try_from(slot)
            .ok()
            .and_then(|slot| self.temperatures.as_ref()?.get(slot).copied())
    }
}

/// Counts describe the policy, not the number of seats successfully selected.
/// Informative JSON members are never used to validate or choose seats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelPolicy {
    pub size: u64,
    pub top: u64,
    pub extra: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_ranking: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<Value>,
}

impl PanelPolicy {
    pub fn new(size: u64, top: u64, extra: u64) -> Self {
        Self {
            size,
            top,
            extra,
            ranking: None,
            extra_ranking: None,
            floor: None,
            seed: None,
        }
    }

    fn phase(&self, slot: u64) -> PanelPhase {
        if slot < self.top {
            PanelPhase::Top
        } else {
            PanelPhase::Extra
        }
    }
}

/// One occupied slot, with a full independently reportable selection decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelSeat {
    pub slot: u64,
    pub phase: PanelPhase,
    pub answer: SelectAnswer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

impl PanelSeat {
    pub fn new(slot: u64, phase: PanelPhase, answer: SelectAnswer) -> Self {
        Self {
            slot,
            phase,
            answer,
            family: None,
            temperature: None,
        }
    }
}

/// Diagnostic evidence for an incomplete panel or an unseated replacement.
/// Unknown fields are ignored so providers can extend their evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelShortfallDetail {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_tokens: Option<u64>,
    pub causes: Vec<PanelRejectedCandidate>,
}

impl PanelShortfallDetail {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            package_tokens: None,
            causes: Vec::new(),
        }
    }
}

/// One rejected route or model pin, with an open reason string and optional
/// evidence. Availability and final admission retain their refusal code and
/// payload rather than being relabeled as a diversity failure.
/// Known reasons are documented in CONTRACT.md §11.1; unknown reasons and
/// unknown fields remain readable. Quota and backoff causes may name a reset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelRejectedCandidate {
    pub model: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<u64>,
}

impl PanelRejectedCandidate {
    pub fn new(model: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            reason: reason.into(),
            detail: None,
            reset_at_ms: None,
        }
    }

    /// Attach package-fit evidence to a package rejection.
    pub fn with_typed_detail(mut self, detail: PanelPackageFit) -> Self {
        self.detail = Some(serde_json::to_value(detail).expect("package fit is JSON"));
        self
    }

    /// Decode package-fit evidence without discarding absent or malformed data.
    /// Older providers use prose detail for package causes; treat a decoding
    /// error as no typed fit, not as an invalid candidate. Other reasons may
    /// carry a different diagnostic shape, including a full refusal object.
    pub fn typed_detail(&self) -> Option<Result<PanelPackageFit, serde_json::Error>> {
        self.detail.clone().map(serde_json::from_value)
    }
}

/// Package-fit evidence for a rejected candidate. Unknown fields are ignored.
/// The package budget is
/// `context_window.saturating_sub(output_reserve_tokens + prompt_reserve_tokens) / 2`;
/// the other half is for panel discussion.
/// An unknown window uses 128,000 tokens, output reserves are
/// `min(catalog_max_output_tokens.unwrap_or(32_000), 64_000)`, and the prompt
/// reserve is 16,000 tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelPackageFit {
    pub package_tokens: u64,
    pub budget_tokens: u64,
    pub context_window: u64,
    pub context_window_known: bool,
    #[serde(default)]
    pub catalog_max_output_tokens: Option<u64>,
    pub output_reserve_tokens: u64,
    pub prompt_reserve_tokens: u64,
}

impl PanelPackageFit {
    pub fn new(
        package_tokens: u64,
        budget_tokens: u64,
        context_window: u64,
        context_window_known: bool,
        catalog_max_output_tokens: Option<u64>,
        output_reserve_tokens: u64,
        prompt_reserve_tokens: u64,
    ) -> Self {
        Self {
            package_tokens,
            budget_tokens,
            context_window,
            context_window_known,
            catalog_max_output_tokens,
            output_reserve_tokens,
            prompt_reserve_tokens,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelShortfall {
    pub code: ShortfallCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl PanelShortfall {
    pub fn new(code: ShortfallCode) -> Self {
        Self { code, detail: None }
    }

    pub fn with_typed_detail(mut self, detail: PanelShortfallDetail) -> Self {
        self.detail = Some(serde_json::to_value(detail).expect("shortfall detail is JSON"));
        self
    }

    pub fn typed_detail(&self) -> Option<Result<PanelShortfallDetail, serde_json::Error>> {
        self.detail.clone().map(serde_json::from_value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectPanelAnswer {
    pub panel_id: String,
    pub policy: PanelPolicy,
    pub seats: Vec<PanelSeat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortfall: Option<PanelShortfall>,
}

impl SelectPanelAnswer {
    pub fn new(panel_id: impl Into<String>, policy: PanelPolicy, seats: Vec<PanelSeat>) -> Self {
        Self {
            panel_id: panel_id.into(),
            policy,
            seats,
            shortfall: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelSlot {
    pub panel_id: String,
    pub slot: u64,
}

impl PanelSlot {
    pub fn new(panel_id: impl Into<String>, slot: u64) -> Self {
        Self {
            panel_id: panel_id.into(),
            slot,
        }
    }
}

/// Replacement form of `route.select_panel`. Only the panel's original caller
/// may replace it; foreign, unknown and no-longer-retained ids are all
/// `unknown_decision`. Expiry alone does not end retention.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelReplacementRequest {
    pub replace: PanelSlot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
}

impl PanelReplacementRequest {
    pub fn new(panel_id: impl Into<String>, slot: u64) -> Self {
        Self {
            replace: PanelSlot::new(panel_id, slot),
            exclude: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelReplacement {
    pub panel_id: String,
    pub slot: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat: Option<PanelSeat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<ReplacementCause>,
    /// Diagnostic evidence is allowed only with a cause, never with a seat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl PanelReplacement {
    pub fn seated(panel_id: impl Into<String>, seat: PanelSeat) -> Self {
        Self {
            panel_id: panel_id.into(),
            slot: seat.slot,
            seat: Some(seat),
            cause: None,
            detail: None,
        }
    }

    pub fn unseated(panel_id: impl Into<String>, slot: u64, cause: ReplacementCause) -> Self {
        Self {
            panel_id: panel_id.into(),
            slot,
            seat: None,
            cause: Some(cause),
            detail: None,
        }
    }
}

const SHARED_FIELDS: &[&str] = &[
    "consumer",
    "runners",
    "available_routes",
    "exclude",
    "harness",
    "service_tier",
    "correlation_id",
    "task_ref",
    "settings",
];

/// Decode the new-panel form with no optional select groups declared. The
/// caller must dispatch the replacement form to [`parse_panel_replacement_request`].
pub fn parse_select_panel_request(params: &Value) -> Result<SelectPanelRequest, Refusal> {
    parse_select_panel_request_serving::<&str>(params, &[])
}

/// Decode a new-panel request, refusing unsupported select members by name.
/// The router still resolves class policy and consumer ownership itself.
pub fn parse_select_panel_request_serving<C: AsRef<str>>(
    params: &Value,
    capabilities: &[C],
) -> Result<SelectPanelRequest, Refusal> {
    let members = Members::of(params)?;
    for field in members.object.keys() {
        if !SHARED_FIELDS.contains(&field.as_str())
            && !matches!(
                field.as_str(),
                "class" | "requirements" | "temperatures" | "package_tokens" | "seed"
            )
        {
            return Err(members.refuse(field, "not a new-panel request member"));
        }
    }
    let class = PanelClass::parse(&members.text("class", None)?);
    let requirements = parse_panel_requirements(members.object.get("requirements"))?;
    // Reuse select's validation and group checks only for shared members. Panel
    // requirements have a different unknown-member refusal and no quality demand.
    let mut select_params: serde_json::Map<String, Value> = members
        .object
        .iter()
        .filter(|(field, _)| SHARED_FIELDS.contains(&field.as_str()))
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect();
    select_params.insert("requirements".into(), serde_json::json!({"iq": 0}));
    let shared = select::parse_select_request_serving(&Value::Object(select_params), capabilities)?;
    let temperatures: Option<Vec<f64>> = members.optional("temperatures")?;
    if temperatures.as_ref().is_some_and(|values| {
        values.len() > 8
            || values
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=2.0).contains(value))
    }) {
        return Err(members.refuse("temperatures", "expected at most 8 finite numbers in 0..=2"));
    }
    let seed: Option<String> = members.optional("seed")?;
    if seed.as_ref().is_some_and(|seed| seed.len() > 256) {
        return Err(members.refuse("seed", "seed exceeds 256 bytes"));
    }
    Ok(SelectPanelRequest {
        class,
        consumer: shared.consumer,
        requirements,
        runners: shared.runners,
        available_routes: shared.available_routes,
        exclude: shared.exclude,
        harness: shared.harness,
        service_tier: shared.service_tier,
        correlation_id: shared.correlation_id,
        task_ref: shared.task_ref,
        settings: shared.settings,
        temperatures,
        package_tokens: members.optional("package_tokens")?,
        seed,
    })
}

fn parse_panel_requirements(value: Option<&Value>) -> Result<Option<PanelRequirements>, Refusal> {
    let object = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(object)) => object,
        _ => {
            return Err(Refusal::invalid_requirements(
                "requirements",
                "requirements must be an object",
            ))
        }
    };
    for field in object.keys() {
        if !matches!(field.as_str(), "context_window" | "modalities") {
            return Err(Refusal::invalid_request(
                format!("requirements.{field}"),
                "unknown panel requirement",
            ));
        }
    }
    let members = Members {
        object,
        refuse: Refusal::invalid_requirements,
    };
    let context_window = members.optional("context_window")?;
    let modalities: Option<Vec<Modality>> = members.optional("modalities")?;
    if modalities
        .iter()
        .flatten()
        .any(|modality| !modality.is_known())
    {
        return Err(members.refuse("modalities", "undefined input modality"));
    }
    Ok(Some(PanelRequirements {
        context_window,
        modalities,
    }))
}

/// Decode the replacement form. Policy size and panel ownership require the
/// router's retained panel; a slot at or above that size is `replace.slot`.
pub fn parse_panel_replacement_request(params: &Value) -> Result<PanelReplacementRequest, Refusal> {
    let members = Members::of(params)?;
    for field in members.object.keys() {
        if !matches!(field.as_str(), "replace" | "exclude") {
            return Err(members.refuse(field, "not a panel replacement member"));
        }
    }
    let replace: Value = members.required("replace")?;
    let object = replace
        .as_object()
        .ok_or_else(|| members.refuse("replace", "replace must be an object"))?;
    let target = Members {
        object,
        refuse: |field, message| Refusal::invalid_request(format!("replace.{field}"), message),
    };
    for field in object.keys() {
        if !matches!(field.as_str(), "panel_id" | "slot") {
            return Err(target.refuse(field, "unknown replacement target member"));
        }
    }
    let exclude: Option<Vec<String>> = members.optional("exclude")?;
    if exclude
        .iter()
        .flatten()
        .any(|key| !select::is_plain_route_key(key))
    {
        return Err(members.refuse(
            "exclude",
            "expected route keys provider/model, without a variant",
        ));
    }
    Ok(PanelReplacementRequest {
        replace: PanelSlot::new(target.text("panel_id", None)?, target.required("slot")?),
        exclude,
    })
}

/// Caller-held history. Construct after [`check_panel_answer`] succeeds and
/// apply only replacements accepted by [`check_panel_replacement`]. History is
/// panel-wide: displacing a seat never makes its route or decision id reusable.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PanelState {
    request: SelectPanelRequest,
    panel_id: String,
    policy: PanelPolicy,
    seats: BTreeMap<u64, PanelSeat>,
    seated_route_keys: BTreeSet<String>,
    seated_decision_ids: BTreeSet<String>,
}

impl PanelState {
    pub fn new(request: SelectPanelRequest, answer: SelectPanelAnswer) -> Self {
        let mut state = Self {
            request,
            panel_id: answer.panel_id,
            policy: answer.policy,
            seats: BTreeMap::new(),
            seated_route_keys: BTreeSet::new(),
            seated_decision_ids: BTreeSet::new(),
        };
        for seat in answer.seats {
            state.record(seat);
        }
        state
    }

    pub fn request(&self) -> &SelectPanelRequest {
        &self.request
    }
    pub fn panel_id(&self) -> &str {
        &self.panel_id
    }
    pub fn policy(&self) -> &PanelPolicy {
        &self.policy
    }
    pub fn seats(&self) -> &BTreeMap<u64, PanelSeat> {
        &self.seats
    }
    pub fn seated_route_keys(&self) -> &BTreeSet<String> {
        &self.seated_route_keys
    }
    pub fn seated_decision_ids(&self) -> &BTreeSet<String> {
        &self.seated_decision_ids
    }

    /// Record an already-checked replacement. A cause changes nothing, even
    /// when the slot still holds an earlier seat.
    pub fn apply(&mut self, answer: &PanelReplacement) {
        if let Some(seat) = &answer.seat {
            self.record(seat.clone());
        }
    }

    fn record(&mut self, seat: PanelSeat) {
        self.seated_route_keys.insert(seat.answer.route.route_key());
        self.seated_decision_ids
            .insert(seat.answer.decision_id.clone());
        self.seats.insert(seat.slot, seat);
    }
}

/// A panel rule a caller can verify without router state or catalog facts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanelProblem {
    Policy,
    Slots,
    Phase(u64),
    Temperature(u64),
    DuplicateDecision(String),
    DuplicateRoute(String),
    DuplicateFamily(Option<String>),
    Shortfall,
    SeatAnswer {
        slot: u64,
        problem: select::AnswerProblem,
    },
    WindowFloor(u64),
    ProjectSettings,
    PackageCeiling(u64),
    PanelId,
    ReplacementSlot,
    SeatOrCause,
    ReusedRoute(String),
    ReusedDecision(String),
}

impl std::fmt::Display for PanelProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "panel rule violated: {self:?}")
    }
}

impl std::error::Error for PanelProblem {}

fn check_seat(
    request: &SelectPanelRequest,
    derived: &SelectRequest,
    policy: &PanelPolicy,
    seat: &PanelSeat,
) -> Result<(), PanelProblem> {
    if seat.phase != policy.phase(seat.slot) {
        return Err(PanelProblem::Phase(seat.slot));
    }
    if seat.temperature != request.temperature(seat.slot) {
        return Err(PanelProblem::Temperature(seat.slot));
    }
    select::check_answer(derived, &seat.answer).map_err(|problem| PanelProblem::SeatAnswer {
        slot: seat.slot,
        problem,
    })?;
    if let Some(floor) = request
        .requirements
        .as_ref()
        .and_then(|requirements| requirements.context_window)
    {
        if seat.answer.context_window == 0 || seat.answer.context_window < floor {
            return Err(PanelProblem::WindowFloor(seat.slot));
        }
    }
    if let Some(package) = request.package_tokens {
        let capacities = [BudgetSeat::from_select_answer(&seat.answer)];
        let quote = budget::quote(&capacities, None).expect("one distinct decision");
        if quote.package.tokens < package {
            return Err(PanelProblem::PackageCeiling(seat.slot));
        }
    }
    Ok(())
}

/// Check policy, seats, diversity, project-settings agreement and per-seat
/// constraints. Gaps and zero seats are valid with the corresponding shortfall.
pub fn check_panel_answer(
    request: &SelectPanelRequest,
    answer: &SelectPanelAnswer,
) -> Result<(), PanelProblem> {
    let policy = &answer.policy;
    if !(1..=8).contains(&policy.size) || policy.top.checked_add(policy.extra) != Some(policy.size)
    {
        return Err(PanelProblem::Policy);
    }
    let mut previous = None;
    let mut decisions = BTreeSet::new();
    let mut routes = BTreeSet::new();
    let mut families = BTreeSet::new();
    let derived = request.seat_request(&[]);
    for seat in &answer.seats {
        if seat.slot >= policy.size || previous.is_some_and(|slot| slot >= seat.slot) {
            return Err(PanelProblem::Slots);
        }
        previous = Some(seat.slot);
        check_seat(request, &derived, policy, seat)?;
        if !decisions.insert(&seat.answer.decision_id) {
            return Err(PanelProblem::DuplicateDecision(
                seat.answer.decision_id.clone(),
            ));
        }
        let key = seat.answer.route.route_key();
        if !routes.insert(key.clone()) {
            return Err(PanelProblem::DuplicateRoute(key));
        }
        if !families.insert(&seat.family) {
            return Err(PanelProblem::DuplicateFamily(seat.family.clone()));
        }
        if seat.answer.project_settings != answer.seats[0].answer.project_settings {
            return Err(PanelProblem::ProjectSettings);
        }
    }
    if answer.shortfall.is_some() != ((answer.seats.len() as u64) < policy.size) {
        return Err(PanelProblem::Shortfall);
    }
    Ok(())
}

/// Check a replacement against the original panel and its complete history.
/// Only standing seats in other slots constrain family diversity, but every
/// historically seated route and decision id remains excluded forever.
pub fn check_panel_replacement(
    state: &PanelState,
    replace: &PanelReplacementRequest,
    answer: &PanelReplacement,
) -> Result<(), PanelProblem> {
    if replace.replace.panel_id != state.panel_id || answer.panel_id != state.panel_id {
        return Err(PanelProblem::PanelId);
    }
    if replace.replace.slot >= state.policy.size || answer.slot != replace.replace.slot {
        return Err(PanelProblem::ReplacementSlot);
    }
    if answer.seat.is_some() == answer.cause.is_some()
        || (answer.detail.is_some() && answer.cause.is_none())
    {
        return Err(PanelProblem::SeatOrCause);
    }
    if let Some(seat) = &answer.seat {
        if seat.slot != answer.slot {
            return Err(PanelProblem::ReplacementSlot);
        }
        let derived = state
            .request
            .seat_request(replace.exclude.as_deref().unwrap_or_default());
        check_seat(&state.request, &derived, &state.policy, seat)?;
        let key = seat.answer.route.route_key();
        if state.seated_route_keys.contains(&key) {
            return Err(PanelProblem::ReusedRoute(key));
        }
        if state.seated_decision_ids.contains(&seat.answer.decision_id) {
            return Err(PanelProblem::ReusedDecision(
                seat.answer.decision_id.clone(),
            ));
        }
        if state
            .seats
            .values()
            .any(|standing| standing.slot != seat.slot && standing.family == seat.family)
        {
            return Err(PanelProblem::DuplicateFamily(seat.family.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::RefusalCode,
        select::{ProjectSettings, RouteRef, RunnerRef},
    };
    use serde_json::json;

    fn seat(slot: u64, model: &str) -> PanelSeat {
        let route = RouteRef::new("p", model);
        let answer = SelectAnswer::new(
            format!("decision-{model}"),
            route.clone(),
            RunnerRef::new("runner", route),
            128_000,
            32_000,
            1,
            "ranked",
        );
        let mut seat = PanelSeat::new(
            slot,
            if slot == 0 {
                PanelPhase::Top
            } else {
                PanelPhase::Extra
            },
            answer,
        );
        seat.family = Some(model.into());
        seat.temperature = if slot == 0 { Some(0.4) } else { None };
        seat
    }

    fn fixture() -> (SelectPanelRequest, SelectPanelAnswer) {
        let mut request = SelectPanelRequest::new(PanelClass::Audit, "consumer");
        request.runners = Some(vec!["runner".into()]);
        request.available_routes = Some(["p/a", "p/b", "p/c", "p/d"].map(String::from).to_vec());
        request.requirements = Some(PanelRequirements {
            context_window: Some(50_000),
            modalities: Some(vec![Modality::Image]),
        });
        request.temperatures = Some(vec![0.4]);
        request.package_tokens = Some(10_000);
        let answer = SelectPanelAnswer::new(
            "panel",
            PanelPolicy::new(2, 1, 1),
            vec![seat(0, "a"), seat(1, "b")],
        );
        (request, answer)
    }

    fn replacement() -> (PanelState, PanelReplacementRequest, PanelReplacement) {
        let (request, answer) = fixture();
        assert_eq!(check_panel_answer(&request, &answer), Ok(()));
        (
            PanelState::new(request, answer),
            PanelReplacementRequest::new("panel", 0),
            PanelReplacement::seated("panel", seat(0, "c")),
        )
    }

    fn refusal(params: Value, code: RefusalCode, field: &str) {
        let error = parse_select_panel_request(&params).unwrap_err();
        assert_eq!(error.code, code, "{params}");
        assert_eq!(error.detail["field"], field, "{params}");
    }

    #[test]
    fn unknown_panel_requirements_are_invalid_request_by_name() {
        for member in ["iq", "eq", "max_output_tokens", "tools", "future"] {
            let mut params = json!({"class":"audit", "consumer":"c", "requirements":{}});
            params["requirements"][member] = json!(1);
            refusal(
                params,
                RefusalCode::InvalidRequest,
                &format!("requirements.{member}"),
            );
        }
    }

    #[test]
    fn malformed_window_or_modality_is_invalid_requirements() {
        for value in [
            json!(-1),
            json!(1.5),
            json!("100"),
            json!(true),
            json!(18446744073709551616.0),
        ] {
            refusal(
                json!({"class":"audit", "consumer":"c", "requirements":{"context_window":value}}),
                RefusalCode::InvalidRequirements,
                "context_window",
            );
        }
        for value in [
            json!(["text"]),
            json!(["future"]),
            json!([1]),
            json!("image"),
        ] {
            refusal(
                json!({"class":"audit", "consumer":"c", "requirements":{"modalities":value}}),
                RefusalCode::InvalidRequirements,
                "modalities",
            );
        }
        refusal(
            json!({"class":"audit", "consumer":"c", "requirements":[]}),
            RefusalCode::InvalidRequirements,
            "requirements",
        );
    }

    #[test]
    fn parser_accepts_panel_boundaries_and_open_class() {
        let params = json!({"class":"future", "consumer":"c", "requirements":{
            "context_window":0, "modalities":["image", "pdf", "audio"]},
            "temperatures":[0, 2, 0, 2, 0, 2, 0, 2], "seed":"", "package_tokens":0});
        let parsed = parse_select_panel_request(&params).unwrap();
        assert_eq!(parsed.class, PanelClass::Other("future".into()));
        assert_eq!(parsed.requirements.unwrap().context_window, Some(0));
        let mut params = json!({"class":"lab", "consumer":"c", "seed":"é".repeat(128)});
        assert!(parse_select_panel_request(&params).is_ok());
        params["seed"] = json!("é".repeat(129));
        refusal(params, RefusalCode::InvalidRequest, "seed");
    }

    #[test]
    fn parser_rejects_malformed_panel_members() {
        for (field, value) in [
            ("class", json!("")),
            ("consumer", json!("")),
            ("temperatures", json!(vec![0; 9])),
            ("temperatures", json!([-0.01])),
            ("temperatures", json!([2.01])),
            ("temperatures", json!([null])),
            ("temperatures", json!("hot")),
            ("package_tokens", json!(-1)),
            ("package_tokens", json!(1.5)),
            ("seed", json!(1)),
        ] {
            let mut params = json!({"class":"audit", "consumer":"c"});
            params[field] = value;
            refusal(params, RefusalCode::InvalidRequest, field);
        }
    }

    #[test]
    fn panel_refuses_all_non_panel_select_members() {
        for field in [
            "pin",
            "pin_slot",
            "pin_fallback",
            "prefer",
            "prefer_prompt_cache",
            "affinity",
            "exclude_providers",
            "concurrency_slots",
            "detail",
            "hold_ok",
            "hold_wait_max_s",
            "unknown",
        ] {
            let mut params = json!({"class":"audit", "consumer":"c"});
            params[field] = json!(true);
            refusal(params, RefusalCode::InvalidRequest, field);
        }
    }

    #[test]
    fn panel_shared_members_use_select_groups_and_validation() {
        for (field, value, group) in [
            (
                "available_routes",
                json!(["p/a"]),
                "select_available_routes",
            ),
            ("harness", json!("cli"), "select_harness"),
            ("service_tier", json!("fast"), "service_tier"),
            ("task_ref", json!("task-1"), "select_task_ref"),
            (
                "settings",
                json!({"project_id":"project"}),
                "select_settings",
            ),
        ] {
            let mut params = json!({"class":"audit", "consumer":"c"});
            params[field] = value;
            refusal(params.clone(), RefusalCode::InvalidRequest, field);
            assert!(
                parse_select_panel_request_serving(&params, &[group]).is_ok(),
                "{field}"
            );
        }
        for (field, value) in [
            ("runners", json!([""])),
            ("exclude", json!(["not-a-route"])),
            ("correlation_id", json!("x".repeat(129))),
        ] {
            let mut params = json!({"class":"audit", "consumer":"c"});
            params[field] = value;
            refusal(params, RefusalCode::InvalidRequest, field);
        }
        let error = parse_select_panel_request_serving(
            &json!({"class":"audit", "consumer":"c",
            "available_routes":[]}),
            &["select_available_routes"],
        )
        .unwrap_err();
        assert_eq!(error.detail["field"], "available_routes");
        let error = parse_select_panel_request_serving(
            &json!({"class":"audit", "consumer":"c",
            "settings":{"folder":"x"}}),
            &["select_settings"],
        )
        .unwrap_err();
        assert_eq!(error.detail["field"], "settings.folder");
    }

    #[test]
    fn seat_request_carries_constraints_and_combines_exclusions() {
        let (mut request, _) = fixture();
        request.exclude = Some(vec!["p/old".into()]);
        request.harness = Some("cli".into());
        request.service_tier = Some(ServiceTier::Fast);
        request.settings = Some(SelectSettings::new().with_project_id("project"));
        request.correlation_id = Some("log".into());
        request.task_ref = Some("task".into());
        let derived = request.seat_request(&["p/new".into()]);
        assert_eq!(derived.consumer, request.consumer);
        assert_eq!(derived.requirements.iq, 0);
        assert_eq!(derived.requirements.context_window, Some(50_000));
        assert_eq!(derived.requirements.modalities, Some(vec![Modality::Image]));
        assert_eq!(derived.runners, request.runners);
        assert_eq!(derived.available_routes, request.available_routes);
        assert_eq!(derived.exclude, Some(vec!["p/old".into(), "p/new".into()]));
        assert_eq!(derived.harness, request.harness);
        assert_eq!(derived.service_tier, request.service_tier);
        assert_eq!(derived.settings, request.settings);
        assert_eq!(derived.correlation_id, request.correlation_id);
        assert_eq!(derived.task_ref, request.task_ref);
        assert_eq!(derived.pin, None);
        assert_eq!(derived.pin_slot, None);
        assert_eq!(derived.prefer, None);
        assert_eq!(derived.prefer_prompt_cache, None);
        assert_eq!(derived.affinity, None);
        assert_eq!(derived.requirements.max_output_tokens, None);
        let original = request.clone();
        request.seat_request(&["p/another".into()]);
        assert_eq!(request, original);
    }

    #[test]
    fn full_gapped_and_empty_panels_pass() {
        let (request, mut answer) = fixture();
        assert_eq!(check_panel_answer(&request, &answer), Ok(()));
        answer.policy = PanelPolicy::new(3, 1, 2);
        answer.seats[1].slot = 2;
        answer.shortfall = Some(PanelShortfall::new(ShortfallCode::DiversityUnmeetable));
        assert_eq!(check_panel_answer(&request, &answer), Ok(()));
        answer.seats.clear();
        assert_eq!(check_panel_answer(&request, &answer), Ok(()));
        for policy in [PanelPolicy::new(1, 1, 0), PanelPolicy::new(8, 0, 8)] {
            answer.policy = policy;
            assert_eq!(check_panel_answer(&request, &answer), Ok(()));
        }
    }

    #[test]
    fn panel_policy_bounds_and_sum_are_checked_without_overflow() {
        let (request, answer) = fixture();
        for policy in [
            PanelPolicy::new(0, 0, 0),
            PanelPolicy::new(9, 1, 8),
            PanelPolicy::new(2, 1, 0),
            PanelPolicy::new(2, u64::MAX, 3),
        ] {
            let mut answer = answer.clone();
            answer.policy = policy;
            assert_eq!(
                check_panel_answer(&request, &answer),
                Err(PanelProblem::Policy)
            );
        }
    }

    #[test]
    fn panel_slots_are_distinct_ascending_and_bounded() {
        let (request, answer) = fixture();
        let mut duplicate = answer.clone();
        duplicate.seats[1].slot = 0;
        assert_eq!(
            check_panel_answer(&request, &duplicate),
            Err(PanelProblem::Slots)
        );
        let mut reversed = answer.clone();
        reversed.seats.reverse();
        assert_eq!(
            check_panel_answer(&request, &reversed),
            Err(PanelProblem::Slots)
        );
        let mut out_of_bounds = answer;
        out_of_bounds.seats[1].slot = 2;
        assert_eq!(
            check_panel_answer(&request, &out_of_bounds),
            Err(PanelProblem::Slots)
        );
    }

    #[test]
    fn panel_phase_follows_slot_and_not_seated_order() {
        let (request, mut answer) = fixture();
        answer.seats[1].phase = PanelPhase::Top;
        assert_eq!(
            check_panel_answer(&request, &answer),
            Err(PanelProblem::Phase(1))
        );
        answer.seats[1].phase = PanelPhase::Other("future".into());
        assert_eq!(
            check_panel_answer(&request, &answer),
            Err(PanelProblem::Phase(1))
        );
    }

    #[test]
    fn panel_temperature_matches_slot_and_is_absent_past_list() {
        let (request, answer) = fixture();
        for temperature in [None, Some(0.5), Some(f64::NAN), Some(f64::INFINITY)] {
            let mut answer = answer.clone();
            answer.seats[0].temperature = temperature;
            assert_eq!(
                check_panel_answer(&request, &answer),
                Err(PanelProblem::Temperature(0))
            );
        }
        let mut answer = answer;
        answer.seats[1].temperature = Some(0.4);
        assert_eq!(
            check_panel_answer(&request, &answer),
            Err(PanelProblem::Temperature(1))
        );
    }

    #[test]
    fn panel_decisions_routes_and_families_are_distinct() {
        let (request, answer) = fixture();
        let mut repeated = answer.clone();
        repeated.seats[1].answer.decision_id = repeated.seats[0].answer.decision_id.clone();
        assert_eq!(
            check_panel_answer(&request, &repeated),
            Err(PanelProblem::DuplicateDecision("decision-a".into()))
        );
        let mut repeated = answer.clone();
        repeated.seats[1].answer.route = repeated.seats[0].answer.route.clone();
        repeated.seats[1].answer.route.variant = Some("different".into());
        assert_eq!(
            check_panel_answer(&request, &repeated),
            Err(PanelProblem::DuplicateRoute("p/a".into()))
        );
        let mut repeated = answer.clone();
        repeated.seats[1].family = repeated.seats[0].family.clone();
        assert_eq!(
            check_panel_answer(&request, &repeated),
            Err(PanelProblem::DuplicateFamily(Some("a".into())))
        );
        let mut repeated = answer;
        repeated.seats[0].family = None;
        assert_eq!(check_panel_answer(&request, &repeated), Ok(()));
        repeated.seats[1].family = None;
        assert_eq!(
            check_panel_answer(&request, &repeated),
            Err(PanelProblem::DuplicateFamily(None))
        );
    }

    #[test]
    fn shortfall_is_present_exactly_when_panel_is_incomplete() {
        let (request, mut answer) = fixture();
        answer.shortfall = Some(PanelShortfall::new(ShortfallCode::SizeReduced));
        assert_eq!(
            check_panel_answer(&request, &answer),
            Err(PanelProblem::Shortfall)
        );
        answer.shortfall = None;
        answer.seats.pop();
        assert_eq!(
            check_panel_answer(&request, &answer),
            Err(PanelProblem::Shortfall)
        );
    }

    #[test]
    fn panel_seats_share_project_settings_and_require_presence() {
        let (mut request, mut answer) = fixture();
        request.settings = Some(SelectSettings::new().with_project_id("project"));
        assert!(matches!(
            check_panel_answer(&request, &answer),
            Err(PanelProblem::SeatAnswer {
                slot: 0,
                problem: select::AnswerProblem::MissingProjectSettings
            })
        ));
        for seat in &mut answer.seats {
            seat.answer.project_settings = Some(ProjectSettings::Unresolved);
        }
        assert_eq!(check_panel_answer(&request, &answer), Ok(()));
        answer.seats[1].answer.project_settings = Some(ProjectSettings::None);
        assert_eq!(
            check_panel_answer(&request, &answer),
            Err(PanelProblem::ProjectSettings)
        );
    }

    #[test]
    fn both_panel_helpers_apply_select_answer_checks() {
        for change in [0, 1, 2, 3, 4] {
            let (request, mut panel) = fixture();
            let (state, replace, mut replacement) = replacement();
            for answer in [
                &mut panel.seats[0].answer,
                &mut replacement.seat.as_mut().unwrap().answer,
            ] {
                match change {
                    0 => answer.runner.module = "unlisted".into(),
                    1 => answer.route.model = "unavailable".into(),
                    2 => answer.explanation = "x".repeat(201),
                    3 => answer.pin_outcome = Some(select::PinOutcome::Pinned),
                    _ => answer.project_settings = Some(ProjectSettings::Applied),
                }
            }
            assert!(matches!(
                check_panel_answer(&request, &panel),
                Err(PanelProblem::SeatAnswer { slot: 0, .. })
            ));
            assert!(matches!(
                check_panel_replacement(&state, &replace, &replacement),
                Err(PanelProblem::SeatAnswer { slot: 0, .. })
            ));
        }
    }

    #[test]
    fn both_panel_helpers_refuse_zero_or_below_floor_windows() {
        for window in [0, 49_999, 50_000, 50_001] {
            let (mut request, mut panel) = fixture();
            let (mut state, replace, mut replacement) = replacement();
            request.package_tokens = None;
            state.request.package_tokens = None;
            panel.seats[0].answer.context_window = window;
            replacement.seat.as_mut().unwrap().answer.context_window = window;
            let expected = if window < 50_000 {
                Err(PanelProblem::WindowFloor(0))
            } else {
                Ok(())
            };
            assert_eq!(check_panel_answer(&request, &panel), expected);
            assert_eq!(
                check_panel_replacement(&state, &replace, &replacement),
                expected
            );
        }
        let (mut request, mut panel) = fixture();
        request.requirements.as_mut().unwrap().context_window = Some(0);
        panel.seats[0].answer.context_window = 0;
        assert_eq!(
            check_panel_answer(&request, &panel),
            Err(PanelProblem::WindowFloor(0))
        );
        request.requirements = None;
        assert_eq!(check_panel_answer(&request, &panel), Ok(()));
    }

    #[test]
    fn both_panel_helpers_use_budget_package_ceiling() {
        // Known capacities give (128000 - 32000 - 16000) / 2 = 40000.
        for package in [40_000, 40_001] {
            let (mut request, panel) = fixture();
            request.package_tokens = Some(package);
            let expected = if package > 40_000 {
                Err(PanelProblem::PackageCeiling(0))
            } else {
                Ok(())
            };
            assert_eq!(check_panel_answer(&request, &panel), expected);
            let (_, replace, replacement) = replacement();
            let state = PanelState::new(request, panel);
            assert_eq!(
                check_panel_replacement(&state, &replace, &replacement),
                expected
            );
        }
    }

    #[test]
    fn package_check_uses_unknown_capacity_fallbacks_and_optional_constraint() {
        let (mut request, mut panel) = fixture();
        request.requirements = None;
        for seat in &mut panel.seats {
            seat.answer.context_window = 0;
            seat.answer.max_output_tokens = 0;
        }
        request.package_tokens = Some(40_000);
        assert_eq!(check_panel_answer(&request, &panel), Ok(()));
        request.package_tokens = Some(40_001);
        assert_eq!(
            check_panel_answer(&request, &panel),
            Err(PanelProblem::PackageCeiling(0))
        );
        request.package_tokens = None;
        for seat in &mut panel.seats {
            seat.answer.context_window = 1;
        }
        assert_eq!(check_panel_answer(&request, &panel), Ok(()));
    }

    #[test]
    fn replacement_parser_accepts_only_target_and_exclude() {
        let params = json!({"replace":{"panel_id":"panel", "slot":0}, "exclude":["p/a", "p/a"]});
        let parsed = parse_panel_replacement_request(&params).unwrap();
        assert_eq!(parsed.replace, PanelSlot::new("panel", 0));
        assert_eq!(parsed.exclude, Some(vec!["p/a".into(), "p/a".into()]));
        for (field, value) in [
            ("class", json!("audit")),
            ("consumer", json!("c")),
            ("seed", json!("x")),
            ("requirements", json!({})),
            ("unknown", Value::Null),
            ("exclude", json!(["bad"])),
        ] {
            let mut params = params.clone();
            params[field] = value;
            let error = parse_panel_replacement_request(&params).unwrap_err();
            assert_eq!(error.code, RefusalCode::InvalidRequest);
            assert_eq!(error.detail["field"], field);
        }
        for (target, field) in [
            (json!(null), "replace"),
            (json!(1), "replace"),
            (json!({"slot":0}), "replace.panel_id"),
            (json!({"panel_id":"p", "slot":-1}), "replace.slot"),
            (json!({"panel_id":"p", "slot":1.5}), "replace.slot"),
            (
                json!({"panel_id":"p", "slot":0, "future":true}),
                "replace.future",
            ),
        ] {
            let error = parse_panel_replacement_request(&json!({"replace":target})).unwrap_err();
            assert_eq!(error.code, RefusalCode::InvalidRequest);
            assert_eq!(error.detail["field"], field);
        }
    }

    #[test]
    fn replacement_echoes_panel_and_slot_and_stays_in_policy() {
        let (state, replace, replacement) = replacement();
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Ok(())
        );
        let mut foreign_request = replace.clone();
        foreign_request.replace.panel_id = "other".into();
        assert_eq!(
            check_panel_replacement(&state, &foreign_request, &replacement),
            Err(PanelProblem::PanelId)
        );
        let mut foreign_answer = replacement.clone();
        foreign_answer.panel_id = "other".into();
        assert_eq!(
            check_panel_replacement(&state, &replace, &foreign_answer),
            Err(PanelProblem::PanelId)
        );
        let mut wrong_slot = replacement.clone();
        wrong_slot.slot = 1;
        assert_eq!(
            check_panel_replacement(&state, &replace, &wrong_slot),
            Err(PanelProblem::ReplacementSlot)
        );
        let mut wrong_slot = replacement;
        wrong_slot.seat.as_mut().unwrap().slot = 1;
        assert_eq!(
            check_panel_replacement(&state, &replace, &wrong_slot),
            Err(PanelProblem::ReplacementSlot)
        );
        for slot in [2, u64::MAX] {
            let replace = PanelReplacementRequest::new("panel", slot);
            let answer = PanelReplacement::unseated("panel", slot, ReplacementCause::NoCandidate);
            assert_eq!(
                check_panel_replacement(&state, &replace, &answer),
                Err(PanelProblem::ReplacementSlot)
            );
        }
    }

    #[test]
    fn replacement_has_exactly_one_seat_or_cause() {
        let (state, replace, mut replacement) = replacement();
        replacement.cause = Some(ReplacementCause::NoCandidate);
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Err(PanelProblem::SeatOrCause)
        );
        replacement.seat = None;
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Ok(())
        );
        replacement.cause = Some(ReplacementCause::Other("future".into()));
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Ok(())
        );
        replacement.cause = None;
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Err(PanelProblem::SeatOrCause)
        );
    }

    #[test]
    fn replacement_rejects_detail_with_seat() {
        let (state, replace, mut replacement) = replacement();
        replacement.detail = Some(json!({"message":"No spare route", "causes":[]}));
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Err(PanelProblem::SeatOrCause)
        );
        replacement.seat = None;
        replacement.cause = Some(ReplacementCause::NoCandidate);
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Ok(())
        );
        replacement.cause = None;
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Err(PanelProblem::SeatOrCause)
        );
    }

    #[test]
    fn panel_typed_evidence_helpers_preserve_json_and_report_decode_errors() {
        let fit = PanelPackageFit::new(46_960, 40_000, 128_000, false, None, 32_000, 16_000);
        let package = PanelRejectedCandidate::new("p/small", "package_exceeds_ceiling")
            .with_typed_detail(fit.clone());
        assert_eq!(package.typed_detail().unwrap().unwrap(), fit);
        assert_eq!(json!(fit)["catalog_max_output_tokens"], Value::Null);
        let mut detail = PanelShortfallDetail::new("One candidate did not fit");
        detail.package_tokens = Some(46_960);
        detail.causes.push(package);
        let shortfall =
            PanelShortfall::new(ShortfallCode::PackageUnmeetable).with_typed_detail(detail.clone());
        assert_eq!(shortfall.typed_detail().unwrap().unwrap(), detail);
        assert_eq!(shortfall.detail, Some(json!(detail)));
        assert!(PanelShortfall::new(ShortfallCode::SizeReduced)
            .typed_detail()
            .is_none());
        assert!(PanelRejectedCandidate::new("p/model", "future")
            .typed_detail()
            .is_none());
        let mut malformed = shortfall;
        malformed.detail = Some(json!({"message":5}));
        assert!(malformed.typed_detail().unwrap().is_err());
        let mut legacy = PanelRejectedCandidate::new("p/small", "package_too_large");
        legacy.detail = Some(json!("package exceeds ceiling"));
        assert!(legacy.typed_detail().unwrap().is_err());
        legacy.detail = Some(json!({"budget_tokens": "not unsigned"}));
        assert!(legacy.typed_detail().unwrap().is_err());
    }

    #[test]
    fn panel_evidence_and_replacement_readers_ignore_unknown_fields() {
        let value = json!({
            "message":"No candidate", "package_tokens":null, "future":true,
            "causes":[{"model":"p/model", "reason":"future_reason", "future":true,
                "reset_at_ms":1234, "detail":{
                    "package_tokens":46_960, "budget_tokens":40_000,
                    "context_window":128_000, "context_window_known":false,
                    "catalog_max_output_tokens":null, "output_reserve_tokens":32_000,
                    "prompt_reserve_tokens":16_000, "future":true}}]
        });
        let detail: PanelShortfallDetail = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(detail.causes[0].reason, "future_reason");
        assert_eq!(detail.causes[0].reset_at_ms, Some(1234));
        assert_eq!(
            detail.causes[0]
                .typed_detail()
                .unwrap()
                .unwrap()
                .budget_tokens,
            40_000
        );
        let replacement: PanelReplacement = serde_json::from_value(json!({
            "panel_id":"p", "slot":0, "cause":"no_candidate", "detail":value,
            "future":true
        }))
        .unwrap();
        assert!(replacement.detail.is_some());
        let legacy: PanelReplacement = serde_json::from_value(json!({
            "panel_id":"p", "slot":0, "cause":"no_candidate"
        }))
        .unwrap();
        assert!(legacy.detail.is_none());
        assert!(json!(legacy).get("detail").is_none());
    }

    #[test]
    fn replacement_preserves_phase_and_temperature() {
        let (state, replace, mut replacement) = replacement();
        replacement.seat.as_mut().unwrap().phase = PanelPhase::Extra;
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Err(PanelProblem::Phase(0))
        );
        replacement.seat.as_mut().unwrap().phase = PanelPhase::Top;
        replacement.seat.as_mut().unwrap().temperature = None;
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Err(PanelProblem::Temperature(0))
        );
        replacement.slot = 1;
        replacement.seat.as_mut().unwrap().slot = 1;
        replacement.seat.as_mut().unwrap().phase = PanelPhase::Extra;
        replacement.seat.as_mut().unwrap().temperature = Some(0.4);
        let replace = PanelReplacementRequest::new("panel", 1);
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Err(PanelProblem::Temperature(1))
        );
        replacement.seat.as_mut().unwrap().temperature = None;
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Ok(())
        );
    }

    #[test]
    fn replacement_honors_original_and_additional_excludes() {
        let (mut request, panel) = fixture();
        request.exclude = Some(vec!["p/c".into()]);
        let state = PanelState::new(request, panel);
        let replace = PanelReplacementRequest::new("panel", 0);
        let answer = PanelReplacement::seated("panel", seat(0, "c"));
        assert_eq!(
            check_panel_replacement(&state, &replace, &answer),
            Err(PanelProblem::SeatAnswer {
                slot: 0,
                problem: select::AnswerProblem::Excluded("p/c".into())
            })
        );
        let (state, mut replace, answer) = replacement();
        replace.exclude = Some(vec!["p/c".into()]);
        assert_eq!(
            check_panel_replacement(&state, &replace, &answer),
            Err(PanelProblem::SeatAnswer {
                slot: 0,
                problem: select::AnswerProblem::Excluded("p/c".into())
            })
        );
    }

    #[test]
    fn replacement_refuses_a_to_b_to_a_route_cycle() {
        let (mut state, replace, first) = replacement();
        assert_eq!(check_panel_replacement(&state, &replace, &first), Ok(()));
        state.apply(&first);
        let mut back = PanelReplacement::seated("panel", seat(0, "a"));
        // A fresh decision cannot make the displaced route eligible again.
        back.seat.as_mut().unwrap().answer.decision_id = "fresh-decision".into();
        assert_eq!(
            check_panel_replacement(&state, &replace, &back),
            Err(PanelProblem::ReusedRoute("p/a".into()))
        );
        back.seat.as_mut().unwrap().answer.route.variant = Some("new-variant".into());
        assert_eq!(
            check_panel_replacement(&state, &replace, &back),
            Err(PanelProblem::ReusedRoute("p/a".into()))
        );
    }

    #[test]
    fn replacement_refuses_reused_displaced_decision_on_new_route() {
        let (mut state, replace, first) = replacement();
        assert_eq!(check_panel_replacement(&state, &replace, &first), Ok(()));
        state.apply(&first);
        let mut next = PanelReplacement::seated("panel", seat(0, "d"));
        next.seat.as_mut().unwrap().answer.decision_id = "decision-a".into();
        assert_eq!(
            check_panel_replacement(&state, &replace, &next),
            Err(PanelProblem::ReusedDecision("decision-a".into()))
        );
        next.seat.as_mut().unwrap().answer.decision_id = "decision-b".into();
        assert_eq!(
            check_panel_replacement(&state, &replace, &next),
            Err(PanelProblem::ReusedDecision("decision-b".into()))
        );
    }

    #[test]
    fn replacement_route_history_is_panel_wide() {
        let (mut state, replace, first) = replacement();
        state.apply(&first);
        let replace = PanelReplacementRequest::new(&replace.replace.panel_id, 1);
        let answer = PanelReplacement::seated("panel", seat(1, "a"));
        assert_eq!(
            check_panel_replacement(&state, &replace, &answer),
            Err(PanelProblem::ReusedRoute("p/a".into()))
        );
    }

    #[test]
    fn replacement_families_constrain_only_other_standing_slots() {
        let (mut state, replace, mut replacement) = replacement();
        replacement.seat.as_mut().unwrap().family = Some("b".into());
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Err(PanelProblem::DuplicateFamily(Some("b".into())))
        );
        replacement.seat.as_mut().unwrap().family = Some("a".into());
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Ok(())
        );
        state.apply(&replacement);
        let mut next = PanelReplacement::seated("panel", seat(1, "d"));
        next.seat.as_mut().unwrap().family = None;
        let replace = PanelReplacementRequest::new("panel", 1);
        assert_eq!(check_panel_replacement(&state, &replace, &next), Ok(()));

        let (request, mut panel) = fixture();
        panel.seats[1].family = None;
        let state = PanelState::new(request, panel);
        let mut next = PanelReplacement::seated("panel", seat(0, "c"));
        next.seat.as_mut().unwrap().family = None;
        assert_eq!(
            check_panel_replacement(&state, &PanelReplacementRequest::new("panel", 0), &next),
            Err(PanelProblem::DuplicateFamily(None))
        );
    }

    #[test]
    fn state_records_all_history_and_cause_leaves_it_unchanged() {
        let (mut state, replace, replacement) = replacement();
        assert_eq!(state.request().consumer, "consumer");
        assert_eq!(state.panel_id(), "panel");
        assert_eq!(state.policy(), &PanelPolicy::new(2, 1, 1));
        let snapshot = state.clone();
        let cause = PanelReplacement::unseated("panel", 0, ReplacementCause::PackageUnmeetable);
        assert_eq!(check_panel_replacement(&state, &replace, &cause), Ok(()));
        state.apply(&cause);
        assert_eq!(state, snapshot);
        assert_eq!(
            check_panel_replacement(&state, &replace, &replacement),
            Ok(())
        );
        state.apply(&replacement);
        assert_eq!(state.seats()[&0], *replacement.seat.as_ref().unwrap());
        assert_eq!(state.seats()[&1], snapshot.seats()[&1]);
        assert_eq!(
            state.seated_route_keys(),
            &BTreeSet::from(["p/a".into(), "p/b".into(), "p/c".into()])
        );
        assert_eq!(
            state.seated_decision_ids(),
            &BTreeSet::from([
                "decision-a".into(),
                "decision-b".into(),
                "decision-c".into()
            ])
        );
    }

    #[test]
    fn never_seated_slot_can_be_replaced_even_after_cause() {
        let (request, mut panel) = fixture();
        panel.seats.remove(0);
        panel.shortfall = Some(PanelShortfall::new(ShortfallCode::SizeReduced));
        assert_eq!(check_panel_answer(&request, &panel), Ok(()));
        let mut state = PanelState::new(request, panel);
        let replace = PanelReplacementRequest::new("panel", 0);
        let cause = PanelReplacement::unseated("panel", 0, ReplacementCause::NoCandidate);
        assert_eq!(check_panel_replacement(&state, &replace, &cause), Ok(()));
        state.apply(&cause);
        assert!(!state.seats().contains_key(&0));
        let answer = PanelReplacement::seated("panel", seat(0, "c"));
        assert_eq!(check_panel_replacement(&state, &replace, &answer), Ok(()));
        state.apply(&answer);
        assert_eq!(state.seats().len(), 2);
    }

    #[test]
    fn open_panel_sets_round_trip_and_absent_members_are_omitted() {
        for text in ["audit", "future"] {
            let encoded = json!(text);
            let class: PanelClass = serde_json::from_value(encoded.clone()).unwrap();
            assert_eq!(serde_json::to_value(class).unwrap(), encoded);
        }
        let phase: PanelPhase = serde_json::from_value(json!("future")).unwrap();
        let code: ShortfallCode = serde_json::from_value(json!("future")).unwrap();
        let cause: ReplacementCause = serde_json::from_value(json!("future")).unwrap();
        assert_eq!(serde_json::to_value(phase).unwrap(), json!("future"));
        assert_eq!(serde_json::to_value(code).unwrap(), json!("future"));
        assert_eq!(serde_json::to_value(cause).unwrap(), json!("future"));
        let request = SelectPanelRequest::new(PanelClass::Audit, "c");
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({"class":"audit", "consumer":"c"})
        );
        assert_eq!(
            serde_json::to_value(PanelRequirements::new()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(PanelPolicy::new(1, 1, 0)).unwrap(),
            json!({"size":1, "top":1, "extra":0})
        );
        assert_eq!(
            serde_json::to_value(PanelShortfall::new(ShortfallCode::SizeReduced)).unwrap(),
            json!({"code":"size_reduced"})
        );
        assert_eq!(
            serde_json::to_value(PanelReplacementRequest::new("p", 0)).unwrap(),
            json!({"replace":{"panel_id":"p", "slot":0}})
        );
        assert_eq!(
            serde_json::to_value(PanelReplacement::unseated(
                "p",
                0,
                ReplacementCause::NoCandidate
            ))
            .unwrap(),
            json!({"panel_id":"p", "slot":0, "cause":"no_candidate"})
        );
        let (_, answer) = fixture();
        let encoded = serde_json::to_value(&answer).unwrap();
        assert!(encoded.get("shortfall").is_none());
        assert!(encoded["seats"][1].get("temperature").is_none());
        assert_eq!(
            serde_json::from_value::<SelectPanelAnswer>(encoded).unwrap(),
            answer
        );
        let mut unknown_family = seat(1, "b");
        unknown_family.family = None;
        assert!(serde_json::to_value(unknown_family)
            .unwrap()
            .get("family")
            .is_none());
        let (mut request, mut answer) = fixture();
        request.requirements = None;
        request.temperatures = None;
        for seat in &mut answer.seats {
            seat.temperature = None;
        }
        answer.policy.ranking = Some(json!({"arbitrary":[1, true]}));
        answer.policy.floor = Some(json!("informative"));
        assert_eq!(check_panel_answer(&request, &answer), Ok(()));
    }

    #[test]
    fn replacement_exclude_refuses_a_variant_pin() {
        let ok = serde_json::json!({"replace": {"panel_id": "p", "slot": 0}, "exclude": ["provider-a/model-b"]});
        assert!(parse_panel_replacement_request(&ok).is_ok());
        let pin = serde_json::json!({"replace": {"panel_id": "p", "slot": 0}, "exclude": ["provider-a/model-b:fast"]});
        let refusal = parse_panel_replacement_request(&pin).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], "exclude");
    }

    #[test]
    fn replacement_exclude_accepts_a_colon_in_the_provider_id() {
        let ok = serde_json::json!({"replace": {"panel_id": "p", "slot": 0}, "exclude": ["org:edge/model-b"]});
        assert!(parse_panel_replacement_request(&ok).is_ok());
    }
}
