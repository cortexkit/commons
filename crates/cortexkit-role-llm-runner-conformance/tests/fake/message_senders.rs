//! Observe each message start immediately after its record is applied, before later work.

use std::{collections::BTreeSet, future::Future, pin::Pin, time::Duration};

use cortexkit_role_llm_runner::{capabilities as groups, ops};
use cortexkit_role_llm_runner_conformance::{Reply, RouteFailure, SubscribeOutcome};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

use super::{FakeRoute, Module, RouteStamp};

pub(super) struct Observer {
    session: String,
    stamp: RouteStamp,
    seen: BTreeSet<String>,
    rows: Vec<(Value, Reply)>,
    bindings: Vec<JoinHandle<()>>,
}

pub(super) async fn observe(
    route: &FakeRoute,
    operation: Pin<&mut (dyn Future<Output = Result<Reply, RouteFailure>> + Send)>,
) -> Result<Option<(Reply, Vec<(Value, Reply)>)>, RouteFailure> {
    let defects = &route.module.world.defects;
    if !route.module.world.serves(groups::MESSAGE_SENDERS) || defects.message_senders_no_observer {
        return Ok(None);
    }
    let seen = route.module.sessions.lock().unwrap()[&route.session]
        .messages
        .iter()
        .map(|m| m.mid.clone())
        .collect();
    *route.module.sender_observer.lock().unwrap() = Some(Observer {
        session: route.session.clone(),
        stamp: route.stamp.clone(),
        seen,
        rows: Vec::new(),
        bindings: Vec::new(),
    });
    let reply = operation.await;
    let observer = route.module.sender_observer.lock().unwrap().take().unwrap();
    // Quiet-window checks see reconciled rows; the captured reads cannot be
    // repaired when the asynchronous binding arrives a few milliseconds later.
    for binding in observer.bindings {
        binding.await.unwrap();
    }
    Ok(Some((reply?, observer.rows)))
}

pub(super) fn capture(module: &Module, session: &str) {
    let Some(mut observer) = module.sender_observer.lock().unwrap().take() else {
        return;
    };
    if observer.session == session {
        let SubscribeOutcome::Events(frames) = module.subscribe(session, json!({"from":"start"}))
        else {
            panic!("live observer subscription refused");
        };
        for frame in frames {
            if frame["event"]["type"] != "message_started" {
                continue;
            }
            let mid = frame["event"]["mid"].as_str().unwrap().to_owned();
            if !observer.seen.insert(mid.clone()) {
                continue;
            }
            let late = {
                let mut sessions = module.sessions.lock().unwrap();
                let message = sessions
                    .get_mut(session)
                    .unwrap()
                    .messages
                    .iter_mut()
                    .find(|m| m.mid == mid)
                    .unwrap();
                let affected = !module.world.defects.message_senders_join_only
                    || message
                        .delivered_as
                        .as_ref()
                        .is_some_and(|mode| mode["mode"] == "steer");
                if module.world.defects.message_senders_late && affected {
                    message.sender.take()
                } else {
                    None
                }
            };
            if let Some(sender) = late {
                // The emitted message start precedes host reconciliation. This
                // timer is independent of model holds and their release.
                let sessions = module.sessions.clone();
                let session = session.to_owned();
                let mid = mid.clone();
                observer.bindings.push(tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    sessions
                        .lock()
                        .unwrap()
                        .get_mut(&session)
                        .unwrap()
                        .messages
                        .iter_mut()
                        .find(|m| m.mid == mid)
                        .unwrap()
                        .sender = Some(sender);
                }));
            }
            let captured = module
                .handle(
                    &observer.stamp,
                    session,
                    ops::SESSION_HISTORY,
                    json!({"from_mid":mid,"limit":1,"form":"display"}),
                )
                .unwrap();
            observer.rows.push((frame, captured));
        }
    }
    *module.sender_observer.lock().unwrap() = Some(observer);
}
