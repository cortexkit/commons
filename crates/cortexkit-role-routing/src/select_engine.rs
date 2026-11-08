//! Media engine selection (`route.select_engine`, group `select_engine`).
//!
//! Catalog attestation, consumer ownership, retained decisions and admission
//! replay are provider responsibilities, pinned in CONTRACT.md §§9 and 12.

use cortexkit_role_media::describe::{language_matches, mime_matches};
pub use cortexkit_role_media::describe::{Engine, Kind, LatencyClass};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{errors::Refusal, members::Members};

/// The catalog slot read independently of `routing/v1` declarations.
pub const ROLE_DECLARATIONS_KEY: &str = "media-provider/v1";
/// Lifetime of an engine decision, measured from selection.
pub const DECISION_TTL_MS: u64 = 600_000;

/// Privacy is closed; unknown values are not permission to use remote engines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Privacy {
    LocalOnly,
    RemoteAllowed,
}

/// The first filter that left no engines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineFilter {
    Kind,
    Requirements,
    Privacy,
    Reachability,
}

impl std::fmt::Display for EngineFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Kind => "kind",
            Self::Requirements => "requirements",
            Self::Privacy => "privacy",
            Self::Reachability => "reachability",
        })
    }
}

/// Every requirement is optional; an absent fact fails a stated requirement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EngineRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_formats: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bias: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_class: Option<LatencyClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
}

macro_rules! requirement_builders {
    ($($name:ident: $ty:ty => $method:ident),* $(,)?) => {
        $(pub fn $method(mut self, value: $ty) -> Self {
            self.$name = Some(value);
            self
        })*
    };
}

impl EngineRequirements {
    pub fn new() -> Self {
        Self::default()
    }

    requirement_builders! {
        language: String => with_language, voice: String => with_voice,
        format: String => with_format, duration_ms: u64 => with_duration_ms,
        input_bytes: u64 => with_input_bytes,
        reference_formats: Vec<String> => with_reference_formats,
        bias: bool => with_bias, size: String => with_size,
        resolution: String => with_resolution, quality: String => with_quality,
        latency_class: LatencyClass => with_latency_class,
        metric: String => with_metric,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SelectEngineRequest {
    pub consumer: String,
    pub kind: Kind,
    pub requirements: EngineRequirements,
    pub privacy: Privacy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_local: Option<Vec<Engine>>,
}

impl SelectEngineRequest {
    pub fn new(
        consumer: impl Into<String>,
        kind: Kind,
        requirements: EngineRequirements,
        privacy: Privacy,
    ) -> Self {
        Self {
            consumer: consumer.into(),
            kind,
            requirements,
            privacy,
            caller_local: None,
        }
    }

    pub fn with_caller_local(mut self, engines: Vec<Engine>) -> Self {
        self.caller_local = Some(engines);
        self
    }
}

/// The exclusive module/index matrix is checked by `check_answer`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectedEngine {
    pub provider: String,
    pub model: String,
    pub local: bool,
    pub caller_local: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

impl SelectedEngine {
    pub fn new(module: impl Into<String>, engine: &Engine) -> Self {
        Self {
            provider: engine.provider.clone(),
            model: engine.model.clone(),
            local: engine.local,
            caller_local: false,
            module: Some(module.into()),
            index: None,
        }
    }

    pub fn caller_local(index: usize, engine: &Engine) -> Self {
        Self {
            provider: engine.provider.clone(),
            model: engine.model.clone(),
            local: true,
            caller_local: true,
            module: None,
            index: Some(index),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectEngineAnswer {
    pub decision_id: String,
    pub expires_at_ms: u64,
    pub engine: SelectedEngine,
}

impl SelectEngineAnswer {
    pub fn new(decision_id: impl Into<String>, expires_at_ms: u64, engine: SelectedEngine) -> Self {
        Self {
            decision_id: decision_id.into(),
            expires_at_ms,
            engine,
        }
    }
}

/// Parse requests before doing any ownership lookup or selection.
pub fn parse_select_engine_request(value: &Value) -> Result<SelectEngineRequest, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| Refusal::invalid_request("params", "expected an object"))?;
    let members = Members {
        object,
        refuse: |field, message| Refusal::invalid_request(field, message),
    };
    only(
        &members,
        &[
            "consumer",
            "kind",
            "requirements",
            "privacy",
            "caller_local",
        ],
    )?;
    let consumer: String = members.required("consumer")?;
    if consumer.is_empty() {
        return Err(members.refuse("consumer", "consumer must not be empty"));
    }
    let kind: Kind = members.required("kind")?;
    let requirements = parse_requirements(object.get("requirements"), &kind)?;
    let privacy: Privacy = members.required("privacy")?;
    let caller_local = match object.get("caller_local").filter(|v| !v.is_null()) {
        None => None,
        Some(value) => Some(
            crate::engine_declarations::parse_engines(value, "caller_local", true)
                .map_err(|error| Refusal::invalid_request(error.field, error.code))?,
        ),
    };
    Ok(SelectEngineRequest {
        consumer,
        kind,
        requirements,
        privacy,
        caller_local,
    })
}

fn parse_requirements(value: Option<&Value>, kind: &Kind) -> Result<EngineRequirements, Refusal> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| Refusal::invalid_requirements("requirements", "expected an object"))?;
    let members = Members {
        object,
        refuse: |field, message| Refusal::invalid_requirements(field, message),
    };
    only(
        &members,
        &[
            "language",
            "voice",
            "format",
            "duration_ms",
            "input_bytes",
            "reference_formats",
            "bias",
            "size",
            "resolution",
            "quality",
            "latency_class",
            "metric",
        ],
    )?;
    let requirements = EngineRequirements {
        language: members.optional("language")?,
        voice: members.optional("voice")?,
        format: members.optional("format")?,
        duration_ms: members.optional("duration_ms")?,
        input_bytes: members.optional("input_bytes")?,
        reference_formats: members.optional("reference_formats")?,
        bias: members.optional("bias")?,
        size: members.optional("size")?,
        resolution: members.optional("resolution")?,
        quality: members.optional("quality")?,
        latency_class: members.optional("latency_class")?,
        metric: members.optional("metric")?,
    };
    if *kind == Kind::Realtime && requirements.format.is_some() {
        return Err(members.refuse("format", "realtime formats are not matched"));
    }
    if requirements
        .latency_class
        .as_ref()
        .is_some_and(|class| latency_rank(class).is_none())
    {
        return Err(members.refuse("latency_class", "unknown latency class"));
    }
    Ok(requirements)
}

fn latency_rank(class: &LatencyClass) -> Option<u8> {
    match class {
        LatencyClass::Realtime => Some(0),
        LatencyClass::Interactive => Some(1),
        LatencyClass::Batch => Some(2),
        _ => None,
    }
}

fn only(members: &Members<'_>, allowed: &[&str]) -> Result<(), Refusal> {
    if let Some(key) = members
        .object
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        return Err(members.refuse(key, "unknown member"));
    }
    Ok(())
}

