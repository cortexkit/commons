//! Opt-in host-preserving restart experiments and independent observations.
use crate::{
    cases::{Case, Ending},
    route::RunnerRoute,
    subject::LlmRunnerSubject,
};
use cortexkit_role_llm_runner::{
    capabilities,
    read::{HeadMeta, HostState},
    run::{interrupt_reasons, RunResult, RunState},
    send::SendReply,
};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

/// Where an adapter must observe the real in-flight operation before restarting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RestartPoint {
    ModelStep,
    HeldToolCall,
    SendHandoff,
    UnknownHost,
    Recovering,
}

/// Both ways of restarting must preserve a host that remains alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RestartMode {
    Crash,
    Orderly,
}

/// Captures of one experiment, from actual management routes and independent
/// host/provider recorders. Event payloads are unmodified JSON; stream frames
/// retain `seq`, `cursor`, `kind` and `event`. Counts must live outside the
/// runner's process and state root, and include retries after restart.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct RestartSurvivalObservation {
    /// Native process identity (pid and start time), independently observed
    /// before and after. A changed runner process and the same surviving host
    /// are required; generated experiment labels are not process evidence.
    pub runner_process_before: Option<String>,
    pub runner_process_after: Option<String>,
    pub host_process_before: Option<String>,
    pub host_process_after: Option<String>,
    pub before: Option<RunResult>,
    /// Read before releasing the held model/provider after restart.
    pub recovering: Option<RunResult>,
    pub ended: Option<RunResult>,
    pub head_before: Option<HeadMeta>,
    pub head_after: Option<HeadMeta>,
    /// Key of each call held at restart, independently seen by the provider.
    pub held_keys: Vec<String>,
    /// All provider executions, not transport reattachments, for those calls.
    pub executed_keys: Vec<String>,
    pub saved_cursor: Option<Value>,
    /// Actual `from` value sent on the resumed subscription.
    pub resumed_from: Option<Value>,
    pub subscribe_error: Option<String>,
    /// Full stream before the saved cursor, including the run's start.
    pub prior_frames: Vec<Value>,
    pub resumed_frames: Vec<Value>,
    /// Durable control event payloads produced by the host in the gap and
    /// afterward, and the corresponding resumed control payloads in order.
    pub host_events: Vec<Value>,
    pub stored_events: Vec<Value>,
    /// The host's final stored assistant text, independent of runner reads.
    pub stored_text: Option<String>,
    /// Handoffs of the experiment's send_id, observed at the host boundary.
    pub send_handoffs: usize,
    pub send_outcome: Option<SendReply>,
    pub send_replay: Option<SendReply>,
    /// Unknown-host experiment: head before Stop, result immediately after it,
    /// and host requests before liveness resolves. Later turn interrupts are
    /// recorded by session id; process-wide stop/kill requests are separate.
    pub unknown_head: Option<HeadMeta>,
    pub stopped: Option<RunResult>,
    pub requests_while_unknown: Vec<Value>,
    pub late_interrupted_sessions: Vec<String>,
    pub session: Option<String>,
    pub host_process_stops: usize,
    pub replacement_hosts_started: usize,
    pub other_session_before: Option<RunResult>,
    pub other_session_after: Option<RunResult>,
    /// Recovery experiment: exact refused mutation error body and store writes
    /// it caused. Reads are raw successful result bodies for each served op.
    pub recovery_reads: Vec<(String, Value)>,
    pub history_served: bool,
    pub recovery_refusal: Option<Value>,
    pub recovery_writes: usize,
}

fn need<'a, T>(value: &'a Option<T>, what: &str) -> Result<&'a T, String> {
    value
        .as_ref()
        .ok_or_else(|| format!("missing {what} observation"))
}

fn natural(o: &RestartSurvivalObservation, marker: &str) -> Result<(), String> {
    let before = need(&o.before, "active run before restart")?;
    let recovering = need(&o.recovering, "run during reattach")?;
    let ended = need(&o.ended, "natural run end")?;
    if before.run_state() != RunState::Active
        || recovering.run_state() != RunState::Active
        || before.run_id != recovering.run_id
        || before.run_id != ended.run_id
        || ended.run_state() != RunState::Completed
        || ended.reason.is_some()
        || ended.final_message.as_ref().map(|m| m.text.as_str()) != Some(marker)
        || o.stored_text.as_deref() != Some(marker)
    {
        return Err(
            "restart changed run_id, sealed the run, or lost the natural host reply".into(),
        );
    }
    Ok(())
}

