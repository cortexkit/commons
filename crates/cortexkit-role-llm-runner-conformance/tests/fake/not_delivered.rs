//! Durable sends stopped before host hand-over, plus an idle queue fixture.

use super::*;

pub(super) fn intercept(
    module: &Module,
    stamp: &RouteStamp,
    session: &str,
    params: &Value,
) -> Result<Option<Reply>, Killed> {
    let Some(send_id) = params["send_id"].as_str() else {
        return Ok(None);
    };
    let id = format!("{} {send_id}", stamp.principal);
    let earlier = module
        .sessions
        .lock()
        .unwrap()
        .get(session)
        .and_then(|sess| sess.pre_delivery.get(&id))
        .cloned();
    if let Some(record) = earlier {
        if module.world.defects.not_delivered == Some("redeliver")
            && record["reply"]["state"] == "not_delivered"
        {
            return Ok(None);
        }
        let mut reply = record["reply"].clone();
        if module.world.defects.not_delivered == Some("reason_changes") {
            reply["reason"] = json!("another_reason");
        }
        return Ok(Some(respond(reply)));
    }
    let reason = {
        let mut next = module.world.pre_delivery_next.lock().unwrap();
        match next.get(session).cloned() {
            Some(reason) if reason == "parked" => Some(reason),
            _ => next.remove(session),
        }
    };
    let Some(reason) = reason else {
        return Ok(None);
    };
    let mut reply = if reason == "parked" {
        json!({"state":"pending", "submission_id":send_id})
    } else {
        json!({"state":"not_delivered", "reason":reason})
    };
    if reason == "handover_failed" && module.world.serves(groups::RUN_OPS) {
        reply["run_id"] = json!(format!("failed-{send_id}"));
    }
    if reason == "parked" && module.world.defects.not_delivered == Some("queue_failed") {
        reply = json!({"state":"not_delivered", "reason":"handover_failed"});
    }
    if reason == "parked" && module.world.defects.not_delivered == Some("pending_run_id") {
        reply["run_id"] = json!("run-2");
    }
    if module.world.defects.not_delivered == Some("receipt") && reason != "parked" {
        reply["delivered"] = json!({"as":"unknown"});
    }
    module.commit(
        session,
        json!({
            "kind":"pre_delivery", "id":id, "request":params,
            "principal":stamp.principal, "reply":reply
        }),
    )?;
    Ok(Some(respond(reply)))
}

pub(super) fn run_result(module: &Module, session: &str, params: &Value) -> Option<Reply> {
    let sessions = module.sessions.lock().unwrap();
    let record = sessions
        .get(session)?
        .pre_delivery
        .values()
        .find(|record| {
            record["reply"]["run_id"].is_string() && record["reply"]["run_id"] == params["run_id"]
        })?;
    let run_id = record["reply"]["run_id"].as_str().unwrap();
    let mut result = RunResult::new(run_id, "error");
    if module.world.defects.not_delivered == Some("run_not_error") {
        result.state = "cancelled".into();
    }
    if module.world.defects.not_delivered != Some("run_no_refusal") {
        result = result.with_error(cortexkit_role_llm_runner::run::RunError::new(
            "runner",
            "the host could not be relaunched",
            "host_relaunch_failed",
        ));
    }
    Some(respond(result))
}

pub(super) fn release_parked(subject: &FakeSubject, session: &str) -> Result<(), HarnessError> {
    subject
        .world()
        .pre_delivery_next
        .lock()
        .unwrap()
        .remove(session);
    let module = subject
        .modules
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|module| module.alive.load(Ordering::SeqCst))
        .cloned()
        .ok_or_else(|| HarnessError::new("no running module"))?;
    let records: Vec<_> = module
        .sessions
        .lock()
        .unwrap()
        .get(session)
        .unwrap()
        .pre_delivery
        .iter()
        .filter(|(_, record)| record["reply"]["state"] == "pending")
        .map(|(id, record)| (id.clone(), record.clone()))
        .collect();
    for (id, record) in records {
        module
            .commit(session, json!({"kind":"pre_delivery_forget", "id":id}))
            .map_err(|_| HarnessError::new("killed while draining parked queue"))?;
        let mut stamp = subject.owner_stamp();
        stamp.principal = record["principal"].as_str().unwrap().into();
        module
            .send(&stamp, session, record["request"].clone())
            .map_err(|_| HarnessError::new("killed while delivering parked send"))?;
    }
    Ok(())
}
