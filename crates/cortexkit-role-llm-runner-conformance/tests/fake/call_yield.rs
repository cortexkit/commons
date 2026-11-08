use super::*;
use cortexkit_role_llm_runner_conformance::CallYieldObservation;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    OnePollLate,
    NeverStored,
    QueueYields,
}

#[derive(Default)]
struct Pending {
    arguments: Value,
    delivery: String,
    polls: usize,
    requests: Vec<Value>,
    released: bool,
}

pub(super) struct State {
    mode: Option<Mode>,
    pending: Mutex<BTreeMap<String, Pending>>,
}

pub struct Snapshot {
    pub polls: usize,
    pub requests: usize,
    pub model_requests: usize,
    pub run_states: Vec<String>,
}

impl State {
    pub(super) fn new(mode: Option<Mode>) -> Self {
        Self {
            mode,
            pending: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn install(&self, session: &str, arguments: &Value) -> bool {
        if self.mode.is_none() {
            return false;
        }
        self.pending.lock().unwrap().insert(
            session.into(),
            Pending {
                arguments: arguments.clone(),
                ..Default::default()
            },
        );
        true
    }

    pub(super) fn observe(
        &self,
        module: &Module,
        session: &str,
        send_id: &str,
    ) -> Result<CallYieldObservation, HarnessError> {
        let mut pending = self.pending.lock().unwrap();
        let pending = pending
            .get_mut(session)
            .ok_or_else(|| HarnessError::new("no call-yield fixture"))?;
        pending.polls += 1;
        let sessions = module.sessions.lock().unwrap();
        let sess = &sessions[session];
        let call_key = sess
            .calls
            .iter()
            .find(|(_, call)| call.arguments == pending.arguments)
            .map(|(key, _)| key.clone())
            .ok_or_else(|| HarnessError::new("fixture call was not dispatched"))?;
        // The faithful fixture dispatches one observation after admission,
        // not in the staging poll. QueueYields deliberately dispatches early
        // for the wrong message. Releasing an answer cannot create a request.
        if (pending.polls > 1 || self.mode == Some(Mode::QueueYields))
            && !pending.released
            && pending.requests.is_empty()
            && (pending.delivery == "steer" || self.mode == Some(Mode::QueueYields))
        {
            pending
                .requests
                .push(json!({"name":"tool.yield","arguments":{
                    "call_key":call_key, "reason":"person_message"
                }}));
        }
        let mut observation = CallYieldObservation::default();
        // The shared fake is compiled inside and outside the library, so
        // fill the non-exhaustive observation through its open field pattern.
        let CallYieldObservation {
            call_key: observed_key,
            yield_requests,
            steer_stored_sequence,
            first_yield_answer_sequence,
            ..
        } = &mut observation;
        *observed_key = call_key;
        *yield_requests = pending.requests.clone();
        *steer_stored_sequence = (sess.sends.contains_key(send_id)
            || sess.ordered.values().any(|send| send.send_id == send_id))
        .then_some(1);
        *first_yield_answer_sequence = pending.released.then_some(2);
        Ok(observation)
    }

    pub(super) fn release(&self, arguments: &Value) {
        for pending in self.pending.lock().unwrap().values_mut() {
            if &pending.arguments == arguments {
                pending.released = true;
            }
        }
    }
}

pub(super) fn intercept(module: &Module, session: &str, params: &Value) -> Option<Reply> {
    let mut pending = module.world.call_yield.pending.lock().unwrap();
    let pending = pending.get_mut(session)?;
    let delivery = params.get("delivery")?.as_str()?;
    if !matches!(delivery, "steer" | "queue") {
        return None;
    }
    pending.delivery = delivery.into();
    if delivery == "steer" && module.world.call_yield.mode == Some(Mode::NeverStored) {
        // A deliberately incorrect runner acknowledges the steer without
        // committing it. Observation reads the real store, so storage stays absent.
        return Some(respond(
            json!({"send_id":params["send_id"],"delivered":"pending"}),
        ));
    }
    None
}

impl FakeSubject {
    pub fn call_yield_fixture_snapshot(&self) -> Snapshot {
        let world = self.world();
        let pending = world.call_yield.pending.lock().unwrap();
        let mut snapshot = Snapshot {
            polls: pending.values().map(|p| p.polls).sum(),
            requests: pending.values().map(|p| p.requests.len()).sum(),
            model_requests: 0,
            run_states: vec![],
        };
        for session in pending.keys() {
            snapshot.model_requests += world
                .model_requests
                .lock()
                .unwrap()
                .get(session)
                .map_or(0, Vec::len);
            for module in self.modules.lock().unwrap().iter() {
                if module.alive.load(Ordering::SeqCst) {
                    if let Some(sess) = module.sessions.lock().unwrap().get(session) {
                        snapshot
                            .run_states
                            .extend(sess.runs.iter().map(|r| r.state.clone()));
                    }
                }
            }
        }
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_yield_fixture_is_opt_in_and_empty_before_installation() {
        for mode in [
            None,
            Some(Mode::OnePollLate),
            Some(Mode::NeverStored),
            Some(Mode::QueueYields),
        ] {
            let mut fake = FakeSubject::new(Defects::default());
            fake.call_yield_fixture = mode;
            assert_eq!(fake.world().serves(groups::CALL_YIELD), mode.is_some());
            let snapshot: super::super::CallYieldSnapshot = fake.call_yield_fixture_snapshot();
            assert_eq!(snapshot.polls, 0);
            assert_eq!(snapshot.requests, 0);
            assert_eq!(snapshot.model_requests, 0);
            assert!(snapshot.run_states.is_empty());
        }
    }
}