/// Match only requirements. Kind, privacy and reachability are separate filters.
pub fn meets_requirements(engine: &Engine, requirements: &EngineRequirements) -> bool {
    fn listed(
        list: &Option<Vec<String>>,
        value: &Option<String>,
        matches: impl Fn(&str, &str) -> bool,
    ) -> bool {
        value.as_ref().is_none_or(|value| {
            list.as_ref()
                .is_some_and(|list| list.iter().any(|fact| matches(fact, value)))
        })
    }
    let equal = |a: &str, b: &str| a == b;
    listed(&engine.languages, &requirements.language, language_matches)
        && listed(&engine.voices, &requirements.voice, equal)
        && requirements
            .format
            .as_ref()
            .is_none_or(|format| engine.formats.iter().any(|fact| mime_matches(fact, format)))
        && requirements
            .duration_ms
            .is_none_or(|n| engine.max_duration_ms.is_some_and(|max| n <= max))
        && requirements
            .input_bytes
            .is_none_or(|n| engine.max_input_bytes.is_some_and(|max| n <= max))
        && requirements
            .reference_formats
            .as_ref()
            .is_none_or(|formats| {
                engine.reference_formats.as_ref().is_some_and(|facts| {
                    formats
                        .iter()
                        .all(|format| facts.iter().any(|fact| mime_matches(fact, format)))
                })
            })
        && (requirements.bias != Some(true) || engine.bias_limit.is_some())
        && listed(&engine.sizes, &requirements.size, equal)
        && listed(&engine.resolutions, &requirements.resolution, equal)
        && listed(&engine.qualities, &requirements.quality, equal)
        && requirements.latency_class.as_ref().is_none_or(|class| {
            match (latency_rank(&engine.latency_class), latency_rank(class)) {
                (Some(actual), Some(wanted)) => actual <= wanted,
                _ => false,
            }
        })
}

