//! Live step publication checked against independent native durable records.

use cortexkit_role_llm_runner::{
    describe::check_describe,
    ops,
    step_events::{decode_step_finished, StepFinished, StepRef},
};
use serde_json::Value;

use crate::{
    cases::{Case, Ending},
    drive::{call, expect_ok},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedTurn},
};

/// One native durable record, read independently of the publication path.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RecordedStep {
    pub position: StepRef,
    pub run_id: String,
    pub step_id: String,
    pub attempt: std::num::NonZeroU64,
    /// False only for an internal retry hidden by `per_completed_step`.
    /// A terminal failed/cancelled step still completes a step.
    pub completes_step: bool,
    pub harness_step_ref: Option<String>,
}

impl RecordedStep {
    pub fn new(
        position: StepRef,
        run_id: impl Into<String>,
        step_id: impl Into<String>,
        attempt: std::num::NonZeroU64,
        completes_step: bool,
    ) -> Self {
        Self {
            position,
            run_id: run_id.into(),
            step_id: step_id.into(),
            attempt,
            completes_step,
            harness_step_ref: None,
        }
    }
    pub fn with_harness_step_ref(mut self, value: impl Into<String>) -> Self {
        self.harness_step_ref = Some(value.into());
        self
    }
}

/// A body captured at the actual bus publish seam, not regenerated from history.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PublishedStep {
    /// Actual publish subject, whose module token is the publisher authority.
    pub subject: String,
    pub body: Value,
    /// Observed at publish time against the runner's actual durability point,
    /// not inferred from a successful read made after publication.
    pub record_durable_at_publish: bool,
}

impl PublishedStep {
    pub fn new(subject: impl Into<String>, body: Value, record_durable_at_publish: bool) -> Self {
        Self {
            subject: subject.into(),
            body,
            record_durable_at_publish,
        }
    }
}

/// A settled fixture snapshot: all its native step records and captured sends.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct StepEventObservation {
    pub recorded: Vec<RecordedStep>,
    pub published: Vec<PublishedStep>,
}

impl StepEventObservation {
    pub fn new(recorded: Vec<RecordedStep>, published: Vec<PublishedStep>) -> Self {
        Self {
            recorded,
            published,
        }
    }
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "step-events",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("step-one").0]),
                    ScriptedTurn::new(vec![case.mint.text("step-two").0]),
                ],
            },
        )
        .await?;
    let raw = expect_ok(
        "role.describe",
        call(&session.route, ops::ROLE_DESCRIBE, serde_json::json!({})).await?,
    )?;
    let policy = check_describe(&raw)
        .map_err(|e| format!("role.describe: {e:?}"))?
        .step_events
        .ok_or("role.describe omitted step_events")?;
    let mut runs = Vec::new();
    for _ in 0..2 {
        let prompt = case.mint.next("step-prompt");
        runs.push(case.run_prompt(&mut session, &prompt).await?.1.run_id);
    }
    let observed = case
        .subject
        .step_event_observation(case.handle.inner(), &session.name)
        .await
        .map_err(|e| format!("step event evidence unavailable: {e}"))?;
    if runs.iter().any(|run| {
        !observed
            .recorded
            .iter()
            .any(|record| &record.run_id == run && record.completes_step)
    }) {
        return Err("native-history evidence omitted a fixture run's completed step".into());
    }
    let expected: Vec<_> = observed
        .recorded
        .iter()
        .filter(|r| policy.granularity == "per_attempt" || r.completes_step)
        .collect();
    let events: Vec<StepFinished> = observed
        .published
        .iter()
        .map(|p| {
            decode_step_finished(
                &p.subject,
                &serde_json::to_vec(&p.body).expect("a captured JSON body serializes"),
            )
            .map_err(|e| format!("step.finished.v1: {e}"))
        })
        .collect::<Result<_, _>>()?;
    if events.is_empty() {
        return Err("no step events were published".into());
    }
    match name {
        "step_events_one_per_recorded_step" => {
            if events.len() != expected.len()
                || expected.iter().any(|r| {
                    events
                        .iter()
                        .filter(|e| {
                            e.run_id == r.run_id && e.step_id == r.step_id && e.attempt == r.attempt
                        })
                        .count()
                        != 1
                })
            {
                return Err(format!(
                    "expected one event per recorded step at {}, got {} events for {} records",
                    policy.granularity,
                    events.len(),
                    expected.len()
                ));
            }
        }
        "step_events_match_durable_position" => {
            for event in &events {
                let record = expected
                    .iter()
                    .find(|r| {
                        r.run_id == event.run_id
                            && r.step_id == event.step_id
                            && r.attempt == event.attempt
                    })
                    .ok_or("event has no matching native step record")?;
                if event.step_ref() != record.position {
                    return Err(format!(
                        "event step_ref {:?} differs from native durable position {:?}",
                        event.step_ref(),
                        record.position
                    ));
                }
                if (policy.history_key == "step_ref" && event.harness_step_ref.is_some())
                    || (policy.history_key == "harness"
                        && event.harness_step_ref != record.harness_step_ref)
                {
                    return Err("harness_step_ref disagrees with the declared history key and native record".into());
                }
            }
        }
        "step_events_after_durability" => {
            if observed
                .published
                .iter()
                .any(|p| !p.record_durable_at_publish)
            {
                return Err("step event published before its record was durable".into());
            }
        }
        _ => return Err(format!("unknown step-event case {name}")),
    }
    Ok(Ending::Passed)
}
