//! Session-wide stop checks with externally observed provider requests.
use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_ok, head, run_result},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, ScriptedTurn},
};
use cortexkit_role_llm_runner::{
    capabilities, errors, ops,
    run::RunCancelRequest,
    send::SendReply,
    stop_all::{StopAllReply, StopAllRequest, BUDGET_MS},
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// Actual provider-bound requests and committed host-store observations.
/// Provider sets come from the run snapshot and applied plan, not the fixture's
/// intended configuration. Request rows contain `provider`, `request_id` and
/// `sequence`. Reply rows retain the provider's unmodified response body.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct StopAllObservation {
    pub frozen_providers: Vec<String>,
    pub applied_providers: Vec<String>,
    pub requests: Vec<Value>,
    pub turn_end_sequence: Option<u64>,
    pub provider_replies: Vec<Value>,
    pub stored_reply: Option<Value>,
}
fn check(
    name: &str,
    observation: &StopAllObservation,
    reply: Option<&Value>,
) -> Result<(), String> {
    if name == "stop_all_interrupt_grant_required" {
        return if observation.requests.is_empty()
            && observation.turn_end_sequence.is_none()
            && observation.stored_reply.is_none()
        {
            Ok(())
        } else {
            Err("ungranted sender caused stop-all effects".into())
        };
    }
    let reply = reply.ok_or("stop-all did not return a reply")?;
    let parsed: StopAllReply = serde_json::from_value(reply.clone()).map_err(|e| e.to_string())?;
    let allowed: BTreeSet<_> = observation
        .frozen_providers
        .iter()
        .chain(&observation.applied_providers)
        .map(String::as_str)
        .collect();
    let answered: BTreeSet<_> = parsed
        .providers
        .iter()
        .map(|provider| provider.provider.as_str())
        .collect();
    if answered != allowed || answered.len() != parsed.providers.len() {
        return Err("reply omitted or duplicated a provider in the plan union".into());
    }
    for request in &observation.requests {
        let provider = request["provider"]
            .as_str()
            .ok_or("observed request lacks provider")?;
        if !allowed.contains(provider) {
            return Err("called a provider in neither plan".into());
        }
        if !matches!((observation.turn_end_sequence, request["sequence"].as_u64()), (Some(end), Some(sent)) if end < sent)
        {
            return Err("fan-out preceded durable turn end".into());
        }
    }
    if name == "stop_all_request_id_replays" {
        let mut seen = BTreeSet::new();
        for request in &observation.requests {
            if !seen.insert((request["provider"].as_str(), request["request_id"].as_str())) {
                return Err("request_id replay fanned out again".into());
            }
        }
    }
    if observation.stored_reply.as_ref() != Some(reply) {
        return Err("first stop-all reply was not durably stored unchanged".into());
    }
    for observed in &observation.provider_replies {
        let provider = parsed
            .providers
            .iter()
            .find(|provider| observed["provider"] == provider.provider)
            .ok_or("observed provider absent from reply")?;
        if provider.outcome != "answered"
            || provider.items.as_ref().map(|items| json!(items))
                != observed["reply"].get("items").cloned()
            || provider.more.map(|more| json!(more)) != observed["reply"].get("more").cloned()
        {
            return Err("provider items or more were not forwarded unchanged".into());
        }
    }
    Ok(())
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    if !case
        .checked_describe()
        .await?
        .declares(capabilities::STOP_ALL)
    {
        return Ok(Ending::Inapplicable(
            "runner does not declare stop_all".into(),
        ));
    }
    if case.subject.plan_sender().is_none() {
        return Ok(Ending::Inapplicable(
            "adapter supplies no named sender fixture".into(),
        ));
    }
    let granted = name != "stop_all_interrupt_grant_required";
    let (session, sender) = crate::delivery_order::sender_session(
        case,
        name,
        vec![ScriptedTurn::new(vec![
            case.mint.text("held-for-stop-all").0,
        ])],
        false,
        granted,
    )
    .await?;
    if !case
        .subject
        .install_stop_all(&session.name, name == "stop_all_provider_failure_isolated")
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(
            "adapter supplies no scripted stop-all providers".into(),
        ));
    }
    case.subject
        .hold_next_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let _: SendReply = decode(
        "start",
        expect_ok(
            "start",
            call(
                &session.route,
                ops::SESSION_SEND,
                crate::send_helpers::params(
                    case,
                    &session,
                    None,
                    "start",
                    &case.mint.next("start"),
                    &Map::new(),
                ),
            )
            .await?,
        )?,
    )?;
    case.subject
        .await_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let run_id = head(&session.route)
        .await?
        .last_run_state
        .ok_or("no active run")?
        .run_id;
    let id = case.mint.next("stop-all");
    let params = serde_json::to_value(StopAllRequest::new(&id).with_run_id(&run_id)).unwrap();
    let result = async {
        let answer = tokio::time::timeout(
            std::time::Duration::from_millis(BUDGET_MS),
            call(&sender, ops::SESSION_STOP_ALL, params.clone()),
        )
        .await
        .map_err(|_| "stop-all exceeded its total 15 s budget")??;
        if !granted {
            let error = expect_error("ungranted stop-all", answer)?;
            expect_code("ungranted stop-all", &error, errors::SCOPE_OWNER_MISMATCH)?;
            if run_result(&session.route, &run_id)
                .await?
                .run_state()
                .is_terminal()
            {
                return Err("ungranted stop-all ended the turn".into());
            }
            let observed = case
                .subject
                .observe_stop_all(&session.name, &id)
                .await
                .map_err(|e| e.to_string())?;
            check(name, &observed, None)?;
            expect_ok(
                "cleanup Stop",
                call(
                    &session.route,
                    ops::RUN_CANCEL,
                    serde_json::to_value(RunCancelRequest::new(&run_id)).unwrap(),
                )
                .await?,
            )?;
        } else {
            let reply = expect_ok("stop-all", answer)?;
            if run_result(&session.route, &run_id).await?.state != "cancelled" {
                return Err("stop-all did not end the turn cancelled".into());
            }
            if name == "stop_all_request_id_replays" {
                let replay = expect_ok(
                    "stop-all replay",
                    call(&sender, ops::SESSION_STOP_ALL, params).await?,
                )?;
                if replay != reply {
                    return Err("request_id replay changed the first reply".into());
                }
            }
            let observation = case
                .subject
                .observe_stop_all(&session.name, &id)
                .await
                .map_err(|e| e.to_string())?;
            if observation.frozen_providers.len() < 2 && observation.applied_providers.len() < 2 {
                return Err("stop-all fixture observed fewer than two providers".into());
            }
            check(name, &observation, Some(&reply))?;
        }
        Ok::<_, String>(())
    }
    .await;
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    result.map_err(|e| format!("{name}: {e}"))?;
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        BeforeCommit,
        RepeatFanout,
        AbortOnFailure,
        OutsidePlan,
        Ungranted,
    }
    struct FakeRunner {
        fault: Fault,
    }
    impl FakeRunner {
        fn execute(&self, name: &str) -> (StopAllObservation, Option<Value>) {
            if name == "stop_all_interrupt_grant_required" && self.fault != Fault::Ungranted {
                return (StopAllObservation::default(), None);
            }
            let mut observed = StopAllObservation {
                frozen_providers: vec!["one".into()],
                applied_providers: vec!["two".into()],
                turn_end_sequence: Some(if self.fault == Fault::BeforeCommit {
                    3
                } else {
                    1
                }),
                ..Default::default()
            };
            let mut providers = vec![];
            for provider in ["one", "two"] {
                if provider == "two" {
                    if self.fault == Fault::AbortOnFailure {
                        return (observed, None);
                    }
                    providers.push(json!({"provider":provider,"outcome":"unreachable"}));
                } else {
                    observed
                        .requests
                        .push(json!({"provider":provider,"request_id":"stop-1","sequence":2}));
                    observed
                        .provider_replies
                        .push(json!({"provider":provider,"reply":{"items":[]}}));
                    providers.push(json!({"provider":provider,"outcome":"answered","items":[]}));
                }
            }
            if self.fault == Fault::OutsidePlan {
                observed
                    .requests
                    .push(json!({"provider":"outside","request_id":"stop-1","sequence":2}));
            }
            let reply = json!({"providers":providers,"runner":{"items":[]}});
            observed.stored_reply = Some(reply.clone());
            if name == "stop_all_request_id_replays" && self.fault == Fault::RepeatFanout {
                observed
                    .requests
                    .push(json!({"provider":"one","request_id":"stop-1","sequence":4}));
            }
            (observed, Some(reply))
        }
    }
    #[test]
    fn stop_all_faulty_runners_fail_each_named_case() {
        for (name, fault) in [
            ("stop_all_after_durable_turn_end", Fault::BeforeCommit),
            ("stop_all_request_id_replays", Fault::RepeatFanout),
            ("stop_all_provider_failure_isolated", Fault::AbortOnFailure),
            ("stop_all_plan_union_only", Fault::OutsidePlan),
            ("stop_all_interrupt_grant_required", Fault::Ungranted),
        ] {
            let (observed, reply) = FakeRunner { fault: Fault::None }.execute(name);
            check(name, &observed, reply.as_ref()).unwrap();
            let (observed, reply) = FakeRunner { fault }.execute(name);
            let failure = check(name, &observed, reply.as_ref())
                .map_err(|error| format!("{name}: {error}"))
                .unwrap_err();
            assert!(failure.starts_with(name), "{failure}");
        }
    }
}
