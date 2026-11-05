//! Consumer-specific stored model judgments (`route.consumer_models`).
//!
//! Call only routers declaring `consumer_models`. Admission is the same as
//! selection: the router checks consumer existence and ownership from trusted
//! caller and configuration state. Catalog facts are read from the model catalog.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::billing::Billing;
use crate::errors::Refusal;
use crate::members::Members;

/// Consumer whose stored model judgments are requested. Unknown members fail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ConsumerModelsRequest {
    pub consumer: String,
}

impl ConsumerModelsRequest {
    pub fn new(consumer: impl Into<String>) -> Self {
        Self {
            consumer: consumer.into(),
        }
    }
}

/// One provider/model pair and the router's judgments for the requested consumer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConsumerModel {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<f64>,
    pub excluded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing: Option<Billing>,
}

impl ConsumerModel {
    pub fn new(provider: impl Into<String>, model: impl Into<String>, excluded: bool) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            elo: None,
            eq: None,
            excluded,
            billing: None,
        }
    }

    pub fn with_elo(mut self, elo: f64) -> Self {
        self.elo = Some(elo);
        self
    }

    pub fn with_eq(mut self, eq: f64) -> Self {
        self.eq = Some(eq);
        self
    }

    pub fn with_billing(mut self, billing: Billing) -> Self {
        self.billing = Some(billing);
        self
    }
}

/// Sorted, unranked judgments at the stated catalog version; empty is valid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConsumerModelsAnswer {
    pub catalog_version: String,
    pub consumer: String,
    pub models: Vec<ConsumerModel>,
}

impl ConsumerModelsAnswer {
    pub fn new(catalog_version: impl Into<String>, consumer: impl Into<String>) -> Self {
        Self {
            catalog_version: catalog_version.into(),
            consumer: consumer.into(),
            models: Vec::new(),
        }
    }
}

/// Decode a strict request after discovery has declared `consumer_models`.
/// Existence and ownership require router state, just as for `route.select`.
pub fn parse_consumer_models_request(params: &Value) -> Result<ConsumerModelsRequest, Refusal> {
    let members = Members::of(params)?;
    for field in members.object.keys() {
        if field != "consumer" {
            return Err(members.refuse(field, "unknown request member"));
        }
    }
    Ok(ConsumerModelsRequest::new(members.text("consumer", None)?))
}

/// A violation visible from the request and answer alone.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConsumerModelsProblem {
    Consumer,
    ModelsOrder,
    Elo { index: usize },
    Eq { index: usize },
}

impl std::fmt::Display for ConsumerModelsProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Consumer => f.write_str("consumer does not echo the request"),
            Self::ModelsOrder => {
                f.write_str("models must be bytewise sorted and unique by provider/model")
            }
            Self::Elo { index } => write!(f, "models[{index}].elo must be finite"),
            Self::Eq { index } => write!(f, "models[{index}].eq must be finite"),
        }
    }
}

