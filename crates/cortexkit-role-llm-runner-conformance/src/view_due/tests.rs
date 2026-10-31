use super::*;
use crate::{
    drive::Mint,
    route::{Reply, RouteFailure, SubscribeOutcome},
    subject::Capability,
};
use async_trait::async_trait;
use cortexkit_role_compaction_provider::{
    lineage::MessageRef,
    view::{CompactionView, ViewRange},
};
use cortexkit_role_harness::{
    CrashDriver, Harness, HarnessError, KillPoint, KillReport, PointDeclaration, RouteStamp,
    Trigger,
};
use cortexkit_role_llm_runner::{ops, read::HeadMeta};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
};

const SESSION_REF: &str = "0123456789abcdef0123456789abcdef";
const CASES: &[&str] = &[
    "view_due_first_model_uses_view",
    "view_due_refused_fetch_blocks_model",
    "view_due_later_step_retries",
];
const PENDING_CASES: &[&str] = &[
    "view_pending_first_notice_then_view",
    "view_pending_hold_expires",
    "view_pending_both_members_fail_setup",
    "view_pending_undeclared_fails_setup",
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Fault {
    #[default]
    None,
    RendersInitial,
    CallsAfterRefusal,
    DropsDue,
    AppliesWithoutRetryFetch,
    NoOptIn,
    MissingHook,
    DecoratedFixture,
    NoPendingOptIn,
    MissingPendingHook,
    MissingPublishHook,
    CallsBeforePublication,
    CallsAfterPendingTimeout,
    AcceptsBoth,
    AcceptsUndeclared,
    NoNotice,
    RefusesEarly,
    MissingPendingCap,
}

#[derive(Default)]
struct Stored {
    initial: Option<crate::CompactionMessage>,
    view: Option<CompactionView>,
    refused: bool,
    due: bool,
    setup_done: bool,
    applied: bool,
    calls: Vec<crate::ProviderCall>,
    model_inputs: Vec<Vec<ModelEntry>>,
    messages: Vec<Value>,
    result: Value,
    runs: usize,
    pending: Option<crate::SetupViewPendingScenario>,
    notices: Vec<BTreeMap<String, String>>,
    expires_at: Option<tokio::time::Instant>,
}

#[derive(Clone, Default)]
struct NativeRunner {
    fault: Fault,
    state: Arc<Mutex<BTreeMap<String, Stored>>>,
}

#[derive(Clone)]
struct Route {
    runner: NativeRunner,
    session: String,
}

fn message(text: &str) -> Value {
    json!({"role":"user","content":[{"type":"text","text":text}]})
}

fn row(ordinal: usize, value: Value) -> Value {
    json!({"ordinal":ordinal,"mid":format!("mid-{ordinal}"),"message":value,
        "digest":cortexkit_role_llm_runner::content_digest::content_digest(&value),
        "part_digests":[],"tags":[],"written_at":1000 + ordinal})
}

impl Stored {
    fn send(&mut self, fault: Fault, params: &Value) {
        self.runs += 1;
        self.messages.push(row(
            self.messages.len(),
            message(params["prompt"].as_str().unwrap()),
        ));
        if !self.setup_done {
            let mut setup = json!({"session":"example","harness":"test","request_id":"setup"});
            if fault != Fault::NoOptIn {
                setup["view_due_max_ms"] = json!(10);
            }
            if !matches!(fault, Fault::NoOptIn | Fault::NoPendingOptIn)
                && self.pending != Some(crate::SetupViewPendingScenario::Undeclared)
            {
                setup["view_features"] = json!(["view_pending"]);
            }
            if fault == Fault::MissingPendingCap {
                setup.as_object_mut().unwrap().remove("view_due_max_ms");
            }
            self.calls.push(crate::ProviderCall::new(
                provider_ops::COMPACTION_SETUP,
                setup,
            ));
            self.setup_done = true;
            self.due = self.view.is_some();
        }
        if let Some(scenario) = self.pending {
            use crate::SetupViewPendingScenario::*;
            if matches!(scenario, BothMembers | Undeclared) {
                if !((scenario == BothMembers && fault == Fault::AcceptsBoth)
                    || (scenario == Undeclared && fault == Fault::AcceptsUndeclared))
                {
                    self.fail_pending(cortexkit_role_compaction_provider::setup::VIEW_PENDING);
                    return;
                }
                self.render_model(fault);
                return;
            }
            if fault != Fault::NoNotice {
                let last = self.messages.last().unwrap();
                self.notices.push(
                    cortexkit_role_compaction_provider::notice::AdvanceNotice::new(
                        SESSION_REF,
                        1,
                        "lineage",
                        last["ordinal"].as_u64().unwrap(),
                        last["digest"].as_str().unwrap(),
                        1,
                        "model",
                    )
                    .to_headers(),
                );
            }
            self.calls.push(crate::ProviderCall::new(
                provider_ops::VIEW_GET,
                json!({"session_ref":SESSION_REF,"view_version":1}),
            ));
            if scenario == PublishAfterNotice && fault == Fault::CallsBeforePublication {
                self.render_model(fault);
                return;
            }
            if fault == Fault::RefusesEarly && scenario == HoldExpires {
                self.fail_pending(VIEW_DUE_UNAPPLIED);
                return;
            }
            self.result = json!({"run_id":format!("run-{}",self.runs),"state":"active"});
            if scenario == HoldExpires {
                self.expires_at =
                    Some(tokio::time::Instant::now() + std::time::Duration::from_millis(10));
            }
            return;
        }
        if self.due {
            if fault != Fault::AppliesWithoutRetryFetch || self.runs == 1 {
                self.calls.push(crate::ProviderCall::new(
                    provider_ops::VIEW_GET,
                    json!({"session_ref":SESSION_REF,"view_version":1}),
                ));
            }
            if self.refused && fault != Fault::CallsAfterRefusal {
                self.result = json!({"run_id":format!("run-{}",self.runs),"state":"error",
                    "error":{"class":"compaction","message":"due fetch refused","provider_code":COMPACTION_UNAVAILABLE,"provider":"example-provider","reason":VIEW_DUE_UNAPPLIED}});
                if fault == Fault::DropsDue {
                    self.due = false;
                }
                return;
            }
            if !self.refused {
                self.applied = true;
                self.due = false;
            }
        }
        self.render_model(fault);
    }

    fn fail_pending(&mut self, reason: &str) {
        self.result = json!({"run_id":format!("run-{}",self.runs),"state":"error",
            "error":{"class":"compaction","message":"pending view refused","provider_code":COMPACTION_UNAVAILABLE,"provider":"example-provider","reason":reason}});
    }

    fn render_model(&mut self, fault: Fault) {
        let rendered = if self.applied && fault != Fault::RendersInitial {
            let view = self.view.as_ref().unwrap();
            let replacement: Vec<Value> = match &view.ranges[0] {
                ViewRange::Replace { bytes, .. } => serde_json::from_str(bytes).unwrap(),
                _ => unreachable!(),
            };
            let mut entries: Vec<_> = replacement
                .into_iter()
                .map(|message| {
                    ModelEntry::new(
                        EntrySource::Replacement {
                            compaction_id: "published-view".into(),
                            version: view.view_version,
                            from_ordinal: 0,
                            to_ordinal: 1,
                        },
                        message,
                    )
                })
                .collect();
            entries.extend(self.messages.iter().skip(1).map(|row| {
                ModelEntry::new(
                    EntrySource::Message {
                        ordinal: row["ordinal"].as_u64().unwrap(),
                        mid: row["mid"].as_str().unwrap().into(),
                    },
                    row["message"].clone(),
                )
            }));
            entries
        } else {
            let initial = self.initial.as_ref().unwrap();
            let mut entries: Vec<_> = initial
                .replacement
                .iter()
                .map(|text| {
                    ModelEntry::new(
                        EntrySource::Replacement {
                            compaction_id: initial.compaction_id.clone(),
                            version: initial.version,
                            from_ordinal: 0,
                            to_ordinal: 0,
                        },
                        message(text),
                    )
                })
                .collect();
            entries.extend(self.messages.iter().map(|row| {
                ModelEntry::new(
                    EntrySource::Message {
                        ordinal: row["ordinal"].as_u64().unwrap(),
                        mid: row["mid"].as_str().unwrap().into(),
                    },
                    row["message"].clone(),
                )
            }));
            entries
        };
        self.model_inputs.push(rendered);
        self.result = json!({"run_id":format!("run-{}",self.runs),"state":"active"});
    }
}

#[async_trait]
impl RunnerRoute for Route {
    async fn request(&self, method: &str, params: Value) -> Result<Reply, RouteFailure> {
        if method == ops::ROLE_DESCRIBE {
            let capabilities: Vec<_> = self
                .runner
                .capabilities()
                .iter()
                .filter_map(|capability| capability.group())
                .collect();
            let ops: BTreeSet<_> = cortexkit_role_llm_runner::REQUIRED_OPS
                .iter()
                .copied()
                .chain(capabilities.iter().flat_map(|group| {
                    cortexkit_role_llm_runner::capabilities::group_ops(group)
                        .unwrap()
                        .iter()
                        .copied()
                }))
                .collect();
            return Ok(Reply::Response(
                json!({"majors":[{"version":"llm-runner/v1","ops":ops,"stability":"alpha"}],
                "implementation_version":"example","capabilities":capabilities,"session_capabilities_from":"baseline",
                "max_bytes":{"default":4096,"maximum":8192}}),
            ));
        }
        let mut state = self.runner.state.lock().unwrap();
        let stored = state.entry(self.session.clone()).or_default();
        let value = match method {
            ops::SESSION_SEND => {
                stored.send(self.runner.fault, &params);
                json!({"state":"active","run_id":format!("run-{}",stored.runs),"submission_id":params["send_id"]})
            }
            ops::SESSION_HEAD => {
                let mut head =
                    serde_json::to_value(HeadMeta::new("lineage", 0, 1000).with_history_version(1))
                        .unwrap();
                head["last_run_state"] =
                    json!({"run_id":format!("run-{}",stored.runs),"state":stored.result["state"]});
                head
            }
            ops::SESSION_READ => json!({"lineage_id":"lineage","messages":stored.messages}),
            ops::RUN_RESULT => {
                if stored
                    .expires_at
                    .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
                {
                    if self.runner.fault == Fault::CallsAfterPendingTimeout {
                        stored.render_model(self.runner.fault);
                    } else {
                        stored.fail_pending(VIEW_DUE_UNAPPLIED);
                    }
                }
                stored.result.clone()
            }
            _ => panic!("unexpected op {method}"),
        };
        Ok(Reply::Response(value))
    }
    async fn subscribe(&self, _: Value) -> Result<SubscribeOutcome, RouteFailure> {
        unreachable!()
    }
}

#[async_trait]
impl Harness for NativeRunner {
    type Handle = ();
    type Route = Route;
    fn declared_points(&self) -> Vec<PointDeclaration> {
        vec![]
    }
    async fn spawn(&self, _: &Path) -> Result<(), HarnessError> {
        Ok(())
    }
    async fn route(&self, _: &(), _: &RouteStamp) -> Result<Route, HarnessError> {
        Ok(Route {
            runner: self.clone(),
            session: "describe".into(),
        })
    }
    async fn kill_at(
        &self,
        _: (),
        _: &KillPoint,
        _: Trigger<'_>,
    ) -> Result<KillReport, HarnessError> {
        unreachable!()
    }
    async fn restart(&self, _: &Path) -> Result<(), HarnessError> {
        Ok(())
    }
}

#[async_trait]
impl LlmRunnerSubject for NativeRunner {
    fn runner_id(&self) -> &str {
        "example-runner"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        [
            Capability::Compaction,
            Capability::Queue,
            Capability::TranscriptReads,
            Capability::RunOps,
        ]
        .into()
    }
    fn owner_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "owner".into(),
            scope: None,
        }
    }
    fn stranger_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "stranger".into(),
            scope: None,
        }
    }
    async fn session_route(
        &self,
        _: &(),
        session: &str,
        _: &RouteStamp,
    ) -> Result<Route, HarnessError> {
        Ok(Route {
            runner: self.clone(),
            session: session.into(),
        })
    }
    fn send_fields(&self, _: &str, _: bool) -> Map<String, Value> {
        Map::new()
    }
    fn tool_provider_module(&self) -> String {
        "example-tools".into()
    }
    async fn install_script(&self, name: &str, _: Script) -> Result<(), HarnessError> {
        self.state.lock().unwrap().entry(name.into()).or_default();
        Ok(())
    }
    async fn tool_invocations(&self, _: &Value) -> usize {
        0
    }
    async fn await_tool_call(&self, _: &Value) -> Result<(), HarnessError> {
        Ok(())
    }
    async fn release_tool_call(&self, _: &Value) -> Result<(), HarnessError> {
        Ok(())
    }
    async fn pause(&self) {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    async fn install_compaction_setup(
        &self,
        name: &str,
        setup: CompactionSetup,
    ) -> Result<(), HarnessError> {
        let CompactionSetup::Ready(initial) = setup else {
            unreachable!()
        };
        self.state.lock().unwrap().get_mut(name).unwrap().initial = Some(initial);
        Ok(())
    }
    async fn compaction_observation(
        &self,
        name: &str,
    ) -> Result<crate::CompactionObservation, HarnessError> {
        let state = self.state.lock().unwrap();
        Ok(crate::CompactionObservation {
            model_inputs: state[name].model_inputs.clone(),
            notices: state[name].notices.clone(),
            ..Default::default()
        })
    }
    async fn compaction_provider_calls(
        &self,
        name: &str,
    ) -> Result<Vec<crate::ProviderCall>, HarnessError> {
        Ok(self.state.lock().unwrap()[name].calls.clone())
    }
    async fn release_compaction_model(&self, name: &str) -> Result<(), HarnessError> {
        let mut state = self.state.lock().unwrap();
        let stored = state.get_mut(name).unwrap();
        stored.result["state"] = json!("completed");
        stored.result["final_message"] =
            json!({"ordinal":stored.messages.len(),"mid":"answer","text":"answer"});
        Ok(())
    }
    async fn install_setup_view_due_fixture(
        &self,
        name: &str,
        initial: crate::CompactionMessage,
        replacement: Vec<String>,
        refused: bool,
    ) -> Result<SetupViewDueFixture, HarnessError> {
        if self.fault == Fault::MissingHook {
            return Err(HarnessError::new("fixture is not implemented"));
        }
        let messages: Vec<_> = replacement.iter().map(|text| message(text)).collect();
        let view = CompactionView::new(
            SESSION_REF,
            1,
            "lineage",
            MessageRef {
                ordinal: 0,
                mid: "mid-0".into(),
            },
            vec![ViewRange::replace(
                0,
                1,
                serde_json::to_string(&messages).unwrap(),
            )],
        );
        let rendering = messages
            .into_iter()
            .map(|value| {
                ModelEntry::new(
                    EntrySource::Replacement {
                        compaction_id: "published-view".into(),
                        version: 1,
                        from_ordinal: 0,
                        to_ordinal: 1,
                    },
                    value,
                )
            })
            .collect();
        let mut fixture =
            SetupViewDueFixture::new(SESSION_REF, 1, view.digest.clone(), 0, rendering);
        if self.fault == Fault::DecoratedFixture {
            fixture = fixture.with_model_only_decorations(vec!["render_tags".into()]);
        }
        let mut state = self.state.lock().unwrap();
        let stored = state.get_mut(name).unwrap();
        stored.messages.push(row(0, message("retained prefix")));
        stored.initial = Some(initial);
        stored.view = Some(view);
        stored.refused = refused;
        Ok(fixture)
    }
    async fn set_setup_view_due_fetch_refused(
        &self,
        name: &str,
        refused: bool,
    ) -> Result<(), HarnessError> {
        self.state.lock().unwrap().get_mut(name).unwrap().refused = refused;
        Ok(())
    }

    async fn install_setup_view_pending_fixture(
        &self,
        name: &str,
        initial: crate::CompactionMessage,
        replacement: Vec<String>,
        scenario: crate::SetupViewPendingScenario,
    ) -> Result<SetupViewDueFixture, HarnessError> {
        if self.fault == Fault::MissingPendingHook {
            return Err(HarnessError::new("pending fixture is not implemented"));
        }
        let fixture = self
            .install_setup_view_due_fixture(name, initial, replacement, false)
            .await?;
        self.state.lock().unwrap().get_mut(name).unwrap().pending = Some(scenario);
        Ok(fixture)
    }

    async fn publish_setup_view_pending(&self, name: &str) -> Result<(), HarnessError> {
        if self.fault == Fault::MissingPublishHook {
            return Err(HarnessError::new("pending publication is not implemented"));
        }
        let mut state = self.state.lock().unwrap();
        let stored = state.get_mut(name).unwrap();
        assert!(!stored.notices.is_empty());
        assert!(stored.model_inputs.is_empty());
        stored.applied = true;
        stored.due = false;
        stored.render_model(self.fault);
        Ok(())
    }
}

