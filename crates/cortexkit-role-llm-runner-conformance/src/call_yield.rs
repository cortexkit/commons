//! Person-steer cases using a real scripted provider and independent store reads.

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, wait_run_end, MAX_POLLS},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, ScriptedPart, ScriptedTurn},
    ScriptedLateEntry,
};
use cortexkit_role_llm_runner::{
    capabilities, ops,
    run::{RunCancelRequest, RunStatus},
    send::SendReply,
};
use serde_json::{json, Map, Value};

/// Independently captured provider requests and host-store facts. Sequence
/// numbers share one adapter observation log; a stored event means committed,
/// not merely enqueued. Notice rows retain the provider sender and call name.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CallYieldObservation {
    pub call_key: String,
    pub yield_requests: Vec<Value>,
    pub withdraw_requests: Vec<Value>,
    pub steer_stored_sequence: Option<u64>,
    pub first_yield_answer_sequence: Option<u64>,
    pub notices: Vec<Value>,
}

fn check(
    name: &str,
    observation: &CallYieldObservation,
    held_wait: bool,
    provider: &str,
) -> Result<(), String> {
    let negative = name == "call_yield_queue_does_not_yield"
        || name == "call_yield_marked_message_does_not_yield"
        || name == "call_yield_unmarked_sender_does_not_yield";
    if negative {
        return if observation.yield_requests.is_empty() {
            Ok(())
        } else {
            Err("message that is not the person's steer triggered a yield".into())
        };
    }
    if observation.call_key.is_empty()
        || observation.yield_requests.len() != 1
        || observation.yield_requests[0]["arguments"]["call_key"] != observation.call_key
        || observation.yield_requests[0]["arguments"]["reason"] != "person_message"
    {
        return Err("expected exactly one person_message yield for the observed call".into());
    }
    if held_wait {
        return Err("yielded held reply paused the run or entered held_calls".into());
    }
    let stored_without_wait = observation.steer_stored_sequence.is_some_and(|stored| {
        observation
            .first_yield_answer_sequence
            .is_none_or(|answer| stored < answer)
    });
    if name == "call_yield_steer_stored_before_waiting" && !stored_without_wait {
        return Err("steer storage waited behind the yield answer".into());
    }
    if name == "call_yield_stop_never_withdraws" && !observation.withdraw_requests.is_empty() {
        return Err("Stop withdrew a yielded call".into());
    }
    if matches!(
        name,
        "call_yield_late_result_is_notice" | "call_yield_stop_never_withdraws"
    ) && !observation.notices.iter().any(|notice| {
        notice["sender"] == provider
            && notice["notice"] == true
            && notice["text"]
                .as_str()
                .is_some_and(|text| text.contains(&observation.call_key))
    }) {
        return Err(
            "terminal late result was not durably stored as a provider notice naming the call"
                .into(),
        );
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
        .declares(capabilities::CALL_YIELD)
    {
        return Ok(Ending::Inapplicable(
            "runner does not declare call_yield".into(),
        ));
    }
    if case.subject.plan_sender().is_none() {
        return Ok(Ending::Inapplicable(
            "adapter supplies no named sender fixture".into(),
        ));
    }
    let marks: &[Option<&str>] = if name == "call_yield_marked_message_does_not_yield" {
        &[Some("notice"), Some("speaks_for")]
    } else {
        &[None]
    };
    for mark in marks {
        let mut tool = case.mint.call(name);
        tool.hold = true;
        let (session, sender) = crate::delivery_order::sender_session_with_person(
            case,
            name,
            vec![
                ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool.clone())]),
                ScriptedTurn::new(vec![case.mint.text("after-yield").0]),
            ],
            false,
            true,
            name != "call_yield_unmarked_sender_does_not_yield",
        )
        .await?;
        let blocked = name == "call_yield_steer_stored_before_waiting";
        if !case
            .subject
            .install_call_yield(&session.name, &tool.arguments, blocked)
            .await
            .map_err(|e| e.to_string())?
        {
            return Ok(Ending::Inapplicable(
                "adapter supplies no scripted call-yield provider".into(),
            ));
        }
        let start: SendReply = decode(
            "start waiting call",
            expect_ok(
                "start waiting call",
                call(
                    &session.route,
                    ops::SESSION_SEND,
                    crate::send_helpers::params(
                        case,
                        &session,
                        None,
                        "start work",
                        &case.mint.next("start"),
                        &Map::new(),
                    ),
                )
                .await?,
            )?,
        )?;
        case.subject
            .await_tool_call(&tool.arguments)
            .await
            .map_err(|e| e.to_string())?;
        let run_id = head(&session.route)
            .await?
            .last_run_state
            .ok_or("no active run")?
            .run_id;
        if name == "call_yield_stop_never_withdraws" {
            case.subject
                .hold_next_model_step(&session.name)
                .await
                .map_err(|e| e.to_string())?;
        }
        let send_id = case.mint.next("person-send");
        let mut fields = Map::new();
        fields.insert(
            "delivery".into(),
            if name == "call_yield_queue_does_not_yield" {
                json!("queue")
            } else {
                json!("steer")
            },
        );
        if let Some(mark) = mark {
            fields.insert(
                (*mark).into(),
                if *mark == "notice" {
                    json!(true)
                } else {
                    json!({"agent_id":"agent"})
                },
            );
        }
        let send = call(
            &sender,
            ops::SESSION_SEND,
            crate::send_helpers::params(case, &session, None, "person input", &send_id, &fields),
        );
        let inspect = async {
            if blocked {
                let stored = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    case.subject.send_staged(&session.name, &send_id),
                )
                .await;
                let observation = case
                    .subject
                    .observe_call_yield(&session.name, &tool.arguments, &send_id)
                    .await
                    .map_err(|e| e.to_string());
                case.subject
                    .release_call_yield(&tool.arguments)
                    .await
                    .map_err(|e| e.to_string())?;
                stored
                    .map_err(|_| "durable steer storage waited behind a blocked yield")?
                    .map_err(|e| e.to_string())?;
                return observation;
            }
            case.subject.pause().await;
            case.subject
                .observe_call_yield(&session.name, &tool.arguments, &send_id)
                .await
                .map_err(|e| e.to_string())
        };
        let (sent, observed) = futures_util::future::join(send, inspect).await;
        if mark != &Some("speaks_for") {
            expect_ok("person send", sent?)?;
        }
        let mut observed = observed?;
        let negative = name.contains("does_not_yield");
        if negative {
            case.subject
                .release_call_yield(&tool.arguments)
                .await
                .map_err(|e| e.to_string())?;
        }
        if name == "call_yield_stop_never_withdraws" {
            expect_ok(
                "Stop",
                call(
                    &session.route,
                    ops::RUN_CANCEL,
                    serde_json::to_value(RunCancelRequest::new(&run_id)).unwrap(),
                )
                .await?,
            )?;
        }
        if !negative && !blocked {
            for _ in 0..MAX_POLLS {
                observed = case
                    .subject
                    .observe_call_yield(&session.name, &tool.arguments, &send_id)
                    .await
                    .map_err(|e| e.to_string())?;
                if !observed.yield_requests.is_empty() {
                    break;
                }
                case.subject.pause().await;
            }
        }
        let status: RunStatus = decode(
            "run.status",
            expect_ok(
                "run.status",
                call(&session.route, ops::RUN_STATUS, json!({"run_id":run_id})).await?,
            )?,
        )?;
        if !negative && (status.is_held_wait() || !status.held_calls().is_empty()) {
            return Err("yielded call paused the run or entered held_calls".into());
        }
        if matches!(
            name,
            "call_yield_late_result_is_notice" | "call_yield_stop_never_withdraws"
        ) {
            case.subject
                .publish_late_entry(
                    &tool.arguments,
                    ScriptedLateEntry::result(tool.result.clone()),
                )
                .await
                .map_err(|e| e.to_string())?;
            for _ in 0..MAX_POLLS {
                observed = case
                    .subject
                    .observe_call_yield(&session.name, &tool.arguments, &send_id)
                    .await
                    .map_err(|e| e.to_string())?;
                if !observed.notices.is_empty() {
                    break;
                }
                case.subject.pause().await;
            }
        }
        if name != "call_yield_stop_never_withdraws" {
            wait_run_end(case.subject, &session.route, case.declared, None, &start).await?;
        }
        if negative {
            observed = case
                .subject
                .observe_call_yield(&session.name, &tool.arguments, &send_id)
                .await
                .map_err(|e| e.to_string())?;
        }
        check(name, &observed, false, &case.subject.tool_provider_module())
            .map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        Queue,
        Marks,
        Sender,
        Pause,
        DropLate,
        StoreLate,
        Withdraw,
    }
    struct FakeRunner {
        fault: Fault,
    }
    impl FakeRunner {
        fn execute(&self, name: &str) -> (CallYieldObservation, bool) {
            let queued = name == "call_yield_queue_does_not_yield";
            let marked = name == "call_yield_marked_message_does_not_yield";
            let person = name != "call_yield_unmarked_sender_does_not_yield";
            let yield_call = (!queued || self.fault == Fault::Queue)
                && (!marked || self.fault == Fault::Marks)
                && (person || self.fault == Fault::Sender);
            let mut observed = CallYieldObservation {
                call_key: "call-1".into(),
                steer_stored_sequence: Some(if self.fault == Fault::StoreLate { 3 } else { 1 }),
                ..Default::default()
            };
            if yield_call {
                observed.yield_requests.push(json!({"name":"tool.yield","arguments":{"call_key":"call-1","reason":"person_message"}}));
                observed.first_yield_answer_sequence = Some(2);
                if self.fault != Fault::DropLate {
                    observed
                        .notices
                        .push(json!({"sender":"provider","notice":true,"text":"call-1 finished"}));
                }
                if name == "call_yield_stop_never_withdraws" && self.fault == Fault::Withdraw {
                    observed
                        .withdraw_requests
                        .push(json!({"name":"tool.withdraw"}));
                }
            }
            (observed, yield_call && self.fault == Fault::Pause)
        }
    }
    #[test]
    fn call_yield_faulty_runners_fail_each_named_case() {
        for (name, fault) in [
            ("call_yield_queue_does_not_yield", Fault::Queue),
            ("call_yield_marked_message_does_not_yield", Fault::Marks),
            ("call_yield_unmarked_sender_does_not_yield", Fault::Sender),
            ("call_yield_held_reply_never_pauses", Fault::Pause),
            ("call_yield_late_result_is_notice", Fault::DropLate),
            ("call_yield_steer_stored_before_waiting", Fault::StoreLate),
            ("call_yield_stop_never_withdraws", Fault::Withdraw),
        ] {
            let (observed, paused) = FakeRunner { fault: Fault::None }.execute(name);
            check(name, &observed, paused, "provider").unwrap();
            let (observed, paused) = FakeRunner { fault }.execute(name);
            let failure = check(name, &observed, paused, "provider")
                .map_err(|error| format!("{name}: {error}"))
                .unwrap_err();
            assert!(failure.starts_with(name), "{failure}");
        }
    }
}
