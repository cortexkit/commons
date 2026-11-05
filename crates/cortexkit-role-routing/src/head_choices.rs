//! Unranked model choices for a session-starting runner (`route.head_choices`).
//!
//! Call only routers declaring `head_choices`. The router chooses its admitted
//! subset at `catalog_version`; limits and prices still come from the catalog.
//! Unknown runners and runners that cannot start sessions answer an empty list.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::billing::Billing;
use crate::errors::Refusal;
use crate::members::Members;

/// The runner whose admitted model choices are requested. Any caller may ask.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeadChoicesRequest {
    pub runner_module: String,
}

impl HeadChoicesRequest {
    pub fn new(runner_module: impl Into<String>) -> Self {
        Self {
            runner_module: runner_module.into(),
        }
    }
}

/// One admitted provider/model pair. A router may list a model under its
/// preferred provider only, or under several providers; neither is required.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeadChoice {
    pub provider: String,
    pub model: String,
    pub variants: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_variant: Option<String>,
    pub service_tiers: Vec<String>,
    /// Absent means false: the router may choose this model itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing: Option<Billing>,
}

impl HeadChoice {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            variants: Vec::new(),
            default_variant: None,
            service_tiers: Vec::new(),
            pin_only: None,
            billing: None,
        }
    }

    pub fn with_pin_only(mut self, pin_only: bool) -> Self {
        self.pin_only = Some(pin_only);
        self
    }

    pub fn with_billing(mut self, billing: Billing) -> Self {
        self.billing = Some(billing);
        self
    }
}

/// Unranked choices computed from the stated model catalog version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeadChoicesAnswer {
    pub catalog_version: String,
    pub runner_module: String,
    pub models: Vec<HeadChoice>,
}

impl HeadChoicesAnswer {
    pub fn new(catalog_version: impl Into<String>, runner_module: impl Into<String>) -> Self {
        Self {
            catalog_version: catalog_version.into(),
            runner_module: runner_module.into(),
            models: Vec::new(),
        }
    }
}

/// Decode a request after discovery has declared `head_choices`.
pub fn parse_head_choices_request(params: &Value) -> Result<HeadChoicesRequest, Refusal> {
    let members = Members::of(params)?;
    Ok(HeadChoicesRequest::new(
        members.text("runner_module", None)?,
    ))
}

/// A violation visible from the request and answer alone.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeadChoicesProblem {
    RunnerModule,
    ModelsOrder,
    VariantsOrder { index: usize },
    ServiceTiersOrder { index: usize },
    DefaultVariant { index: usize },
}

impl std::fmt::Display for HeadChoicesProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RunnerModule => f.write_str("runner_module does not echo the request"),
            Self::ModelsOrder => {
                f.write_str("models must be bytewise sorted and unique by provider/model")
            }
            Self::VariantsOrder { index } => write!(
                f,
                "models[{index}].variants must be bytewise sorted and unique"
            ),
            Self::ServiceTiersOrder { index } => write!(
                f,
                "models[{index}].service_tiers must be bytewise sorted and unique"
            ),
            Self::DefaultVariant { index } => {
                write!(f, "models[{index}].default_variant is not in variants")
            }
        }
    }
}

/// Check echoes, strict bytewise ordering (which also forbids duplicates), and
/// default membership. Billing's closed set is enforced when decoding the answer.
/// Runner reachability and catalog admission need router
/// state and cannot be inferred from the answer alone.
pub fn check_head_choices(
    request: &HeadChoicesRequest,
    answer: &HeadChoicesAnswer,
) -> Result<(), HeadChoicesProblem> {
    if answer.runner_module != request.runner_module {
        return Err(HeadChoicesProblem::RunnerModule);
    }
    if !answer
        .models
        .windows(2)
        .all(|pair| (&pair[0].provider, &pair[0].model) < (&pair[1].provider, &pair[1].model))
    {
        return Err(HeadChoicesProblem::ModelsOrder);
    }
    for (index, model) in answer.models.iter().enumerate() {
        if !strictly_sorted(&model.variants) {
            return Err(HeadChoicesProblem::VariantsOrder { index });
        }
        if !strictly_sorted(&model.service_tiers) {
            return Err(HeadChoicesProblem::ServiceTiersOrder { index });
        }
        if model
            .default_variant
            .as_ref()
            .is_some_and(|default| !model.variants.contains(default))
        {
            return Err(HeadChoicesProblem::DefaultVariant { index });
        }
    }
    Ok(())
}