/// Check the answer's location matrix and caller-local identity against the request.
/// Module facts and the router's selection clock are not present in an answer.
pub fn check_answer(
    request: &SelectEngineRequest,
    answer: &SelectEngineAnswer,
) -> Result<(), Refusal> {
    let selected = &answer.engine;
    let invalid = || Refusal::invalid_request("engine", "invalid selected engine");
    if answer.decision_id.is_empty() || answer.decision_id.chars().count() > 256 {
        return Err(Refusal::invalid_request(
            "decision_id",
            "invalid decision id",
        ));
    }
    if request.privacy == Privacy::LocalOnly && !selected.local {
        return Err(invalid());
    }
    if selected.caller_local {
        let engine = selected
            .index
            .and_then(|i| request.caller_local.as_ref()?.get(i))
            .ok_or_else(invalid)?;
        if !selected.local
            || selected.module.is_some()
            || !engine.local
            || engine.kind != request.kind
            || engine.provider != selected.provider
            || engine.model != selected.model
            || !meets_requirements(engine, &request.requirements)
        {
            return Err(invalid());
        }
    } else if selected.index.is_some() || selected.module.as_ref().is_none_or(String::is_empty) {
        return Err(invalid());
    }
    Ok(())
}

/// One catalog or caller-local candidate, with comparable evaluation and cost facts.
/// Generic ordered facts avoid rounding accuracy or cost values.
/// Accuracy must be for this candidate's kind, metric and comparison set (§13).
/// Caller-local engines have no accuracy. `fenced` is the router's current
/// route-health judgment; it never excludes a local engine.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EngineCandidate<Accuracy, Cost> {
    pub engine: Engine,
    pub selected: SelectedEngine,
    pub accuracy: Option<Accuracy>,
    pub cost: Option<Cost>,
    pub fenced: bool,
}

impl<Accuracy, Cost> EngineCandidate<Accuracy, Cost> {
    pub fn new(module: impl Into<String>, engine: Engine) -> Self {
        Self {
            selected: SelectedEngine::new(module, &engine),
            engine,
            accuracy: None,
            cost: None,
            fenced: false,
        }
    }

    pub fn caller_local(index: usize, engine: Engine) -> Self {
        Self {
            selected: SelectedEngine::caller_local(index, &engine),
            engine,
            accuracy: None,
            cost: None,
            fenced: false,
        }
    }

    pub fn with_accuracy(mut self, accuracy: Accuracy) -> Self {
        self.accuracy = Some(accuracy);
        self
    }

    pub fn with_cost(mut self, cost: Cost) -> Self {
        self.cost = Some(cost);
        self
    }

    pub fn with_fenced(mut self, fenced: bool) -> Self {
        self.fenced = fenced;
        self
    }
}

/// Apply the four filters to whole sets, then accuracy, cost and name ranking.
/// Candidates must come from validated, nonce-verified slots and the request's
/// caller-local list. Consumer ownership is checked by the provider separately.
/// The provider mints and retains the decision only after this function succeeds.
pub fn select_engine<'a, Accuracy: Ord, Cost: Ord>(
    request: &SelectEngineRequest,
    candidates: &'a [EngineCandidate<Accuracy, Cost>],
) -> Result<&'a EngineCandidate<Accuracy, Cost>, Refusal> {
    let mut remaining: Vec<_> = candidates.iter().collect();
    for filter in [
        EngineFilter::Kind,
        EngineFilter::Requirements,
        EngineFilter::Privacy,
        EngineFilter::Reachability,
    ] {
        remaining.retain(|candidate| match filter {
            EngineFilter::Kind => candidate.engine.kind == request.kind,
            EngineFilter::Requirements => {
                meets_requirements(&candidate.engine, &request.requirements)
            }
            EngineFilter::Privacy => {
                request.privacy == Privacy::RemoteAllowed || candidate.engine.local
            }
            EngineFilter::Reachability => candidate.engine.local || !candidate.fenced,
        });
        if remaining.is_empty() {
            return Err(Refusal::no_engine(filter));
        }
    }
    remaining
        .into_iter()
        .min_by(|a, b| {
            b.accuracy
                .cmp(&a.accuracy)
                .then_with(|| a.cost.is_none().cmp(&b.cost.is_none()))
                .then_with(|| a.cost.cmp(&b.cost))
                .then_with(|| {
                    a.selected
                        .module
                        .as_deref()
                        .unwrap_or("")
                        .cmp(b.selected.module.as_deref().unwrap_or(""))
                })
                .then_with(|| a.engine.provider.cmp(&b.engine.provider))
                .then_with(|| a.engine.model.cmp(&b.engine.model))
                .then_with(|| a.selected.index.cmp(&b.selected.index))
        })
        .ok_or_else(|| Refusal::no_engine(EngineFilter::Kind))
}
