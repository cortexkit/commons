//! Owner-requested hand-back reuses the person-steer provider and observations.

use crate::{
    call_yield::{check_stored_before_answer, check_yield, CallYieldObservation},
    cases::{Case, Ending},
    drive::{
        call, decode, expect_error, expect_field, expect_ok, head, read_all, wait_run_end,
        MAX_POLLS,
    },
    route::{Reply, RunnerRoute},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use cortexkit_role_llm_runner::{capabilities, errors, ops, run::RunStatus, send::SendReply};
use serde_json::{json, Map, Value};

#[derive(Default)]
struct Evidence {
    calls: Vec<CallYieldObservation>,
    held_wait: bool,
    landed: bool,
    refusals: Vec<(Reply, Vec<u8>, Vec<u8>)>,
}

fn refusal_fields(name: &str) -> &'static [&'static str] {
    match name {
        "notice_yield_sender_and_delivery_refused" => &["yield", "yield"],
        "notice_yield_non_owner_plan_refused" => {
            &["plan.senders[0].yields", "plan.senders[0].yields"]
        }
        "notice_yield_undeclared_members_refused" => {
            &["plan.senders[0].yields", "plan.senders[0].yields", "yield"]
        }
        _ => &[],
    }
}

fn check(name: &str, evidence: &Evidence) -> Result<(), String> {
    let result = (|| {
        let fields = refusal_fields(name);
        if !fields.is_empty() {
            if evidence.refusals.len() != fields.len() {
                return Err("missing refusal observations".into());
            }
            for ((reply, before, after), field) in evidence.refusals.iter().zip(fields) {
                let error = expect_error("urgent hand-back admission", reply.clone())?;
                expect_field(
                    "urgent hand-back admission",
                    &error,
                    errors::INVALID_PARAMS,
                    field,
                )?;
                if before != after {
                    return Err("refused send wrote admission state or fetched a plan".into());
                }
            }
            return Ok(());
        }
        let expected_calls = if name == "notice_yield_running_calls_and_steer" {
            2
        } else {
            1
        };
        if evidence.calls.len() != expected_calls || !evidence.landed {
            return Err("missing eligible call observations or delivered steer".into());
        }
        for call in &evidence.calls {
            check_yield(call, evidence.held_wait, "urgent_notice")?;
            check_stored_before_answer(call)?;
        }
        Ok(())
    })();
    result.map_err(|error: String| format!("{name}: {error}"))
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let declared = case
        .checked_describe()
        .await?
        .declares(capabilities::NOTICE_YIELD);
    let undeclared_case = name == "notice_yield_undeclared_members_refused";
    if declared == undeclared_case {
        return Ok(Ending::Inapplicable(if undeclared_case {
            "runner declares notice_yield".into()
        } else {
            "runner does not declare notice_yield".into()
        }));
    }
    let stamp = case.subject.owner_stamp();
    if !undeclared_case
        && !stamp
            .scope
            .as_ref()
            .is_some_and(|scope| scope.owner == stamp.principal)
    {
        return Ok(Ending::Inapplicable(
            "adapter supplies no scope-owner route".into(),
        ));
    }
    if !refusal_fields(name).is_empty() {
        return refusals(case, name).await;
    }
    effect(case, name).await
}

