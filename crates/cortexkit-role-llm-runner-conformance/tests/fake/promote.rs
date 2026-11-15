//! Promotion decisions and queue moves share one durable fake-log record.

use super::*;
use cortexkit_role_llm_runner::promote::{check_request, PromoteReason, PromoteReply};

/// Faults affect only their named experiment so sibling checks still exercise
/// the faithful implementation during a complete suite run.
fn fault(configured: Option<&'static str>, session: &str) -> Option<&'static str> {
    let defect = configured?;
    let case = match defect {
        "no_delivery" => "send_promote_active_next_step",
        "ended_first" => "send_promote_turn_ended_first",
        "drop_at_end" => "send_promote_run_ends_before_step",
        "parked" => "send_promote_parked",
        "foreign" | "unknown" => "send_promote_sender_namespace",
        "admission" => "send_promote_steer_admission",
        "replay" | "reuse" | "volatile" => "send_promote_request_replay",
        "unsupported" => "send_promote_unsupported",
        _ => return None,
    };
    session.contains(case).then_some(defect)
}

pub(super) fn apply(sess: &mut Sess, record: &Value, defect: Option<&'static str>) {
    let defect = fault(defect, record["session"].as_str().unwrap());
    sess.promotions
        .insert(record["id"].as_str().unwrap().into(), record.clone());
    if record["reply"]["outcome"] == "promoted" {
        if let Some(queued) = record["queued_id"]
            .as_str()
            .and_then(|id| sess.ordered.get_mut(id))
        {
            queued.modes = match defect {
                Some("no_delivery") => vec![Mode::IdleStart],
                Some("drop_at_end") => vec![Mode::Steer],
                _ => vec![Mode::Steer, Mode::IdleStart],
            };
        }
    }
}

/// Promotion experiments use the ordered queue to model a string `queue`
/// delivery. Other send fixtures retain their separate admission and failure
/// simulators. Keep the original request unchanged for retries after promotion.
pub(super) fn queue(
    module: &Module,
    stamp: &RouteStamp,
    session: &str,
    request: &SendRequest,
) -> Result<Option<Reply>, Killed> {
    if !module.world.serves(groups::SEND_PROMOTE)
        || !session.contains("send_promote_")
        || request.delivery_order.is_some()
    {
        return Ok(None);
    }
    let existing = module.sessions.lock().unwrap().get(session).cloned();
    let Some(sess) = existing else {
        return Ok(None);
    };
    let id = format!("{} {}", stamp.principal, request.send_id);
    if sess.ordered.contains_key(&id)
        || (request.delivery() == Delivery::Queue && !sess.sends.contains_key(&request.send_id))
    {
        return module
            .ordered_send(
                stamp,
                session,
                request,
                DeliveryOrder::new(vec![Mode::IdleStart]),
            )
            .map(Some);
    }
    Ok(None)
}

pub(super) fn handle(
    module: &Module,
    stamp: &RouteStamp,
    session: &str,
    params: &Value,
) -> Result<Reply, Killed> {
    let fault = fault(module.world.defects.promote, session);
    if fault == Some("unsupported") {
        return Ok(respond(PromoteReply::Promoted));
    }
    if (!module.world.serves(groups::SEND_PROMOTE) || !module.world.serves(groups::STEER))
        && fault != Some("unsupported")
    {
        return Ok(refuse(errors::UNSUPPORTED, None));
    }
    let request = match params
        .as_object()
        .ok_or_else(|| "send_id".to_owned())
        .and_then(check_request)
    {
        Ok(request) => request,
        Err(field) => return Ok(invalid(&field)),
    };
    let Some(sess) = module.sessions.lock().unwrap().get(session).cloned() else {
        return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
    };
    if order::admit_delivery(
        sender_module(&stamp.principal),
        Some(sender_module(&sess.owner)),
        &sess.senders,
        Delivery::Steer,
    )
    .is_err()
        && fault != Some("admission")
    {
        return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
    }
    let id = format!("{} {}", stamp.principal, request.request_id);
    if let Some(record) = sess.promotions.get(&id) {
        if record["send_id"] != request.send_id && fault != Some("reuse") {
            return Ok(refuse(
                errors::REQUEST_ID_REUSE,
                Some(json!({"field":"send_id"})),
            ));
        }
        if fault != Some("replay")
            && !(fault == Some("volatile") && module.replayed.load(Ordering::SeqCst))
        {
            return Ok(respond(&record["reply"]));
        }
    }
    let own = format!("{} {}", stamp.principal, request.send_id);
    let predecessor = cortexkit_role_llm_runner::sender_succession::predecessor(
        &sess.senders,
        sender_module(&stamp.principal),
    )
    .map(|p| format!("{p} {}", request.send_id));
    let queued_id = predecessor
        .as_ref()
        .filter(|key| sess.ordered.contains_key(*key))
        .cloned()
        .or_else(|| sess.ordered.contains_key(&own).then_some(own.clone()))
        .or_else(|| {
            (fault == Some("foreign"))
                .then(|| {
                    sess.ordered
                        .iter()
                        .find(|(_, q)| q.send_id == request.send_id)
                        .map(|(key, _)| key.clone())
                })
                .flatten()
        });
    let mut reply = PromoteReply::NotQueued;
    if let Some(queued) = queued_id.as_ref().map(|id| &sess.ordered[id]) {
        reply = match queued.state.as_str() {
            "delivered" => PromoteReply::Delivered,
            "pending" if sess.runs.iter().any(|run| run.state == "active") => {
                PromoteReply::Promoted
            }
            "pending" => PromoteReply::NotPromoted {
                reason: PromoteReason::NoActiveRun,
            },
            _ => PromoteReply::NotQueued,
        };
    } else if let Some(record) = sess.pre_delivery.get(&own) {
        if record["reply"]["state"] == "pending" {
            reply = PromoteReply::NotPromoted {
                reason: PromoteReason::NoActiveRun,
            };
        }
    } else if stamp.principal == sess.owner && sess.sends.contains_key(&request.send_id) {
        reply = PromoteReply::Delivered;
    }
    match fault {
        Some("ended_first") if reply == PromoteReply::Delivered => reply = PromoteReply::Promoted,
        Some("parked") if matches!(reply, PromoteReply::NotPromoted { .. }) => {
            reply = PromoteReply::Promoted
        }
        Some("unknown") if reply == PromoteReply::NotQueued => reply = PromoteReply::Promoted,
        Some("unsupported") => reply = PromoteReply::Promoted,
        _ => {}
    }
    module.commit(
        session,
        json!({"kind":"promote", "id":id, "send_id":request.send_id,
        "reply":reply, "queued_id":queued_id}),
    )?;
    Ok(respond(reply))
}