fn strictly_sorted(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (HeadChoicesRequest, HeadChoicesAnswer) {
        let request = HeadChoicesRequest::new("runner");
        let mut answer = HeadChoicesAnswer::new("catalog-1", "runner");
        let mut model = HeadChoice::new("a", "model");
        model.variants = vec!["a".into(), "z".into(), "é".into()];
        model.default_variant = Some("z".into());
        model.service_tiers = vec!["future".into(), "standard".into()];
        answer.models = vec![model, HeadChoice::new("b", "model")];
        (request, answer)
    }

    #[test]
    fn head_choices_accepts_empty_and_both_provider_listing_policies() {
        let (request, mut answer) = fixture();
        assert_eq!(check_head_choices(&request, &answer), Ok(()));
        answer.models.pop();
        assert_eq!(check_head_choices(&request, &answer), Ok(()));
        answer.models.clear();
        assert_eq!(check_head_choices(&request, &answer), Ok(()));
    }

    #[test]
    fn head_choices_requires_runner_echo() {
        let (request, mut answer) = fixture();
        answer.runner_module = "other".into();
        assert_eq!(
            check_head_choices(&request, &answer),
            Err(HeadChoicesProblem::RunnerModule)
        );
    }

    #[test]
    fn head_choices_models_are_sorted_and_unique_by_pair() {
        let (request, mut answer) = fixture();
        answer.models.reverse();
        assert_eq!(
            check_head_choices(&request, &answer),
            Err(HeadChoicesProblem::ModelsOrder)
        );
        answer.models = vec![HeadChoice::new("a", "z"), HeadChoice::new("a", "a")];
        assert_eq!(
            check_head_choices(&request, &answer),
            Err(HeadChoicesProblem::ModelsOrder)
        );
        answer.models.reverse();
        assert_eq!(check_head_choices(&request, &answer), Ok(()));
        answer.models[1] = answer.models[0].clone();
        assert_eq!(
            check_head_choices(&request, &answer),
            Err(HeadChoicesProblem::ModelsOrder)
        );
    }

    #[test]
    fn head_choices_variants_are_bytewise_sorted_and_unique() {
        let (request, mut answer) = fixture();
        answer.models[0].variants.swap(1, 2);
        assert_eq!(
            check_head_choices(&request, &answer),
            Err(HeadChoicesProblem::VariantsOrder { index: 0 })
        );
        answer.models[0].variants = vec!["z".into(), "z".into()];
        assert_eq!(
            check_head_choices(&request, &answer),
            Err(HeadChoicesProblem::VariantsOrder { index: 0 })
        );
    }

    #[test]
    fn head_choices_tiers_are_sorted_and_unique() {
        let (request, mut answer) = fixture();
        for tiers in [vec!["z", "a"], vec!["a", "a"]] {
            answer.models[0].service_tiers = tiers.into_iter().map(String::from).collect();
            assert_eq!(
                check_head_choices(&request, &answer),
                Err(HeadChoicesProblem::ServiceTiersOrder { index: 0 })
            );
        }
    }

    #[test]
    fn head_choices_default_must_be_in_variants() {
        let (request, mut answer) = fixture();
        answer.models[0].default_variant = Some("missing".into());
        assert_eq!(
            check_head_choices(&request, &answer),
            Err(HeadChoicesProblem::DefaultVariant { index: 0 })
        );
        answer.models[0].default_variant = None;
        answer.models[0].variants.clear();
        assert_eq!(check_head_choices(&request, &answer), Ok(()));
        answer.models[0].default_variant = Some("missing".into());
        assert_eq!(
            check_head_choices(&request, &answer),
            Err(HeadChoicesProblem::DefaultVariant { index: 0 })
        );
    }

    #[test]
    fn head_choices_parser_names_invalid_runner_module() {
        use serde_json::json;
        for body in [
            json!({}),
            json!({"runner_module": null}),
            json!({"runner_module": ""}),
            json!({"runner_module": 1}),
        ] {
            assert_eq!(
                parse_head_choices_request(&body).unwrap_err().detail["field"],
                "runner_module"
            );
        }
        assert_eq!(
            parse_head_choices_request(&json!({"runner_module": "runner"})).unwrap(),
            HeadChoicesRequest::new("runner")
        );
    }

    #[test]
    fn head_choices_wire_omits_absent_default_and_retains_catalog_strings() {
        let model = HeadChoice::new("p", "m");
        assert_eq!(
            serde_json::to_string(&model).unwrap(),
            r#"{"provider":"p","model":"m","variants":[],"service_tiers":[]}"#
        );
        let (request, answer) = fixture();
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"runner_module":"runner"}"#
        );
        let bytes = serde_json::to_string(&answer).unwrap();
        assert_eq!(
            serde_json::from_str::<HeadChoicesAnswer>(&bytes).unwrap(),
            answer
        );
    }

    #[test]
    fn head_choices_billing_and_pin_only_are_optional_and_closed() {
        let model = HeadChoice::new("p", "m")
            .with_pin_only(true)
            .with_billing(Billing::Subscription);
        let mut value = serde_json::to_value(&model).unwrap();
        assert_eq!(value["pin_only"], true);
        assert_eq!(value["billing"], "subscription");
        assert_eq!(
            serde_json::from_value::<HeadChoice>(value.clone()).unwrap(),
            model
        );
        value["billing"] = serde_json::json!("free");
        assert!(serde_json::from_value::<HeadChoice>(value).is_err());
        let mut value = serde_json::to_value(HeadChoice::new("p", "m")).unwrap();
        value["future"] = serde_json::json!(true);
        let decoded: HeadChoice = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.pin_only, None);
        assert_eq!(decoded.billing, None);
    }
}