async fn refusals<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut evidence = Evidence::default();
    for (index, _) in refusal_fields(name).iter().enumerate() {
        let session = case.open(name, Script::default()).await?;
        let Some(before) = case
            .subject
            .call_yield_admission_state(&session.name)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(Ending::Inapplicable(
                "adapter supplies no independent admission-state snapshot".into(),
            ));
        };
        let owner_stamp = case.subject.owner_stamp();
        let owner = crate::send_helpers::stamp_module(name, &owner_stamp)?;
        let sender_case = name == "notice_yield_sender_and_delivery_refused";
        let plan_case = name == "notice_yield_non_owner_plan_refused"
            || (name == "notice_yield_undeclared_members_refused" && index < 2);
        let mut members = Map::new();
        let mut entry = json!({"module": owner});
        if plan_case {
            if name == "notice_yield_non_owner_plan_refused" {
                let Some(sender) = case.subject.plan_sender() else {
                    return Ok(Ending::Inapplicable(
                        "adapter supplies no non-owner sender fixture".into(),
                    ));
                };
                entry["module"] = json!(sender.module);
            }
            entry["yields"] = json!(index == 0);
        } else if sender_case && index == 1 {
            entry["yields"] = json!(true);
        }
        members.insert("senders".into(), json!([entry]));
        let Some(mut fields) = case
            .subject
            .send_fields_with_plan(&session.name, true, &members)
        else {
            return Ok(Ending::Inapplicable(
                "adapter builds no plan carrying senders".into(),
            ));
        };
        fields.extend(json!({"prompt":"refuse urgent input", "send_id":case.mint.next("refused"), "delivery":"steer"}).as_object().unwrap().clone());
        if !plan_case {
            fields.insert("yield".into(), json!(true));
        }
        if sender_case && index == 1 {
            fields.insert("delivery".into(), json!(["idle_start", "steer"]));
        }
        let reply = call(&session.route, ops::SESSION_SEND, Value::Object(fields)).await?;
        let after = case
            .subject
            .call_yield_admission_state(&session.name)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("admission snapshot disappeared")?;
        evidence.refusals.push((reply, before, after));
    }
    check(name, &evidence)?;
    Ok(Ending::Passed)
}

