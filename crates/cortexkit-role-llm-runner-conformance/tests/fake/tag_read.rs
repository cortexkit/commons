use super::*;
use cortexkit_role_llm_runner::{
    read::{TagReadAnswer, TagReadRequest},
    scope_admission::ScopeKind,
};

pub(super) fn snapshot(module: &Module, sess: &Sess) -> BTreeMap<u64, TagReadAnswer> {
    let numbers = part_numbers(sess);
    let mut result = BTreeMap::new();
    for message in sess
        .messages
        .iter()
        .filter(|message| message.ordinal >= sess.first_ordinal)
    {
        let mut entry = module.read_message(sess, message);
        entry.client_message_id = message.client_message_id.clone();
        entry.created_at = module
            .world
            .host_created_at
            .lock()
            .unwrap()
            .get(&(sess.name.clone(), message.mid.clone()))
            .copied();
        entry.completed_at = message.completed_at;
        entry.notices = message.notices.clone();
        for (index, (key, _)) in media::parts(&message.body).into_iter().enumerate() {
            if let Some(number) = numbers.get(&(message.mid.clone(), key)) {
                result.insert(*number, TagReadAnswer::new(entry.clone(), index as u32));
            }
        }
    }
    result
}

pub(super) fn read(module: &Module, stamp: &RouteStamp, session: &str, params: &Value) -> Reply {
    let defect = module.world.defects.tag_read;
    if !module.world.serves(groups::TAG_READ) {
        return if defect == Some("undeclared") {
            refuse(errors::UNKNOWN_CALL, None)
        } else {
            invalid("tag")
        };
    }
    let request = match TagReadRequest::decode(params) {
        Ok(request) => request,
        Err(field) => return invalid(&field),
    };
    let kind = match stamp.scope.as_ref().map(|scope| scope.scope_ref.as_str()) {
        Some("flow-scope") => ScopeKind::Flow { flow_id: "flow" },
        Some("run-scope") => ScopeKind::Run { run_id: "run" },
        _ => ScopeKind::Plain,
    };
    if defect != Some("owner") {
        if let Err(code) = request.check_owner(
            stamp.scope.as_ref().map(|scope| scope.owner.as_str()),
            &stamp.principal,
            kind,
        ) {
            return refuse(code, None);
        }
    }
    let sessions = module.sessions.lock().unwrap();
    let Some(sess) = sessions.get(session) else {
        return refuse(errors::UNKNOWN_CALL, None);
    };
    if defect != Some("owner") && sess.owner != stamp.principal {
        return refuse(errors::SCOPE_OWNER_MISMATCH, None);
    }
    let Some(saved) = sess.tag_read_requests.get(&request.call_key) else {
        return refuse(
            if defect == Some("unknown_call") {
                errors::UNKNOWN_TAG
            } else {
                errors::UNKNOWN_CALL
            },
            None,
        );
    };
    if defect != Some("removed_call")
        && !sess
            .archive
            .iter()
            .chain(&sess.messages)
            .any(|message| message.calls.contains(&request.call_key))
    {
        return refuse(errors::UNKNOWN_CALL, None);
    }
    if defect == Some("cut") && session.contains("tag_read_before_cut") && !sess.cuts.is_empty() {
        return refuse(errors::UNKNOWN_TAG, None);
    }
    let Some(mut answer) = saved.get(&request.tag).cloned() else {
        return if defect == Some("unknown_tag") {
            Reply::Response(json!({}))
        } else {
            refuse(errors::UNKNOWN_TAG, None)
        };
    };
    if defect == Some("part") && session.contains("tag_read_person") {
        answer.part = 0;
    }
    if defect == Some("client_id") && session.contains("tag_read_person") {
        answer.entry.send_id = answer.entry.client_message_id.take();
    }
    Reply::Response(serde_json::to_value(answer).unwrap())
}