/// Check consumer echo, bytewise order, uniqueness and finite scores. Billing's
/// closed set is enforced on decode. Admission requires trusted router state.
pub fn check_consumer_models(
    request: &ConsumerModelsRequest,
    answer: &ConsumerModelsAnswer,
) -> Result<(), ConsumerModelsProblem> {
    if answer.consumer != request.consumer {
        return Err(ConsumerModelsProblem::Consumer);
    }
    if !answer
        .models
        .windows(2)
        .all(|pair| (&pair[0].provider, &pair[0].model) < (&pair[1].provider, &pair[1].model))
    {
        return Err(ConsumerModelsProblem::ModelsOrder);
    }
    for (index, model) in answer.models.iter().enumerate() {
        if model.elo.is_some_and(|score| !score.is_finite()) {
            return Err(ConsumerModelsProblem::Elo { index });
        }
        if model.eq.is_some_and(|score| !score.is_finite()) {
            return Err(ConsumerModelsProblem::Eq { index });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (ConsumerModelsRequest, ConsumerModelsAnswer) {
        let request = ConsumerModelsRequest::new("consumer");
        let mut answer = ConsumerModelsAnswer::new("catalog-1", "consumer");
        answer.models = vec![
            ConsumerModel::new("a", "z", false)
                .with_elo(1200.5)
                .with_eq(-0.25)
                .with_billing(Billing::PerToken),
            ConsumerModel::new("b", "a", true),
        ];
        (request, answer)
    }

    #[test]
    fn consumer_models_checks_echo_order_and_uniqueness() {
        let (request, mut answer) = fixture();
        assert_eq!(check_consumer_models(&request, &answer), Ok(()));
        answer.consumer = "other".into();
        assert_eq!(
            check_consumer_models(&request, &answer),
            Err(ConsumerModelsProblem::Consumer)
        );
        answer.consumer = request.consumer.clone();
        answer.models.reverse();
        assert_eq!(
            check_consumer_models(&request, &answer),
            Err(ConsumerModelsProblem::ModelsOrder)
        );
        answer.models[1] = answer.models[0].clone();
        assert_eq!(
            check_consumer_models(&request, &answer),
            Err(ConsumerModelsProblem::ModelsOrder)
        );
        answer.models.clear();
        assert_eq!(check_consumer_models(&request, &answer), Ok(()));
    }

    #[test]
    fn consumer_models_rejects_nonfinite_scores() {
        let (request, answer) = fixture();
        for score in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut invalid = answer.clone();
            invalid.models[0].elo = Some(score);
            assert_eq!(
                check_consumer_models(&request, &invalid),
                Err(ConsumerModelsProblem::Elo { index: 0 })
            );
            invalid.models[0].elo = None;
            invalid.models[0].eq = Some(score);
            assert_eq!(
                check_consumer_models(&request, &invalid),
                Err(ConsumerModelsProblem::Eq { index: 0 })
            );
        }
    }

    #[test]
    fn consumer_models_request_is_strict_with_select_consumer_grammar() {
        for (body, field) in [
            (json!({}), "consumer"),
            (json!({"consumer":null}), "consumer"),
            (json!({"consumer":""}), "consumer"),
            (json!({"consumer":1}), "consumer"),
            (json!({"consumer":"c", "extra":true}), "extra"),
        ] {
            assert_eq!(
                parse_consumer_models_request(&body).unwrap_err().detail["field"],
                field
            );
        }
        // Consumer ids have no character-set or length restriction beyond nonempty.
        let body = json!({"consumer":"opaque / 名\n".repeat(200)});
        assert_eq!(
            serde_json::to_value(parse_consumer_models_request(&body).unwrap()).unwrap(),
            body
        );
        assert!(serde_json::from_value::<ConsumerModelsRequest>(
            json!({"consumer":"c", "extra":true})
        )
        .is_err());
    }

    #[test]
    fn consumer_models_wire_preserves_optional_scores_and_closed_billing() {
        let (_, answer) = fixture();
        let value = serde_json::to_value(&answer).unwrap();
        assert_eq!(value["models"][0]["elo"], 1200.5);
        assert_eq!(value["models"][0]["eq"], -0.25);
        assert_eq!(
            value["models"][1],
            json!({"provider":"b","model":"a","excluded":true})
        );
        assert_eq!(
            serde_json::from_value::<ConsumerModelsAnswer>(value.clone()).unwrap(),
            answer
        );
        let mut invalid = value.clone();
        invalid["models"][0]["billing"] = json!("free");
        assert!(serde_json::from_value::<ConsumerModelsAnswer>(invalid).is_err());
        let mut future = value;
        future["unknown"] = json!(true);
        future["models"][0]["unknown"] = json!(true);
        assert_eq!(
            serde_json::from_value::<ConsumerModelsAnswer>(future).unwrap(),
            answer
        );
    }
}
