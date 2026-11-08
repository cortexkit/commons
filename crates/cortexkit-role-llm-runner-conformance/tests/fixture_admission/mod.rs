use super::fake::{Defects, FakeSubject};
use cortexkit_role_llm_runner::{errors, ops};
use cortexkit_role_llm_runner_conformance::{
    harness::Harness, Capability, LlmRunnerSubject, Reply, RunnerRoute,
};
use serde_json::{json, Map, Value};

fn subject() -> FakeSubject {
    let mut subject = FakeSubject::new(Defects::default());
    subject
        .capabilities
        .extend([Capability::Plans, Capability::DeliveryOrder]);
    subject
}

fn creating_send(subject: &FakeSubject, session: &str, sender: Value) -> Value {
    let members = Map::from_iter([("senders".into(), json!([sender]))]);
    let mut params = subject
        .send_fields_with_plan(session, true, &members)
        .unwrap();
    params.extend(Map::from_iter([
        ("prompt".into(), json!("opening")),
        ("send_id".into(), json!("opening")),
    ]));
    Value::Object(params)
}

fn refused(reply: Reply, field: &str) {
    let Reply::Error(error) = reply else {
        panic!("old fixture was accepted: {reply:?}");
    };
    assert_eq!(error.code, errors::INVALID_PARAMS);
    assert_eq!(error.detail.unwrap()["field"], field);
}

#[tokio::test]
async fn stamp_module_old_principal_plan_fixture_is_refused_by_faithful_fake() {
    let subject = subject();
    let root = tempfile::tempdir().unwrap();
    let handle = subject.spawn(root.path()).await.unwrap();
    let session = "old-principal-plan-fixture";
    let route = subject
        .session_route(&handle, session, &subject.owner_stamp())
        .await
        .unwrap();
    let old = creating_send(
        &subject,
        session,
        json!({"module":subject.owner_stamp().principal, "idle_start":true}),
    );
    refused(
        route.request(ops::SESSION_SEND, old).await.unwrap(),
        "plan.senders[0].module",
    );
    let fixed = creating_send(
        &subject,
        session,
        json!({"module":"owner", "idle_start":true}),
    );
    let accepted = route.request(ops::SESSION_SEND, fixed).await.unwrap();
    assert!(matches!(accepted, Reply::Response(_)), "{accepted:?}");
}

#[tokio::test]
async fn call_yield_old_person_fixture_without_idle_start_is_refused_by_faithful_fake() {
    for delivery in ["steer", "queue"] {
        for idle_start in [false, true] {
            let subject = subject();
            let root = tempfile::tempdir().unwrap();
            let handle = subject.spawn(root.path()).await.unwrap();
            let session = "person-idle-start-fixture";
            let owner = subject
                .session_route(&handle, session, &subject.owner_stamp())
                .await
                .unwrap();
            let sender = subject.plan_sender().unwrap();
            let opening = creating_send(
                &subject,
                session,
                json!({"module":sender.module, "person":true, "interrupt":true,
                       "idle_start":idle_start}),
            );
            let accepted = owner.request(ops::SESSION_SEND, opening).await.unwrap();
            assert!(matches!(accepted, Reply::Response(_)), "{accepted:?}");
            let route = subject
                .session_route(&handle, session, &sender.stamp)
                .await
                .unwrap();
            let reply = route
                .request(
                    ops::SESSION_SEND,
                    json!({"prompt":"person input", "send_id":"person-send", "delivery":delivery}),
                )
                .await
                .unwrap();
            if idle_start {
                assert!(matches!(reply, Reply::Response(_)), "{delivery}: {reply:?}");
            } else {
                refused(reply, "delivery");
            }
        }
    }
}
