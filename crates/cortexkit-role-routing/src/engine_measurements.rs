//! Attested media-engine speed reports and reference-set accuracy evaluations.
//!
//! Providers retain their documented rolling speed window, bind job reports to
//! engine decisions, and expose samples to apps. The helpers here validate wire
//! data and comparable accuracy; they do not implement a router's storage.

use crate::{
    errors::{Refusal, RefusalCode},
    members::Members,
    select_engine::{
        meets_requirements, Engine, EngineCandidate, Kind, Privacy, SelectEngineRequest,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EngineIdentity {
    pub provider: String,
    pub model: String,
}

impl EngineIdentity {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EngineInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chars: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EngineReportRequest {
    pub decision_id: String,
    pub kind: Kind,
    pub engine: EngineIdentity,
    /// Open media usage outcome: known values are done, failed, cancelled, ended.
    pub outcome: String,
    pub input: EngineInput,
    pub wall_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_output_ms: Option<u64>,
}

impl EngineReportRequest {
    pub fn new(
        decision_id: impl Into<String>,
        kind: Kind,
        engine: EngineIdentity,
        outcome: impl Into<String>,
        input: EngineInput,
        wall_ms: u64,
    ) -> Self {
        Self {
            decision_id: decision_id.into(),
            kind,
            engine,
            outcome: outcome.into(),
            input,
            wall_ms,
            first_output_ms: None,
        }
    }

    /// Zero or missing input duration cannot establish a realtime factor.
    pub fn realtime_factor(&self) -> Option<f64> {
        self.input
            .duration_ms
            .filter(|duration| *duration > 0)
            .map(|duration| self.wall_ms as f64 / duration as f64)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EvaluationSet {
    pub id: String,
    pub items: u64,
}

impl EvaluationSet {
    pub fn new(id: impl Into<String>, items: u64) -> Self {
        Self {
            id: id.into(),
            items,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EngineEvalRequest {
    pub kind: Kind,
    pub engine: EngineIdentity,
    pub metric: String,
    pub score: f64,
    pub set: EvaluationSet,
    pub measured_at_ms: u64,
}

impl EngineEvalRequest {
    pub fn new(
        kind: Kind,
        engine: EngineIdentity,
        metric: impl Into<String>,
        score: f64,
        set: EvaluationSet,
        measured_at_ms: u64,
    ) -> Self {
        Self {
            kind,
            engine,
            metric: metric.into(),
            score,
            set,
            measured_at_ms,
        }
    }
}

/// Both measurement operations acknowledge with an empty object.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EngineMeasurementAnswer {}

fn only(members: &Members<'_>, allowed: &[&str], prefix: &str) -> Result<(), Refusal> {
    if let Some(key) = members
        .object
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        return Err(Refusal::invalid_request(
            format!("{prefix}{key}"),
            "unknown member",
        ));
    }
    Ok(())
}

fn nested<'a>(
    members: &Members<'a>,
    field: &str,
    allowed: &[&str],
) -> Result<Members<'a>, Refusal> {
    let object = members
        .object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| members.refuse(field, "expected an object"))?;
    let inner = Members {
        object,
        refuse: members.refuse,
    };
    only(&inner, allowed, &format!("{field}."))?;
    Ok(inner)
}

fn path<T>(result: Result<T, Refusal>, prefix: &str) -> Result<T, Refusal> {
    result.map_err(|mut error| {
        if let Some(field) = error.detail.get("field").and_then(Value::as_str) {
            error
                .detail
                .insert("field".into(), Value::String(format!("{prefix}.{field}")));
        }
        error
    })
}

fn identity(members: &Members<'_>) -> Result<EngineIdentity, Refusal> {
    let inner = nested(members, "engine", &["provider", "model"])?;
    Ok(EngineIdentity::new(
        path(inner.required::<String>("provider"), "engine")?,
        path(inner.required::<String>("model"), "engine")?,
    ))
}

pub fn parse_engine_report_request(value: &Value) -> Result<EngineReportRequest, Refusal> {
    let members = Members::of(value)?;
    only(
        &members,
        &[
            "decision_id",
            "kind",
            "engine",
            "outcome",
            "input",
            "wall_ms",
            "first_output_ms",
        ],
        "",
    )?;
    let input = nested(&members, "input", &["duration_ms", "bytes", "chars"])?;
    Ok(EngineReportRequest {
        decision_id: members.required("decision_id")?,
        kind: members.required("kind")?,
        engine: identity(&members)?,
        outcome: members.required("outcome")?,
        input: EngineInput {
            duration_ms: path(input.optional("duration_ms"), "input")?,
            bytes: path(input.optional("bytes"), "input")?,
            chars: path(input.optional("chars"), "input")?,
        },
        wall_ms: members.required("wall_ms")?,
        first_output_ms: members.optional("first_output_ms")?,
    })
}

pub fn parse_engine_eval_request(value: &Value) -> Result<EngineEvalRequest, Refusal> {
    let members = Members::of(value)?;
    only(
        &members,
        &["kind", "engine", "metric", "score", "set", "measured_at_ms"],
        "",
    )?;
    let set = nested(&members, "set", &["id", "items"])?;
    let score: f64 = members.required("score")?;
    EvalScore::new(score)?;
    Ok(EngineEvalRequest {
        kind: members.required("kind")?,
        engine: identity(&members)?,
        metric: members.required("metric")?,
        score,
        set: EvaluationSet::new(
            path(set.required::<String>("id"), "set")?,
            path(set.required("items"), "set")?,
        ),
        measured_at_ms: members.required("measured_at_ms")?,
    })
}

/// Declarations and their principals must come from validated, nonce-verified
/// catalog state. Caller-local request entries must never be passed here.
pub fn check_engine_authorization<'a>(
    caller: &str,
    kind: &Kind,
    identity: &EngineIdentity,
    declarations: impl IntoIterator<Item = (&'a str, &'a Engine)>,
) -> Result<(), Refusal> {
    if declarations.into_iter().any(|(principal, engine)| {
        principal == caller
            && &engine.kind == kind
            && engine.provider == identity.provider
            && engine.model == identity.model
    }) {
        Ok(())
    } else {
        Err(Refusal::new(
            RefusalCode::EngineNotDeclaredByCaller,
            "engine not declared by attested caller",
        ))
    }
}

/// An ordered, validated accuracy score; unlike a raw float it cannot hold NaN.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvalScore(f64);
impl EvalScore {
    pub fn new(score: f64) -> Result<Self, Refusal> {
        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
            Err(Refusal::invalid_request(
                "score",
                "expected a finite number in 0..=1",
            ))
        } else {
            Ok(Self(if score == 0.0 { 0.0 } else { score }))
        }
    }
    pub fn get(self) -> f64 {
        self.0
    }
}
impl Eq for EvalScore {}
impl PartialOrd for EvalScore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for EvalScore {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Assign comparable accuracy only after applying eligibility, so an excluded
/// engine's newer reference set cannot change the remaining engines' ranking.
/// Accepted evaluations must have passed parsing and attested authorization.
pub fn apply_evaluations<Cost>(
    request: &SelectEngineRequest,
    candidates: &mut [EngineCandidate<EvalScore, Cost>],
    evaluations: &[EngineEvalRequest],
) {
    let metric = request
        .requirements
        .metric
        .as_deref()
        .or(match request.kind {
            Kind::Transcribe => Some("wer"),
            _ => None,
        });
    let eligible = |candidate: &EngineCandidate<EvalScore, Cost>| {
        candidate.engine.kind == request.kind
            && meets_requirements(&candidate.engine, &request.requirements)
            && (request.privacy == Privacy::RemoteAllowed || candidate.engine.local)
            && (candidate.engine.local || !candidate.fenced)
            && !candidate.selected.caller_local
    };
    let matches = |eval: &EngineEvalRequest, candidate: &EngineCandidate<EvalScore, Cost>| {
        eligible(candidate)
            && eval.kind == request.kind
            && Some(eval.metric.as_str()) == metric
            && eval.engine.provider == candidate.engine.provider
            && eval.engine.model == candidate.engine.model
    };
    let set = evaluations
        .iter()
        .filter(|eval| candidates.iter().any(|candidate| matches(eval, candidate)))
        .max_by(|a, b| {
            a.measured_at_ms
                .cmp(&b.measured_at_ms)
                .then_with(|| b.set.id.as_bytes().cmp(a.set.id.as_bytes()))
        })
        .map(|eval| eval.set.id.as_str());
    for candidate in candidates {
        candidate.accuracy = evaluations
            .iter()
            .filter(|eval| Some(eval.set.id.as_str()) == set && matches(eval, candidate))
            .max_by_key(|eval| eval.measured_at_ms)
            .and_then(|eval| EvalScore::new(eval.score).ok());
    }
}