async fn exercise(name: &str, fault: Fault) -> Result<Ending, String> {
    let subject = NativeRunner {
        fault,
        ..Default::default()
    };
    let directory = tempfile::tempdir().unwrap();
    let mut driver = CrashDriver::new(&subject);
    let handle = driver.spawn(directory.path()).await.unwrap();
    let declared = subject.capabilities();
    let mint = Mint::default();
    Case {
        subject: &subject,
        handle: &handle,
        declared: &declared,
        mint: &mint,
    }
    .run(name)
    .await
}

#[tokio::test]
async fn faithful_runner_passes_all_due_view_cases() {
    for name in CASES {
        assert!(
            matches!(exercise(name, Fault::None).await.unwrap(), Ending::Passed),
            "{name}"
        );
    }
}

#[tokio::test]
async fn initial_rendering_fails_first_model_case_by_name() {
    let name = "view_due_first_model_uses_view";
    let error = exercise(name, Fault::RendersInitial)
        .await
        .err()
        .expect(name);
    assert!(
        error.contains("must equal the due view rendering, never initial"),
        "{name}: {error}"
    );
}

#[tokio::test]
async fn model_after_refusal_fails_both_refusal_cases_by_name() {
    for name in &CASES[1..] {
        let error = exercise(name, Fault::CallsAfterRefusal)
            .await
            .err()
            .expect(name);
        assert!(
            error.contains("called the model after a refused due-view fetch"),
            "{name}: {error}"
        );
    }
}

