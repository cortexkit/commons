//! A scheduler and a provider journal for the controlled cache fixture.

use super::*;
use cortexkit_role_llm_runner_conformance::{
    KeepWarmPrices, KeepWarmRequest, KeepWarmRequests, ScriptedUsage,
};

pub(super) struct State {
    prices: KeepWarmPrices,
    mode: String,
    defect: Option<&'static str>,
    journal: Vec<KeepWarmRequest>,
    last_real: Option<KeepWarmRequest>,
    next_at: Option<u64>,
    warms: u64,
    real_count: u64,
}

pub(super) fn configure(module: &Module, session: &str, prices: &KeepWarmPrices) -> KeepWarmPrices {
    let defect = module.world.defects.keep_warm.and_then(|(case, defect)| {
        (*module.world.current_case.lock().unwrap() == Some(case)).then_some(defect)
    });
    let applied = if defect == Some("prices") {
        KeepWarmPrices::new(1, 4)
    } else {
        prices.clone()
    };
    module.world.warm_sessions.lock().unwrap().insert(
        session.into(),
        State {
            prices: applied,
            mode: "off".into(),
            defect,
            journal: Vec::new(),
            last_real: None,
            next_at: None,
            warms: 0,
            real_count: 0,
        },
    );
    module.world.warm_sessions.lock().unwrap()[session]
        .prices
        .clone()
}

pub(super) fn admit(module: &Module, session: &str, params: &Value) {
    let mut states = module.world.warm_sessions.lock().unwrap();
    let Some(state) = states.get_mut(session) else {
        return;
    };
    if let Some(mode) = params
        .pointer("/plan/cache/keep_warm")
        .and_then(Value::as_str)
    {
        state.mode = mode.into();
    }
}

pub(super) fn real_request(
    module: &Module,
    session: &str,
    observation: &ModelRequestObservation,
    usage: Option<&ScriptedUsage>,
) {
    let mut states = module.world.warm_sessions.lock().unwrap();
    let Some(state) = states.get_mut(session) else {
        return;
    };
    let bytes = serde_json::to_vec(&json!({
        "messages": observation.rendered,
        "tools": observation.tools,
        "max_output_tokens": observation.max_output_tokens,
    }))
    .unwrap();
    let request = KeepWarmRequest::new(
        module.world.warm_now.load(Ordering::SeqCst),
        bytes.clone(),
        bytes,
        usage.unwrap().clone(),
    );
    state.journal.push(request.clone());
    state.last_real = Some(request.clone());
    if state.defect != Some("no_rearm") || state.real_count == 0 {
        state.warms = 0;
    }
    state.real_count += 1;
    let ttl = module.world.observed_cache_lifetimes.lock().unwrap()[session].unwrap();
    state.next_at = Some(request.at_ms + ttl / 2);
}

pub(super) fn advance(module: &Module, by: std::time::Duration) {
    let start = module.world.warm_now.load(Ordering::SeqCst);
    let end = start + u64::try_from(by.as_millis()).unwrap();
    let mut states = module.world.warm_sessions.lock().unwrap();
    for (session, state) in states.iter_mut() {
        let Some(last) = state.last_real.clone() else {
            continue;
        };
        let ttl = module.world.observed_cache_lifetimes.lock().unwrap()[session].unwrap();
        // The fake's scheduler uses its installed price row, independently of
        // the conformance checker. Faults change the scheduler, not the journal.
        let limit = match state.defect {
            Some("forever") => u64::MAX,
            Some("early") => 2,
            Some("late") => 4,
            _ => state.prices.cache_write.div_ceil(state.prices.cache_read),
        };
        while let Some(at) = state.next_at.filter(|at| *at <= end) {
            if state.defect == Some("never")
                || (state.mode == "off" && state.defect != Some("off"))
                || state.warms >= limit
            {
                state.next_at = None;
                break;
            }
            let mut request = last.clone();
            request.at_ms = at;
            if state.defect == Some("bytes") {
                request.request_bytes.push(b' ');
            }
            if state.defect == Some("prefix") {
                request.cache_prefix.push(b'x');
            }
            state.journal.push(request);
            state.warms += 1;
            state.next_at = Some(at + ttl / 2);
            if state.defect == Some("transcript") {
                module
                    .sessions
                    .lock()
                    .unwrap()
                    .get_mut(session)
                    .unwrap()
                    .messages
                    .first_mut()
                    .unwrap()
                    .body["text"] = json!("changed by a cache touch");
            }
            if state.defect == Some("step") {
                module
                    .sessions
                    .lock()
                    .unwrap()
                    .get_mut(session)
                    .unwrap()
                    .runs
                    .last_mut()
                    .unwrap()
                    .usage
                    .steps += 1;
            }
        }
    }
    module.world.warm_now.store(end, Ordering::SeqCst);
}

pub(super) fn requests(world: &World, session: &str, cursor: u64) -> KeepWarmRequests {
    let states = world.warm_sessions.lock().unwrap();
    let state = &states[session];
    KeepWarmRequests::new(
        world.warm_now.load(Ordering::SeqCst),
        state.journal.len() as u64,
        state.journal[cursor as usize..].to_vec(),
    )
}
