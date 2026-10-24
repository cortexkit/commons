//! Fake publication seam and an independent reader of its on-disk records.

use super::*;
use cortexkit_role_llm_runner::step_events::{StepFinished, StepRef};
use cortexkit_role_llm_runner_conformance::step_events::{
    PublishedStep, RecordedStep, StepEventObservation,
};

// Neutralizing these deliberate defects must make their self-tests go red.
const STEP_EVENT_DEFECTS_ENABLED: bool = true;

fn is_step(record: &Value) -> bool {
    matches!(record["kind"].as_str(), Some("step" | "step_retry"))
}

pub(super) fn prepare(module: &Module, session: &str, record: &mut Value) -> Result<(), Killed> {
    if !module.world.serves(groups::STEP_EVENTS) || !is_step(record) {
        return Ok(());
    }
    if record["kind"] == "step" {
        let step = module.sessions.lock().unwrap()[session]
            .run(record["run_id"].as_str().unwrap())
            .unwrap()
            .usage
            .steps;
        record["native_step_id"] = json!(format!("step-{step}"));
        // Model one hidden retry before the final attempt, without changing
        // the fake's transcript or aggregate usage.
        module.commit(session, json!({"kind": "step_retry", "run_id": record["run_id"], "native_step_id": record["native_step_id"], "native_attempt": 1}))?;
        record["native_attempt"] = json!(2);
    }
    record["native_seq"] = json!(std::fs::metadata(&module.log).map(|m| m.len()).unwrap_or(0));
    record["native_incarnation"] = json!(module.sessions.lock().unwrap()[session].incarnation);
    record["native_session_ref"] = json!(cortexkit_role_llm_runner::session_ref(
        &cortexkit_role_llm_runner::SessionIdentity {
            runner: FAKE_RUNNER,
            harness: FAKE_HARNESS,
            session,
        }
    ));
    Ok(())
}

pub(super) fn publish(module: &Module, session: &str, record: &Value, durable: bool) {
    if !module.world.serves(groups::STEP_EVENTS) || !is_step(record) {
        return;
    }
    if record["kind"] == "step_retry"
        && module.world.step_event_policy.granularity == "per_completed_step"
    {
        return;
    }
    let defects = &module.world.defects;
    let early = STEP_EVENT_DEFECTS_ENABLED && defects.step_events_before_durability;
    if durable == early {
        return;
    }
    let mut seq = record["native_seq"].as_u64().unwrap();
    if STEP_EVENT_DEFECTS_ENABLED && defects.step_events_wrong_position {
        seq += 1;
    }
    let position = StepRef::new(
        FAKE_RUNNER,
        record["native_session_ref"].as_str().unwrap(),
        record["native_incarnation"].as_str().unwrap(),
        seq,
    );
    let mut event = StepFinished::new(
        position,
        record["run_id"].as_str().unwrap(),
        record["native_step_id"].as_str().unwrap(),
        std::num::NonZeroU64::new(record["native_attempt"].as_u64().unwrap()).unwrap(),
        if record["kind"] == "step_retry" {
            "failed"
        } else {
            "completed"
        },
        FAKE_HARNESS,
        session,
        FAKE_PROVIDER,
        FAKE_MODEL,
        10,
        20,
    );
    if module.world.step_event_policy.history_key == "harness" && record["kind"] == "step" {
        event = event.with_harness_step_ref(format!(
            "host-{}",
            record["native_step_id"].as_str().unwrap()
        ));
    }
    let mut body = serde_json::to_value(event).unwrap();
    if let Some(member) = defects.step_events_content_member {
        body[member] = json!("forbidden-content");
    }
    let publisher = if defects.step_events_wrong_publisher {
        "runner-b"
    } else {
        FAKE_RUNNER
    };
    let subject = format!("ck.account_a.live.{publisher}.step.finished.v1");
    let publication = PublishedStep::new(subject, body, durable);
    let mut all = module.world.step_publications.lock().unwrap();
    let events = all.entry(session.into()).or_default();
    events.push(publication.clone());
    if STEP_EVENT_DEFECTS_ENABLED && defects.step_events_duplicate {
        events.push(publication);
    }
}

pub(super) fn observe(
    module: &Module,
    session: &str,
) -> Result<StepEventObservation, HarnessError> {
    if module.world.defects.step_events_no_evidence {
        return Err(HarnessError::new(
            "native step-event observations are not implemented",
        ));
    }
    let log = std::fs::read_to_string(&module.log).map_err(|e| HarnessError::new(e.to_string()))?;
    let mut recorded = Vec::new();
    let mut offset = 0;
    for line in log.split_inclusive('\n') {
        let record: Value = serde_json::from_str(line).unwrap();
        if is_step(&record) && record["session"] == session {
            // Native position is the record's byte offset, independently read
            // from disk, not the seq the publishing code put in its event.
            let position = StepRef::new(
                FAKE_RUNNER,
                record["native_session_ref"].as_str().unwrap(),
                record["native_incarnation"].as_str().unwrap(),
                offset,
            );
            let mut step = RecordedStep::new(
                position,
                record["run_id"].as_str().unwrap(),
                record["native_step_id"].as_str().unwrap(),
                std::num::NonZeroU64::new(record["native_attempt"].as_u64().unwrap()).unwrap(),
                record["kind"] == "step",
            );
            if record["kind"] == "step" {
                step = step.with_harness_step_ref(format!(
                    "host-{}",
                    record["native_step_id"].as_str().unwrap()
                ));
            }
            recorded.push(step);
        }
        offset += line.len() as u64;
    }
    let published = module
        .world
        .step_publications
        .lock()
        .unwrap()
        .get(session)
        .cloned()
        .unwrap_or_default();
    Ok(StepEventObservation::new(recorded, published))
}