fn check(name: &str, o: &RestartSurvivalObservation, marker: &str) -> Result<(), String> {
    let runner_before = need(&o.runner_process_before, "runner process before restart")?;
    let runner_after = need(&o.runner_process_after, "runner process after restart")?;
    let host_before = need(&o.host_process_before, "host process before restart")?;
    let host_after = need(&o.host_process_after, "host process after restart")?;
    if runner_before.is_empty()
        || runner_after.is_empty()
        || host_before.is_empty()
        || runner_before == runner_after
        || host_before != host_after
    {
        return Err(
            "experiment did not restart the runner process with the same surviving host".into(),
        );
    }
    match name {
        "restart_survival_model_step" => natural(o, marker),
        "restart_survival_held_tool_call" => {
            natural(o, marker)?;
            let held: BTreeSet<_> = o.held_keys.iter().collect();
            let executed: BTreeSet<_> = o.executed_keys.iter().collect();
            if held.is_empty()
                || held.len() != o.held_keys.len()
                || held != executed
                || o.executed_keys.len() != held.len()
            {
                return Err("provider did not execute each held keyed call exactly once".into());
            }
            Ok(())
        }
        "restart_survival_send_handoff" => {
            natural(o, marker)?;
            let outcome = need(&o.send_outcome, "settled send receipt")?;
            let replay = need(&o.send_replay, "send_id replay")?;
            let as_ = outcome.delivered.as_ref().map(|d| d.r#as.as_str());
            if o.send_handoffs != 1
                || outcome != replay
                || !matches!(as_, Some("step" | "turn" | "unknown"))
            {
                return Err(
                    "pre-restart handoff repeated, lost its final receipt, or replay changed"
                        .into(),
                );
            }
            Ok(())
        }
        "restart_survival_history_identity" => {
            let before = need(&o.head_before, "pre-restart head")?;
            let after = need(&o.head_after, "post-restart head")?;
            if before.history_version != after.history_version
                || before.incarnation != after.incarnation
            {
                return Err("surviving restart changed history_version or incarnation".into());
            }
            Ok(())
        }
        "restart_survival_stored_cursor" => {
            let cursor = need(&o.saved_cursor, "stored cursor")?;
            if o.resumed_from.as_ref() != Some(cursor)
                || o.subscribe_error.is_some()
                || o.host_events.is_empty()
                || o.host_events != o.stored_events
            {
                return Err("stored cursor did not resume contiguously (or cursor_expired)".into());
            }
            Ok(())
        }
        "restart_survival_event_log" => {
            if o.host_events.is_empty() || o.host_events != o.stored_events {
                return Err("catch-up lost, duplicated or reordered stored host events".into());
            }
            let mut running: Option<&str> = None;
            let mut gap = false;
            let mut starts = 0;
            let mut ends = 0;
            let mut resumed_control = Vec::new();
            for (frames, resumed) in [(&o.prior_frames, false), (&o.resumed_frames, true)] {
                for (index, frame) in frames.iter().enumerate() {
                    if frame["seq"].as_u64() != Some(index as u64 + 1) {
                        return Err("noncontiguous stream seq".into());
                    }
                    let event = &frame["event"];
                    if resumed && frame["kind"] == "control" {
                        if frame.get("cursor").is_none() {
                            return Err("control frame has no cursor".into());
                        }
                        resumed_control.push(event.clone());
                    }
                    match event["type"].as_str() {
                        Some("run_started") => {
                            starts += 1;
                            if running.is_some() {
                                return Err("later run started before prior end".into());
                            }
                            running = event["run_id"].as_str();
                            if running.is_none() {
                                return Err("run start has no run_id".into());
                            }
                        }
                        Some("run_finished" | "run_paused") => {
                            ends += 1;
                            if running.is_none() || event["run_id"].as_str() != running {
                                return Err("run end preceded its start".into());
                            }
                            running = None;
                        }
                        Some("display_gap") if resumed => gap = true,
                        Some("text_delta") if resumed => {
                            return Err("lost text deltas were replayed".into());
                        }
                        _ => {}
                    }
                }
            }
            if !gap
                || starts < 2
                || ends < 2
                || running.is_some()
                || resumed_control != o.stored_events
                || o.stored_text.as_deref() != Some(marker)
            {
                return Err("missing display_gap or superseding stored reply".into());
            }
            Ok(())
        }
        "restart_survival_unknown_host_stop" => {
            let head = need(&o.unknown_head, "unknown-host head")?;
            let before = need(&o.before, "unknown active run")?;
            let stopped = need(&o.stopped, "immediate Stop result")?;
            let session = need(&o.session, "session identity")?;
            if head.host != Some(HostState::Unknown)
                || head
                    .last_run_state
                    .as_ref()
                    .is_none_or(|r| r.run_id != before.run_id || r.state != "active")
                || stopped.run_id != before.run_id
                || stopped.run_state() != RunState::Interrupted
                || stopped.reason.as_deref() != Some(interrupt_reasons::HOST_UNKNOWN)
                || !o.requests_while_unknown.is_empty()
                || o.host_process_stops != 0
                || o.replacement_hosts_started != 0
                || o.late_interrupted_sessions != [session.clone()]
                || o.other_session_before
                    .as_ref()
                    .is_none_or(|r| r.run_state() != RunState::Active)
                || o.other_session_before != o.other_session_after
            {
                return Err("unknown-host Stop failed to settle host_unknown without affecting the shared host".into());
            }
            Ok(())
        }
        "restart_survival_recovery_operations" => {
            let head = need(&o.head_before, "stored history identity")?;
            for op in ["session.head", "session.read", "run.status"]
                .into_iter()
                .chain(o.history_served.then_some("session.history"))
            {
                let value = o
                    .recovery_reads
                    .iter()
                    .find(|(name, _)| name == op)
                    .map(|(_, value)| value)
                    .ok_or_else(|| format!("{op} unavailable during reattach"))?;
                if !value.is_object()
                    || (op != "run.status"
                        && value["history_version"].as_u64() != Some(head.history_version))
                {
                    return Err(format!(
                        "{op} unavailable or changed history_version during reattach"
                    ));
                }
            }
            let refusal = need(&o.recovery_refusal, "recovering refusal")?;
            if refusal["code"] != "transient"
                || !cortexkit_role_llm_runner::errors::is_retryable("transient")
                || refusal["detail"]["reason"] != "recovering"
                || o.recovery_writes != 0
            {
                return Err(
                    "host-dependent recovery mutation did not refuse transient without writes"
                        .into(),
                );
            }
            Ok(())
        }
        _ => Err(format!("unknown restart-survival case {name}")),
    }
}

pub(crate) async fn run<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    work_dir: &Path,
    name: &str,
) -> Result<Ending, String> {
    if !case.subject.host_survives_restart() {
        return Ok(Ending::Inapplicable(
            "adapter declares host_survives_restart: false".into(),
        ));
    }
    if !case
        .checked_describe()
        .await?
        .declares(capabilities::RESTART_SURVIVAL)
    {
        return Err("adapter declares surviving hosts but runner lacks restart_survival".into());
    }
    let point = match name {
        "restart_survival_held_tool_call" => RestartPoint::HeldToolCall,
        "restart_survival_send_handoff" => RestartPoint::SendHandoff,
        "restart_survival_unknown_host_stop" => RestartPoint::UnknownHost,
        "restart_survival_recovery_operations" => RestartPoint::Recovering,
        _ => RestartPoint::ModelStep,
    };
    for mode in [RestartMode::Crash, RestartMode::Orderly] {
        let root = work_dir.join(format!("{name}-{mode:?}"));
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let marker = case.mint.next("restart-reply");
        let observed = case
            .subject
            .restart_survival_observation(&root, point, mode, &marker)
            .await
            .map_err(|e| format!("{name} ({mode:?}): {e}"))?;
        check(name, &observed, &marker).map_err(|e| format!("{name} ({mode:?}): {e}"))?;
    }
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FakeRunner {
        defect: Option<&'static str>,
    }
    impl FakeRunner {
        fn observe(&self, name: &str) -> RestartSurvivalObservation {
            let host_events = vec![
                json!({"type":"message_finished","mid":"answer"}),
                json!({"type":"run_finished","run_id":"run-1","reason":"completed"}),
                json!({"type":"run_started","run_id":"run-2"}),
                json!({"type":"run_finished","run_id":"run-2","reason":"completed"}),
            ];
            let mut frames =
                vec![json!({"seq":1,"kind":"display","event":{"type":"display_gap","dropped":1}})];
            for (index, event) in host_events.iter().enumerate() {
                frames.push(json!({"seq":index+2,"kind":"control","cursor":{"offset":index+2},"event":event}));
            }
            let active = RunResult::new("run-1", "active");
            let head = HeadMeta::empty(7, "record-1");
            let receipt: SendReply = serde_json::from_value(json!({"state":"finished","run_id":"run-1","delivered":{"as":"step","ref":"message-1"}})).unwrap();
            let mut observed = RestartSurvivalObservation {
                runner_process_before: Some("100:start-1".into()),
                runner_process_after: Some("101:start-2".into()),
                host_process_before: Some("200:start-3".into()),
                host_process_after: Some("200:start-3".into()),
                before: Some(active.clone()), recovering: Some(active.clone()),
                ended: Some(serde_json::from_value(json!({"run_id":"run-1","state":"completed","final_message":{"mid":"answer","ordinal":2,"text":"natural-reply"}})).unwrap()),
                head_before: Some(head.clone()), head_after: Some(head.clone()),
                held_keys: vec!["key-1".into(), "key-2".into()],
                executed_keys: vec!["key-1".into(), "key-2".into()],
                saved_cursor: Some(json!({"offset":1})), resumed_from: Some(json!({"offset":1})),
                prior_frames: vec![json!({"seq":1,"kind":"control","cursor":{"offset":1},"event":{"type":"run_started","run_id":"run-1"}})],
                resumed_frames: frames,
                host_events: host_events.clone(), stored_events: host_events,
                stored_text: Some("natural-reply".into()), send_handoffs: 1,
                send_outcome: Some(receipt.clone()), send_replay: Some(receipt),
                unknown_head: Some(serde_json::from_value(json!({"history_version":7,"incarnation":"record-1","host":"unknown","last_run_state":{"run_id":"run-1","state":"active"}})).unwrap()),
                stopped: Some(serde_json::from_value(json!({"run_id":"run-1","state":"interrupted","reason":"host_unknown"})).unwrap()),
                session: Some("session-1".into()), late_interrupted_sessions: vec!["session-1".into()],
                other_session_before: Some(RunResult::new("other-run", "active")),
                other_session_after: Some(RunResult::new("other-run", "active")),
                history_served: true,
                recovery_reads: ["session.head", "session.read", "session.history", "run.status"].into_iter().map(|op|(op.into(),json!({"history_version":7}))).collect(),
                recovery_refusal: Some(json!({"code":"transient","message":"reattaching","detail":{"reason":"recovering"}})),
                ..Default::default()
            };
            match self.defect {
                Some("no_restart") => {
                    observed.runner_process_after = observed.runner_process_before.clone()
                }
                Some("seal") => {
                    observed.recovering =
                        Some(RunResult::new("run-1", "interrupted").with_reason("runner_crash"))
                }
                Some("dispatch_twice") => observed.executed_keys.push("key-1".into()),
                Some("handoff_twice") => observed.send_handoffs = 2,
                Some("cursor_expired") => observed.subscribe_error = Some("cursor_expired".into()),
                Some("identity") => observed.head_after.as_mut().unwrap().history_version = 8,
                Some("incarnation") => {
                    observed.head_after.as_mut().unwrap().incarnation = "record-2".into()
                }
                Some("reorder") => observed.stored_events.swap(0, 1),
                Some("end_first") => {
                    observed.prior_frames[0]["event"] =
                        json!({"type":"run_finished","run_id":"run-1"})
                }
                Some("no_gap") => {
                    observed.resumed_frames[0]["event"] = json!({"type":"display_reset"})
                }
                Some("wrong_reason") => {
                    observed.stopped =
                        Some(RunResult::new("run-1", "interrupted").with_reason("host_exited"))
                }
                Some("kill_host") => observed.host_process_stops = 1,
                Some("replace_host") => observed.replacement_hosts_started = 1,
                Some("refuse_stop") => observed.stopped = Some(active),
                Some("recovery_write") => observed.recovery_writes = 1,
                _ => {}
            }
            assert!(name.starts_with("restart_survival_"));
            observed
        }
    }

    #[test]
    fn restart_survival_faulty_runners_fail_each_named_case() {
        for (name, fault) in [
            ("restart_survival_model_step", "no_restart"),
            ("restart_survival_model_step", "seal"),
            ("restart_survival_held_tool_call", "dispatch_twice"),
            ("restart_survival_send_handoff", "handoff_twice"),
            ("restart_survival_stored_cursor", "cursor_expired"),
            ("restart_survival_history_identity", "identity"),
            ("restart_survival_history_identity", "incarnation"),
            ("restart_survival_event_log", "reorder"),
            ("restart_survival_event_log", "end_first"),
            ("restart_survival_event_log", "no_gap"),
            ("restart_survival_unknown_host_stop", "wrong_reason"),
            ("restart_survival_unknown_host_stop", "kill_host"),
            ("restart_survival_unknown_host_stop", "replace_host"),
            ("restart_survival_unknown_host_stop", "refuse_stop"),
            ("restart_survival_recovery_operations", "recovery_write"),
        ] {
            check(
                name,
                &FakeRunner { defect: None }.observe(name),
                "natural-reply",
            )
            .unwrap();
            let error = check(
                name,
                &FakeRunner {
                    defect: Some(fault),
                }
                .observe(name),
                "natural-reply",
            )
            .map_err(|e| format!("{name}: {e}"))
            .unwrap_err();
            assert!(error.starts_with(name), "{error}");
            println!("{fault}: {error}");
        }
    }

    #[test]
    fn restart_survival_unknown_send_receipt_is_final() {
        let name = "restart_survival_send_handoff";
        let mut o = FakeRunner { defect: None }.observe(name);
        let unknown =
            serde_json::from_value(json!({"state":"finished","delivered":{"as":"unknown"}}))
                .unwrap();
        o.send_outcome = Some(unknown);
        o.send_replay = o.send_outcome.clone();
        check(name, &o, "natural-reply").unwrap();
        o.send_replay = Some(serde_json::from_value(json!({"state":"pending"})).unwrap());
        assert!(check(name, &o, "natural-reply").is_err());
    }
}