#[tokio::test]
async fn dropped_due_view_fails_retry_case_by_name() {
    let name = "view_due_later_step_retries";
    let error = exercise(name, Fault::DropsDue).await.err().expect(name);
    assert!(
        error.contains("must equal the due view rendering, never initial"),
        "{name}: {error}"
    );
}

#[tokio::test]
async fn applying_without_a_later_fetch_fails_retry_case_by_name() {
    let name = "view_due_later_step_retries";
    let error = exercise(name, Fault::AppliesWithoutRetryFetch)
        .await
        .err()
        .expect(name);
    assert!(
        error.contains("later step did not retry view.get"),
        "{name}: {error}"
    );
}

#[tokio::test]
async fn runner_without_opt_in_is_inapplicable_by_name() {
    for name in CASES {
        let outcome = exercise(name, Fault::NoOptIn).await.unwrap();
        assert!(
            matches!(outcome, Ending::Inapplicable(reason) if reason.contains("no view_due_max_ms")),
            "{name}"
        );
    }
}

#[tokio::test]
async fn opted_in_adapter_without_fixture_is_inapplicable_by_name() {
    for name in CASES {
        let outcome = exercise(name, Fault::MissingHook).await.unwrap();
        assert!(
            matches!(outcome, Ending::Inapplicable(reason) if reason.contains("install_setup_view_due_fixture")),
            "{name}"
        );
    }
}

