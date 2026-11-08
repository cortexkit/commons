//! Person-steer cases using a real scripted provider and independent store reads.

use crate::{
    cases::{Case, Ending, Session},
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
    check_yield(observation, held_wait, "person_message")?;
    if name == "call_yield_steer_stored_before_waiting" {
        check_stored_before_answer(observation)?;
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

/// Both triggers must use the same provider request and held-reply rules.
pub(crate) fn check_yield(
    observation: &CallYieldObservation,
    held_wait: bool,
    reason: &str,
) -> Result<(), String> {
    if observation.call_key.is_empty()
        || observation.yield_requests.len() != 1
        || observation.yield_requests[0]["arguments"]["call_key"] != observation.call_key
        || observation.yield_requests[0]["arguments"]["reason"] != reason
    {
        return Err(format!(
            "expected exactly one {reason} yield for the observed call"
        ));
    }
    if held_wait {
        return Err("yielded held reply paused the run or entered held_calls".into());
    }
    Ok(())
}

pub(crate) fn check_stored_before_answer(observation: &CallYieldObservation) -> Result<(), String> {
    let stored_without_wait = observation.steer_stored_sequence.is_some_and(|stored| {
        observation
            .first_yield_answer_sequence
            .is_none_or(|answer| stored < answer)
    });
    if !stored_without_wait {
        return Err("steer storage waited behind the yield answer".into());
    }
    Ok(())
}

async fn person_session<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
    turns: Vec<ScriptedTurn>,
) -> Result<(Session<S::Route>, S::Route), String>
where
    S::Route: RunnerRoute,
{
    // Legacy steer and queue include idle_start, even during a running call.
    crate::delivery_order::sender_session_with_person(
        case,
        name,
        turns,
        true,
        true,
        name != "call_yield_unmarked_sender_does_not_yield",
    )
    .await
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
        let queued = name == "call_yield_queue_does_not_yield";
        let mut turns = vec![
            ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool.clone())]),
            ScriptedTurn::new(vec![case.mint.text("after-yield").0]),
        ];
        if queued {
            turns.push(ScriptedTurn::new(vec![case.mint.text("after-queue").0]));
        }
        let (session, sender) = person_session(case, name, turns).await?;
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
            if queued {
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
                let observation = async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        case.subject.send_staged(&session.name, &send_id),
                    )
                    .await
                    .map_err(|_| "durable steer storage waited behind a blocked yield")?
                    .map_err(|e| e.to_string())?;
                    let mut observation = CallYieldObservation::default();
                    for _ in 0..MAX_POLLS {
                        observation = case
                            .subject
                            .observe_call_yield(&session.name, &tool.arguments, &send_id)
                            .await
                            .map_err(|e| e.to_string())?;
                        if !observation.yield_requests.is_empty() {
                            break;
                        }
                        case.subject.pause().await;
                    }
                    Ok::<_, String>(observation)
                }
                .await;
                case.subject
                    .release_call_yield(&tool.arguments)
                    .await
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
        let queued_reply: Option<SendReply> = if queued {
            Some(decode(
                "queued person send",
                expect_ok("person send", sent?)?,
            )?)
        } else if mark != &Some("speaks_for") {
            expect_ok("person send", sent?)?;
            None
        } else {
            None
        };
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
            if let Some(reply) = &queued_reply {
                wait_run_end(
                    case.subject,
                    &session.route,
                    case.declared,
                    Some(&run_id),
                    reply,
                )
                .await?;
            } else {
                wait_run_end(case.subject, &session.route, case.declared, None, &start).await?;
            }
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

    async fn operational_case(
        name: &str,
        mode: crate::fork_fake::CallYieldFixture,
    ) -> (Result<(), String>, crate::fork_fake::CallYieldSnapshot) {
        use crate::{
            drive::Mint,
            fork_fake::{Defects, FakeSubject},
            subject::Capability,
        };
        use cortexkit_role_harness::CrashDriver;

        let mut fake = FakeSubject::new(Defects::default());
        fake.call_yield_fixture = Some(mode);
        fake.advertise_status = true;
        fake.capabilities.extend([
            Capability::Plans,
            Capability::DeliveryOrder,
            Capability::Notices,
            Capability::MessageMarks,
            Capability::RunStatus,
            Capability::ReadDisplayFacts,
            Capability::HistoryDisplay,
            Capability::LiveEvents,
        ]);
        let declared = fake.capabilities();
        let mint = Mint::default();
        let root = tempfile::tempdir().unwrap();
        let mut driver = CrashDriver::new(&fake);
        let handle = driver.spawn(root.path()).await.unwrap();
        let case = Case {
            subject: &fake,
            handle: &handle,
            declared: &declared,
            mint: &mint,
        };
        let outcome = case.run(name).await.and_then(|ending| {
            matches!(ending, Ending::Passed)
                .then_some(())
                .ok_or_else(|| format!("{name} did not pass"))
        });
        (outcome, fake.call_yield_fixture_snapshot())
    }

    #[tokio::test]
    async fn call_yield_queue_case_scripts_and_waits_for_queued_run() {
        let (outcome, snapshot) = operational_case(
            "call_yield_queue_does_not_yield",
            crate::fork_fake::CallYieldFixture::OnePollLate,
        )
        .await;
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(
            snapshot.model_requests, 4,
            "opening, held call, continuation, queued run"
        );
        assert_eq!(
            snapshot.run_states.len(),
            3,
            "opening, held run, queued run"
        );
        assert!(
            !snapshot.run_states.iter().any(|state| state == "error"),
            "{:?}",
            snapshot.run_states
        );
        assert_eq!(snapshot.requests, 0);
    }

    #[tokio::test]
    async fn call_yield_blocked_case_accepts_one_poll_late_yield() {
        let (outcome, snapshot) = operational_case(
            "call_yield_steer_stored_before_waiting",
            crate::fork_fake::CallYieldFixture::OnePollLate,
        )
        .await;
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(snapshot.polls, 2);
        assert_eq!(snapshot.requests, 1);
    }

    #[tokio::test]
    async fn call_yield_blocked_case_still_rejects_never_stored_steer() {
        let (outcome, snapshot) = operational_case(
            "call_yield_steer_stored_before_waiting",
            crate::fork_fake::CallYieldFixture::NeverStored,
        )
        .await;
        let error = outcome.unwrap_err();
        assert!(error.contains("call_yield_steer_stored_before_waiting: steer storage waited behind the yield answer"), "{error}");
        assert_eq!(snapshot.requests, 1);
    }

    #[tokio::test]
    async fn call_yield_queue_case_still_rejects_a_yield_request() {
        let (outcome, snapshot) = operational_case(
            "call_yield_queue_does_not_yield",
            crate::fork_fake::CallYieldFixture::QueueYields,
        )
        .await;
        let error = outcome.unwrap_err();
        assert!(error.contains("call_yield_queue_does_not_yield: message that is not the person's steer triggered a yield"), "{error}");
        assert_eq!(snapshot.requests, 1);
    }

    #[tokio::test]
    async fn call_yield_person_fixture_admits_legacy_steer_and_queue() {
        use crate::{
            drive::Mint,
            fork_fake::{Defects, FakeSubject},
            subject::Capability,
            CASES,
        };
        use cortexkit_role_harness::CrashDriver;

        let mut fake = FakeSubject::new(Defects::default());
        fake.capabilities
            .extend([Capability::Plans, Capability::DeliveryOrder]);
        let declared = fake.capabilities();
        let mint = Mint::default();
        let root = tempfile::tempdir().unwrap();
        let mut driver = CrashDriver::new(&fake);
        let handle = driver.spawn(root.path()).await.unwrap();
        let case = Case {
            subject: &fake,
            handle: &handle,
            declared: &declared,
            mint: &mint,
        };
        for spec in CASES
            .iter()
            .filter(|spec| spec.name.starts_with("call_yield_"))
        {
            for delivery in ["steer", "queue"] {
                let (_session, sender) = person_session(
                    &case,
                    spec.name,
                    vec![ScriptedTurn::new(vec![mint.text("answer").0])],
                )
                .await
                .unwrap();
                let reply = call(
                    &sender,
                    ops::SESSION_SEND,
                    json!({"prompt":"person input", "send_id":mint.next("person-send"), "delivery":delivery}),
                )
                .await
                .unwrap();
                expect_ok(spec.name, reply).unwrap();
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        Queue,
        Marks,
        Sender,
        Pause,
        DropLate,
        StoreLate,
        StoreMissing,
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
                steer_stored_sequence: (self.fault != Fault::StoreMissing)
                    .then_some(if self.fault == Fault::StoreLate { 3 } else { 1 }),
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
            (
                "call_yield_steer_stored_before_waiting",
                Fault::StoreMissing,
            ),
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
