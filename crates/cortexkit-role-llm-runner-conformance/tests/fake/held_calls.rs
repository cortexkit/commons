use super::*;
use cortexkit_role_llm_runner::run::HeldCall;
use cortexkit_role_llm_runner_conformance::ScriptedLateEntry;

pub(super) struct HeldReply {
    reason: String,
    call_key: Option<String>,
    at: u64,
    late: Option<ScriptedLateEntry>,
}

impl World {
    pub(super) fn held_first_reply(&self, arguments: &Value, call_key: &str) -> bool {
        let mut replies = self.held_replies.lock().unwrap();
        let Some(reply) = replies.get_mut(&arguments.to_string()) else {
            return false;
        };
        reply.call_key = Some(call_key.into());
        reply.at = self.now();
        *self
            .invocations
            .lock()
            .unwrap()
            .entry(arguments.to_string())
            .or_default() += 1;
        true
    }
}

impl Module {
    pub(super) fn record_held_status(
        &self,
        session: &str,
        run_id: &str,
        mut held: Vec<HeldCall>,
    ) -> Result<(), Killed> {
        if !self.world.serves(groups::HELD_CALLS) {
            return Ok(());
        }
        let defects = &self.world.defects;
        if defects.held_calls_event_missing {
            held.clear();
        } else if defects.held_calls_event_wrong_key {
            held = held
                .iter()
                .map(|entry| {
                    HeldCall::new(
                        format!("{}-wrong", entry.call_key()),
                        entry.tool(),
                        entry.provider(),
                        entry.reason(),
                        entry.held_at_ms(),
                    )
                })
                .collect();
        }
        let mut frame = cortexkit_role_llm_runner::subscribe::SubscribeEvent::new("control")
            .with_held_calls(held);
        frame.body.insert("run_id".into(), json!(run_id));
        frame.body.insert("state".into(), json!("active"));
        if defects.held_calls_event_empty && !frame.body.contains_key("held_calls") {
            frame.body.insert("held_calls".into(), json!([]));
        }
        self.commit(session, json!({"kind":"held_status", "body":frame.body}))
    }

    pub(super) fn held_status(&self, session: &str, run_id: &str) -> Vec<HeldCall> {
        if self.world.defects.held_calls_never_listed {
            return vec![];
        }
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            return vec![];
        };
        let replies = self.world.held_replies.lock().unwrap();
        sess.calls
            .iter()
            .filter_map(|(key, call)| {
                if call.run_id != run_id || !call.intent {
                    return None;
                }
                let reply = replies.get(&call.arguments.to_string())?;
                let stale = self.world.defects.held_calls_stale
                    && reply
                        .late
                        .as_ref()
                        .is_some_and(|entry| entry.kind() == "result");
                if call.result && !stale {
                    return None;
                }
                Some(HeldCall::new(
                    key,
                    SCRIPTED_TOOL,
                    TOOL_MODULE,
                    &reply.reason,
                    reply.at,
                ))
            })
            .collect()
    }

    fn pull_late_entry(&self, arguments: &Value) -> Result<(), Killed> {
        let entry = self
            .world
            .held_replies
            .lock()
            .unwrap()
            .get(&arguments.to_string())
            .and_then(|reply| reply.late.clone());
        let Some(entry) = entry else {
            return Ok(());
        };
        let found = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find_map(|(session, sess)| {
                sess.calls
                    .iter()
                    .find(|(_, call)| &call.arguments == arguments && call.intent && !call.result)
                    .map(|(key, call)| (session.clone(), key.clone(), call.run_id.clone()))
            });
        let Some((session, key, run_id)) = found else {
            return Ok(());
        };
        let content = match entry.kind() {
            "result" => entry.result_body().cloned().unwrap(),
            "not_started" => json!({"text": "The call never ran.", "reason": entry.reason()}),
            "expired" => json!({"text": "The call's answer expired."}),
            _ => return Ok(()),
        };
        let held = self.held_status(&session, &run_id);
        self.commit(
            &session,
            json!({"kind":"result", "call_key":key, "content":content}),
        )?;
        self.record_held_status(&session, &run_id, Vec::new())?;
        if self.world.defects.held_calls_event_stale {
            // Relist even after a clear, so stopping at the first omitted
            // list would hide this contract violation.
            self.record_held_status(&session, &run_id, held)?;
        }
        self.drive(&session, &run_id)
    }
}

impl FakeSubject {
    pub(super) fn install_held(&self, arguments: &Value, reason: &str) {
        self.world().held_replies.lock().unwrap().insert(
            arguments.to_string(),
            HeldReply {
                reason: reason.into(),
                call_key: None,
                at: 0,
                late: None,
            },
        );
    }

    pub(super) fn observed_held(&self, arguments: &Value) -> Result<String, HarnessError> {
        self.world()
            .held_replies
            .lock()
            .unwrap()
            .get(&arguments.to_string())
            .and_then(|reply| reply.call_key.clone())
            .ok_or_else(|| HarnessError::new("no held reply was sent"))
    }

    pub(super) fn publish_late(
        &self,
        arguments: &Value,
        entry: ScriptedLateEntry,
    ) -> Result<(), HarnessError> {
        let world = self.world();
        let mut replies = world.held_replies.lock().unwrap();
        let reply = replies
            .get_mut(&arguments.to_string())
            .ok_or_else(|| HarnessError::new("no held call"))?;
        reply.late = Some(entry);
        drop(replies);
        for module in self.modules.lock().unwrap().clone() {
            if module.alive.load(Ordering::SeqCst) {
                module
                    .pull_late_entry(arguments)
                    .map_err(|_| HarnessError::new("runner killed during intake"))?;
            }
        }
        Ok(())
    }
}