#[tokio::test]
async fn decorated_fixture_plan_is_inapplicable_by_name() {
    for name in CASES {
        let outcome = exercise(name, Fault::DecoratedFixture).await.unwrap();
        assert!(
            matches!(outcome, Ending::Inapplicable(reason) if reason.contains("model-only decorations: render_tags")),
            "{name}"
        );
    }
}

#[tokio::test]
async fn faithful_runner_passes_all_pending_view_cases() {
    for name in PENDING_CASES {
        assert!(
            matches!(exercise(name, Fault::None).await.unwrap(), Ending::Passed),
            "{name}"
        );
    }
}

#[tokio::test]
async fn faulty_pending_runners_fail_their_case_by_name() {
    for (name, fault, expected) in [
        (
            PENDING_CASES[0],
            Fault::CallsBeforePublication,
            "before the pending view was published",
        ),
        (
            PENDING_CASES[0],
            Fault::RendersInitial,
            "must equal the due view rendering, never initial",
        ),
        (
            PENDING_CASES[0],
            Fault::NoNotice,
            "did not expose its first readable write's matching notice",
        ),
        (
            PENDING_CASES[1],
            Fault::CallsAfterPendingTimeout,
            "called the model after a refused due-view fetch",
        ),
        (
            PENDING_CASES[1],
            Fault::RefusesEarly,
            "before its hold expired",
        ),
        (
            PENDING_CASES[2],
            Fault::AcceptsBoth,
            "instead of failing Setup naming view_pending",
        ),
        (
            PENDING_CASES[3],
            Fault::AcceptsUndeclared,
            "instead of failing Setup naming view_pending",
        ),
    ] {
        let error = exercise(name, fault).await.err().expect(name);
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[tokio::test]
async fn undeclaring_runner_has_all_pending_cases_inapplicable_by_name() {
    for name in PENDING_CASES {
        let outcome = exercise(name, Fault::NoPendingOptIn).await.unwrap();
        assert!(
            matches!(outcome, Ending::Inapplicable(reason) if reason.contains("no view_pending in Setup view_features")),
            "{name}"
        );
    }
}

#[tokio::test]
async fn pending_adapter_hooks_default_to_inapplicable_by_name() {
    for name in PENDING_CASES {
        let outcome = exercise(name, Fault::MissingPendingHook).await.unwrap();
        assert!(
            matches!(outcome, Ending::Inapplicable(reason) if reason.contains("install_setup_view_pending_fixture")),
            "{name}"
        );
    }
    let outcome = exercise(PENDING_CASES[0], Fault::MissingPublishHook)
        .await
        .unwrap();
    assert!(
        matches!(outcome, Ending::Inapplicable(reason) if reason.contains("publish_setup_view_pending"))
    );
}

#[tokio::test]
async fn pending_support_without_cap_is_a_misconfigured_runner() {
    for name in PENDING_CASES {
        let error = exercise(name, Fault::MissingPendingCap)
            .await
            .err()
            .expect(name);
        assert!(
            error.contains("requires a positive view_due_max_ms"),
            "{name}: {error}"
        );
    }
}
