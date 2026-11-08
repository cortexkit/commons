//! The optional host-start op uses one durable head record and no transcript.

use super::*;
use cortexkit_role_llm_runner::host_start::{HostStartReply, HostStartRequest};

pub(super) fn apply(sess: &mut Sess, record: &Value) {
    if let Some(request) = record.get("host_start_request") {
        let id = request["send_id"].as_str().unwrap().to_owned();
        sess.host_starts
            .insert(id, (request.clone(), record["host_start_reply"].clone()));
    }
}

pub(super) fn handle(
    module: &Module,
    stamp: &RouteStamp,
    session: &str,
    params: Value,
) -> Result<Reply, Killed> {
    module
        .host_start_attempts
        .lock()
        .unwrap()
        .insert(session.into());
    let defect = module.world.defects.host_start;
    if let Some(field) = unknown_field(&params, &["send_id", "plan", "model"]) {
        if defect == Some("invalid_params_leaves_nothing") {
            module.commit(
                session,
                json!({"kind":"start", "owner":stamp.principal,
                "lineage":format!("lin-{session}"), "plan_digest":"poisoned"}),
            )?;
        }
        return Ok(invalid(&field));
    }
    let request: HostStartRequest = match serde_json::from_value(params.clone()) {
        Ok(request) => request,
        Err(_) => return Ok(invalid("request")),
    };
    if !stamp.scope.as_ref().is_some_and(|scope| {
        scope.owner == stamp.principal
            && scope.scope_ref != "flow-scope"
            && scope.scope_ref != "run-scope"
    }) {
        return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
    }
    let sess = module.sessions.lock().unwrap().get(session).cloned();
    if let Some(sess) = &sess {
        if sess.owner != stamp.principal {
            return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
        }
        if let Some((earlier, reply)) = sess.host_starts.get(&request.send_id) {
            let earlier: HostStartRequest = serde_json::from_value(earlier.clone()).unwrap();
            return Ok(match request.reused_field(&earlier) {
                Some(_) if defect == Some("send_id_reuse_names_plan") => {
                    Reply::Response(reply.clone())
                }
                Some(field) => refuse(errors::SEND_ID_REUSE, Some(json!({"field":field}))),
                None => {
                    let mut reply = reply.clone();
                    if defect == Some("exact_repeat_replays") {
                        reply["applied_plan_digest"] = json!("not-the-stored-reply");
                    }
                    Reply::Response(reply)
                }
            });
        }
    }
    let sent = plan_digest(&request.plan);
    if let Some(sess) = &sess {
        if sess.plan_digest.as_deref() != Some(&sent) && defect != Some("new_id_plan_drift") {
            return Ok(refuse(
                errors::PLAN_DRIFT,
                Some(json!({"frozen":sess.plan_digest,"sent":sent})),
            ));
        }
    }
    let applied = sess
        .as_ref()
        .and_then(|s| s.plan_digest.as_deref())
        .unwrap_or(&sent);
    let reply = serde_json::to_value(HostStartReply::new(applied)).unwrap();
    let mut record = json!({"kind":"host_start_reply", "host_start_request":params,
        "host_start_reply":reply});
    if sess.is_none() {
        record["kind"] = json!("start");
        record["owner"] = json!(stamp.principal);
        record["lineage"] = json!(format!("lin-{session}"));
        record["plan_digest"] = json!(sent);
        record["tools"] = request.plan.get("tools").cloned().unwrap_or(json!([]));
        record["senders"] = request.plan.get("senders").cloned().unwrap_or(Value::Null);
        record["readers"] = request.plan.get("readers").cloned().unwrap_or(Value::Null);
        record["selection"] = json!(request.model);
    }
    module.commit(session, record)?;
    Ok(Reply::Response(reply))
}