async fn effect<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let count = if name == "notice_yield_running_calls_and_steer" {
        2
    } else {
        1
    };
    let tools: Vec<_> = (0..count)
        .map(|_| {
            let mut tool = case.mint.call(name);
            tool.hold = true;
            tool
        })
        .collect();
    let mut session = case
        .open(
            name,
            Script {
                turns: vec![
                    ScriptedTurn::new(tools.iter().cloned().map(ScriptedPart::ToolCall).collect()),
                    ScriptedTurn::new(vec![case.mint.text("after-urgent-notice").0]),
                ],
            },
        )
        .await?;
    for tool in &tools {
        if !case
            .subject
            .install_call_yield(&session.name, &tool.arguments, true)
            .await
            .map_err(|e| e.to_string())?
        {
            return Ok(Ending::Inapplicable(
                "adapter supplies no scripted call-yield provider".into(),
            ));
        }
    }
    let owner_stamp = case.subject.owner_stamp();
    let owner = crate::send_helpers::stamp_module(name, &owner_stamp)?;
    let members = json!({"senders":[{"module":owner,"yields":true}]})
        .as_object()
        .unwrap()
        .clone();
    let Some(mut fields) = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
    else {
        return Ok(Ending::Inapplicable(
            "adapter builds no plan carrying senders".into(),
        ));
    };
    let start_id = case.mint.next("start");
    fields.extend(
        json!({"prompt":"start waiting work", "send_id":start_id})
            .as_object()
            .unwrap()
            .clone(),
    );
    let start: SendReply = decode(
        "start waiting work",
        expect_ok(
            "start waiting work",
            call(&session.route, ops::SESSION_SEND, Value::Object(fields)).await?,
        )?,
    )?;
    session.first = Some((start_id, "start waiting work".into()));
    for tool in &tools {
        case.subject
            .await_tool_call(&tool.arguments)
            .await
            .map_err(|e| e.to_string())?;
    }
    let run_id = head(&session.route)
        .await?
        .last_run_state
        .ok_or("no waiting run")?
        .run_id;
    let send_id = case.mint.next("urgent-send");
    let text = case.mint.next("urgent-text");
    let second_text = case.mint.next("second-urgent-text");
    let mut marked = json!({"delivery":"steer","yield":true})
        .as_object()
        .unwrap()
        .clone();
    if name == "notice_yield_notice_mark_admitted" {
        marked.insert("notice".into(), json!(true));
    }
    let params = crate::send_helpers::params(case, &session, None, &text, &send_id, &marked);
    let send = call(&session.route, ops::SESSION_SEND, params);
    let inspect = async {
        let result = async {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                case.subject.send_staged(&session.name, &send_id),
            )
            .await
            .map_err(|_| "urgent steer storage waited behind a blocked yield")?
            .map_err(|e| e.to_string())?;
            let mut observations = Vec::new();
            for _ in 0..MAX_POLLS {
                observations.clear();
                for tool in &tools {
                    observations.push(
                        case.subject
                            .observe_call_yield(&session.name, &tool.arguments, &send_id)
                            .await
                            .map_err(|e| e.to_string())?,
                    );
                }
                if observations.iter().all(|o| !o.yield_requests.is_empty()) {
                    break;
                }
                case.subject.pause().await;
            }
            // All answers are still blocked: observing every request here
            // proves dispatch did not wait for one answer before the next.
            for observation in &observations {
                check_yield(observation, false, "urgent_notice")?;
                check_stored_before_answer(observation)?;
            }
            Ok::<(), String>(())
        }
        .await;
        let release = || async {
            // Release even on assertion failure so a faulty runner cannot
            // strand a provider fixture or leak it into another case.
            for tool in &tools {
                case.subject
                    .release_call_yield(&tool.arguments)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        };
        if result.is_ok() && name == "notice_yield_second_send_never_reyields" {
            let second_id = case.mint.next("second-urgent-send");
            let second = call(
                &session.route,
                ops::SESSION_SEND,
                crate::send_helpers::params(
                    case,
                    &session,
                    None,
                    &second_text,
                    &second_id,
                    &marked,
                ),
            );
            let unblock = async {
                let staged = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    case.subject.send_staged(&session.name, &second_id),
                )
                .await;
                release().await?;
                staged
                    .map_err(|_| "second urgent steer was not staged")?
                    .map_err(|e| e.to_string())
            };
            // The send reply may wait for delivery. Release after durable
            // staging, not after the reply, to avoid a circular wait.
            let (sent, unblocked) = futures_util::future::join(second, unblock).await;
            unblocked?;
            expect_ok("second urgent send", sent?)?;
            return Ok(());
        }
        release().await?;
        result
    };
    let (sent, inspected) = futures_util::future::join(send, inspect).await;
    expect_ok("urgent send", sent?)?;
    inspected.map_err(|error| format!("{name}: {error}"))?;
    let status: RunStatus = decode(
        "run.status",
        expect_ok(
            "run.status",
            call(&session.route, ops::RUN_STATUS, json!({"run_id":run_id})).await?,
        )?,
    )?;
    wait_run_end(case.subject, &session.route, case.declared, None, &start).await?;
    let (_, messages) = read_all(&session.route).await?;
    let mut evidence = Evidence {
        held_wait: status.is_held_wait() || !status.held_calls().is_empty(),
        landed: crate::delivery_order::delivered_once(&messages, &text, "steer").is_ok(),
        ..Default::default()
    };
    if name == "notice_yield_second_send_never_reyields" {
        evidence.landed &=
            crate::delivery_order::delivered_once(&messages, &second_text, "steer").is_ok();
    }
    for tool in &tools {
        evidence.calls.push(
            case.subject
                .observe_call_yield(&session.name, &tool.arguments, &send_id)
                .await
                .map_err(|e| e.to_string())?,
        );
    }
    check(name, &evidence)?;
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        PersonReason,
        UnmarkedSender,
        QueueFirst,
        NonOwnerPlan,
        Undeclared,
        IgnoreNotice,
        Reyield,
        WriteBeforeRefusal,
    }

    struct FakeRunner {
        fault: Fault,
        writes: u8,
    }

    impl FakeRunner {
        fn send(
            &mut self,
            group: bool,
            plan: Option<(&str, bool)>,
            sender_marked: bool,
            steer_first: bool,
        ) -> Reply {
            let field = if (plan.is_some() && !group && self.fault != Fault::Undeclared)
                || (plan.is_some_and(|(module, _)| module != "owner")
                    && self.fault != Fault::NonOwnerPlan)
            {
                Some("plan.senders[0].yields")
            } else if plan.is_none()
                && ((!group && self.fault != Fault::Undeclared)
                    || (!sender_marked && self.fault != Fault::UnmarkedSender)
                    || (!steer_first && self.fault != Fault::QueueFirst))
            {
                Some("yield")
            } else {
                None
            };
            if let Some(field) = field {
                if self.fault == Fault::WriteBeforeRefusal {
                    self.writes += 1;
                }
                Reply::Error(serde_json::from_value(json!({"code":"invalid_params","message":"invalid urgent request","retryable":false,"detail":{"field":field}})).unwrap())
            } else {
                self.writes += 1;
                Reply::Response(json!({"state":"pending"}))
            }
        }

        fn refuse(
            &mut self,
            evidence: &mut Evidence,
            group: bool,
            plan: Option<(&str, bool)>,
            marked: bool,
            steer: bool,
        ) {
            let before = vec![self.writes];
            let reply = self.send(group, plan, marked, steer);
            evidence.refusals.push((reply, before, vec![self.writes]));
        }

        fn execute(&mut self, name: &str) -> Evidence {
            let mut evidence = Evidence::default();
            match name {
                "notice_yield_sender_and_delivery_refused" => {
                    self.refuse(&mut evidence, true, None, false, true);
                    self.refuse(&mut evidence, true, None, true, false);
                }
                "notice_yield_non_owner_plan_refused" => {
                    self.refuse(&mut evidence, true, Some(("peer", true)), false, true);
                    self.refuse(&mut evidence, true, Some(("peer", false)), false, true);
                }
                "notice_yield_undeclared_members_refused" => {
                    self.refuse(&mut evidence, false, Some(("owner", true)), false, true);
                    self.refuse(&mut evidence, false, Some(("owner", false)), false, true);
                    self.refuse(&mut evidence, false, None, true, true);
                }
                _ => {
                    let notice = name == "notice_yield_notice_mark_admitted";
                    let repeat = name == "notice_yield_second_send_never_reyields";
                    let count = if name == "notice_yield_running_calls_and_steer" {
                        2
                    } else {
                        1
                    };
                    evidence.landed = true;
                    for index in 0..count {
                        let key = format!("running-call-{index}");
                        let mut call = CallYieldObservation {
                            call_key: key.clone(),
                            steer_stored_sequence: Some(1),
                            first_yield_answer_sequence: Some(3),
                            ..Default::default()
                        };
                        if !(notice && self.fault == Fault::IgnoreNotice) {
                            let reason = if self.fault == Fault::PersonReason {
                                "person_message"
                            } else {
                                "urgent_notice"
                            };
                            call.yield_requests.push(json!({"name":"tool.yield","arguments":{"call_key":key,"reason":reason}}));
                            if repeat && self.fault == Fault::Reyield {
                                call.yield_requests.push(call.yield_requests[0].clone());
                            }
                        }
                        evidence.calls.push(call);
                    }
                }
            }
            evidence
        }
    }

    fn rejects_fault(name: &str, fault: Fault) {
        let faithful = FakeRunner {
            fault: Fault::None,
            writes: 0,
        }
        .execute(name);
        check(name, &faithful).unwrap();
        let faulty = FakeRunner { fault, writes: 0 }.execute(name);
        let failure = check(name, &faulty).unwrap_err();
        assert!(failure.starts_with(name), "{failure}");
        println!("faulty runner rejected: {failure}");
    }

    #[test]
    fn notice_yield_running_calls_and_steer_rejects_faulty_runner() {
        rejects_fault("notice_yield_running_calls_and_steer", Fault::PersonReason);
    }
    #[test]
    fn notice_yield_sender_and_delivery_refused_rejects_faulty_runners() {
        rejects_fault(
            "notice_yield_sender_and_delivery_refused",
            Fault::UnmarkedSender,
        );
        rejects_fault(
            "notice_yield_sender_and_delivery_refused",
            Fault::QueueFirst,
        );
        rejects_fault(
            "notice_yield_sender_and_delivery_refused",
            Fault::WriteBeforeRefusal,
        );
    }
    #[test]
    fn notice_yield_non_owner_plan_refused_rejects_faulty_runner() {
        rejects_fault("notice_yield_non_owner_plan_refused", Fault::NonOwnerPlan);
    }
    #[test]
    fn notice_yield_undeclared_members_refused_rejects_faulty_runner() {
        rejects_fault("notice_yield_undeclared_members_refused", Fault::Undeclared);
    }
    #[test]
    fn notice_yield_notice_mark_admitted_rejects_faulty_runner() {
        rejects_fault("notice_yield_notice_mark_admitted", Fault::IgnoreNotice);
    }
    #[test]
    fn notice_yield_second_send_never_reyields_rejects_faulty_runner() {
        rejects_fault("notice_yield_second_send_never_reyields", Fault::Reyield);
    }
}
