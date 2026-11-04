//! Admission of a durable model and default tier for subsequent runs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{capabilities, describe::RoleDescribe, service_tier::ServiceTier};

pub const UNSUPPORTED_TIER: &str = "unsupported_tier";
pub const UNSUPPORTED_MODEL: &str = "unsupported_model";

/// The strict `plan.model` member, not a description of a running run.
/// An omitted tier means standard, and equality compares that effective tier.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PlanModel {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<ServiceTier>,
}

impl PartialEq for PlanModel {
    fn eq(&self, other: &Self) -> bool {
        self.provider == other.provider
            && self.model == other.model
            && self.variant == other.variant
            && self.service_tier.unwrap_or_default() == other.service_tier.unwrap_or_default()
    }
}

impl Eq for PlanModel {}

impl PlanModel {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            variant: None,
            service_tier: None,
        }
    }

    pub fn with_variant(mut self, variant: impl Into<String>) -> Self {
        self.variant = Some(variant.into());
        self
    }

    pub fn with_service_tier(mut self, tier: ServiceTier) -> Self {
        self.service_tier = Some(tier);
        self
    }
}

/// An `invalid_params` refusal, emitted before accepting or writing the plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanModelProblem {
    Malformed,
    UnsupportedTier,
    UnsupportedModel,
}

impl PlanModelProblem {
    pub fn detail(self) -> Value {
        match self {
            Self::Malformed => serde_json::json!({"field": "plan.model"}),
            Self::UnsupportedTier => serde_json::json!({
                "field": "plan.model.service_tier", "reason": UNSUPPORTED_TIER
            }),
            Self::UnsupportedModel => serde_json::json!({
                "field": "plan.model", "reason": UNSUPPORTED_MODEL
            }),
        }
    }
}

/// Check a raw plan before accepting a refresh or a session-creating send.
/// `route` resolves the runner's selectable model to its wire API; `None`
/// means the host cannot select or serve it. Resolution must not write state.
/// Unknown plan members, including `model` without the group, remain ignored.
pub fn check_plan(
    plan: &Value,
    describe: &RoleDescribe,
    route: impl FnOnce(&PlanModel) -> Option<String>,
) -> Result<Option<PlanModel>, PlanModelProblem> {
    if !describe.declares(capabilities::HEAD_SELECTION) {
        return Ok(None);
    }
    let Some(value) = plan.get("model") else {
        return Ok(None);
    };
    let mut shape = value.clone();
    if let Some(object) = shape.as_object_mut() {
        object.remove("service_tier");
    }
    let mut model: PlanModel =
        serde_json::from_value(shape).map_err(|_| PlanModelProblem::Malformed)?;
    model.service_tier = match value.get("service_tier") {
        None | Some(Value::Null) => None,
        Some(Value::String(tier)) => {
            Some(ServiceTier::parse(tier).ok_or(PlanModelProblem::UnsupportedTier)?)
        }
        Some(_) => return Err(PlanModelProblem::Malformed),
    };
    let wire = route(&model).ok_or(PlanModelProblem::UnsupportedModel)?;
    let declared = describe
        .declares(capabilities::SERVICE_TIER)
        .then_some(describe.service_tiers.as_ref())
        .flatten();
    crate::service_tier::check_send(model.service_tier, None, &wire, declared)
        .map_err(|_| PlanModelProblem::UnsupportedTier)?;
    Ok(Some(model))
}

/// Joining sends inherit the running run, never a newly refreshed default.
/// Pass the latest recorded selection, including any default-tier fallback
/// from a host switch, rather than the original frozen plan's model member.
pub fn check_send_tier(
    requested: Option<ServiceTier>,
    running: Option<ServiceTier>,
    plan: Option<&PlanModel>,
    wire: &str,
    describe: &RoleDescribe,
) -> Result<ServiceTier, crate::service_tier::TierProblem> {
    let requested = requested.or_else(|| {
        (running.is_none() && describe.declares(capabilities::HEAD_SELECTION))
            .then(|| plan.and_then(|model| model.service_tier))
            .flatten()
    });
    let declared = describe
        .declares(capabilities::SERVICE_TIER)
        .then_some(describe.service_tiers.as_ref())
        .flatten();
    crate::service_tier::check_send(requested, running, wire, declared)
}
