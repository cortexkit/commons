use super::*;
use crate::{
    drive::Mint,
    route::{Reply, RouteFailure, SubscribeOutcome},
};
use async_trait::async_trait;
use cortexkit_role_compaction_provider::notice::{AdvanceNotice, Event};
use cortexkit_role_harness::{
    CrashDriver, Harness, HarnessError, KillPoint, KillReport, PointDeclaration, RouteStamp,
    Trigger,
};
use cortexkit_role_llm_runner::archive_prunes_reasoning::ReasoningPruned;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Fault {
    #[default]
    None,
    RequiredTextRemoved,
    FirstPageOnly,
    MessageDigest,
    PartDigest,
    WrongSha,
    WrongBytes,
    UnlistedChange,
    UnlistedTwinChange,
    ActiveRun,
    QueuedSend,
    SetupOptIn,
    UndeclaredKind,
    MovedPart,
    SummaryChanged,
    SignatureTextChanged,
    AddedMember,
    ReopenRestoresReasoning,
    ExpiresCursor,
    NoGenerationBump,
    AcceptsRestoringView,
    AcceptsRestoringEdit,
    SkipsSetupAfterPrune,
    KeepsPrePruneInitial,
    KeepsPrePruneEdit,
    DropsAppliedEdit,
}

#[derive(Default)]
struct Stored {
    messages: Vec<Value>,
    pruned: Vec<Value>,
    accepts: Option<Vec<String>>,
    active: bool,
    queued: bool,
    hold_active: bool,
    hold_queue: bool,
    reads: usize,
    /// The session's prune state, as `session.head` reports it.
    reasoning_pruned: Option<ReasoningPruned>,
    /// Original values of every part a prune emptied: what a view must not
    /// bring back.
    removed: Vec<Value>,
    setup_done: bool,
    setup_due: bool,
    installed: Option<crate::CompactionMessage>,
    rendered: Option<crate::CompactionMessage>,
    pending: Vec<(u64, crate::ScriptedView)>,
    published: u64,
    applied: Option<u64>,
    /// The part edits of the applied view, which the model view serves in
    /// place of the stored parts they name.
    edits: Vec<crate::ScriptedPartEdit>,
    calls: Vec<crate::ProviderCall>,
    notices: Vec<BTreeMap<String, String>>,
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

fn fixture() -> Vec<ArchivePruningPart> {
    let mut parts = vec![
        ArchivePruningPart {
            kind: "reasoning".into(),
            original: json!({"type":"reasoning","text":"Consider the blue door.","signature":"invented-signature"}),
            served: json!({"type":"reasoning","text":""}),
        },
        ArchivePruningPart {
            kind: "opaque_reasoning".into(),
            original: json!({"type":"opaque","summary":"The door is blue.","encrypted_content":"invented-ciphertext"}),
            served: json!({"type":"opaque","summary":"The door is blue."}),
        },
        ArchivePruningPart {
            kind: "signature".into(),
            original: json!({"type":"text","text":"Open the blue door.","signature":"invented-signature"}),
            served: json!({"type":"text","text":"Open the blue door."}),
        },
    ];
    parts.push(parts[0].clone());
    parts
}

/// A later assistant message with signed reasoning, which every run of the
/// native runner appends, so a later prune has something new to remove.
fn later_oracle() -> ArchivePruningPart {
    ArchivePruningPart {
        kind: "reasoning".into(),
        original: json!({"type":"reasoning","text":"A later thought.","signature":"later-signature"}),
        served: json!({"type":"reasoning","text":""}),
    }
}

fn stored_message(ordinal: usize, parts: Vec<Value>, tags: Value) -> Value {
    let message = json!({"role":"assistant","content":parts});
    let digests: Vec<_> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| json!({"part":i.to_string(),"digest":content_digest(p)}))
        .collect();
    json!({"ordinal":ordinal,"mid":format!("message-{ordinal}"),"message":message,"digest":content_digest(&message),"part_digests":digests,"tags":tags,"written_at":1000 + ordinal,"role":"assistant"})
}

impl Stored {
    /// One run of the native runner: the post-prune Setup and the views due
    /// at its step boundary, then its answer and an advance notice.
    fn serve_run(&mut self, fault: Fault) {
        let mut events = Vec::new();
        if self.accepts.is_some() {
            let first_setup = !self.setup_done;
            if first_setup || (self.setup_due && fault != Fault::SkipsSetupAfterPrune) {
                let mut params = json!({"session":"example"});
                if let Some(state) = &self.reasoning_pruned {
                    params["reasoning_pruned"] = serde_json::to_value(state).unwrap();
                }
                self.calls
                    .push(crate::ProviderCall::new("compaction.setup", params));
                if let Some(answer) = self.installed.clone() {
                    if first_setup
                        || (self.applied.is_none() && fault != Fault::KeepsPrePruneInitial)
                    {
                        self.rendered = Some(answer);
                    }
                }
                self.setup_done = true;
                self.setup_due = false;
            }
            for (version, view) in std::mem::take(&mut self.pending) {
                self.calls.push(crate::ProviderCall::new(
                    "view.get",
                    json!({"view_version":version}),
                ));
                let by_replace = view.ranges.iter().any(|range| {
                    matches!(range, crate::ScriptedRange::ReplaceVerbatim { messages, .. }
                        if messages.iter().any(|m| m["content"].as_array().into_iter().flatten()
                            .any(|part| self.removed.contains(part))))
                });
                let by_edit = view.ranges.iter().any(|range| {
                    matches!(range, crate::ScriptedRange::KeepWithVerbatimEdits { edits, .. }
                        if edits.iter().any(|edit| self.removed.contains(&edit.part_value)))
                });
                let restores = (by_replace && fault != Fault::AcceptsRestoringView)
                    || (by_edit && fault != Fault::AcceptsRestoringEdit);
                if restores {
                    events.push(Event::ViewRefused {
                        view_version: version,
                        reason: "pruned_reasoning_restored".into(),
                    });
                } else {
                    self.applied = Some(version);
                    self.edits = view
                        .ranges
                        .iter()
                        .flat_map(|range| match range {
                            crate::ScriptedRange::KeepWithVerbatimEdits { edits, .. } => {
                                edits.clone()
                            }
                            _ => Vec::new(),
                        })
                        .collect();
                }
            }
        }
        let ordinal = self.messages.len();
        self.messages.push(stored_message(
            ordinal,
            vec![later_oracle().original],
            json!([{"part":"0"}]),
        ));
        if self.accepts.is_some() {
            let last = self.messages.last().unwrap();
            let mut notice = AdvanceNotice::new(
                "0123456789abcdef0123456789abcdef",
                self.notices.len() as u64 + 1,
                "example-lineage",
                ordinal as u64,
                last["digest"].as_str().unwrap(),
                1,
                "example/model",
            )
            .with_events(events);
            if let Some(version) = self.applied {
                notice = notice.with_applied_view_version(version);
            }
            if let Some(state) = self.reasoning_pruned {
                notice = notice.with_reasoning_pruned(state);
            }
            self.notices.push(notice.to_headers());
        }
    }
}

impl NativeRunner {
    fn caps() -> BTreeSet<Capability> {
        [
            Capability::ArchivePrunesReasoning,
            Capability::TranscriptReads,
            Capability::HostHistory,
            Capability::HistoryDisplay,
            Capability::ModelView,
            Capability::Queue,
            Capability::Compaction,
            Capability::Tags,
        ]
        .into()
    }

    fn prune(&self, name: &str) {
        let mut state = self.state.lock().unwrap();
        let session = state.get_mut(name).unwrap();
        if (session.active && self.fault != Fault::ActiveRun)
            || (session.queued && self.fault != Fault::QueuedSend)
            || (session
                .accepts
                .as_ref()
                .is_some_and(|a| !a.iter().any(|a| a == "pruned_reasoning"))
                && self.fault != Fault::SetupOptIn)
        {
            return;
        }
        session.reads = 0;
        let mut touched = Vec::new();
        for entry in &mut session.messages {
            let oracles = if entry["message"]["content"].as_array().unwrap().len() == 1 {
                vec![later_oracle()]
            } else {
                fixture()
            };
            let mid = entry["mid"].clone();
            if session.pruned.iter().any(|row| row["mid"] == mid) {
                continue;
            }
            touched.push(entry["ordinal"].as_u64().unwrap());
            for (index, oracle) in oracles.iter().enumerate() {
                session.removed.push(oracle.original.clone());
                let key = index.to_string();
                let mut served = oracle.served.clone();
                match self.fault {
                    Fault::RequiredTextRemoved if index == 0 => {
                        served.as_object_mut().unwrap().remove("text");
                    }
                    Fault::SummaryChanged if index == 1 => {
                        served["summary"] = json!("Different summary.")
                    }
                    Fault::SignatureTextChanged if index == 2 => {
                        served["text"] = json!("Different text.")
                    }
                    Fault::AddedMember => served["pruned"] = json!(true),
                    _ => {}
                }
                entry["message"]["content"][index] = served;
                let mut row = json!({"mid":entry["mid"],"part":key,"kind":oracle.kind,"bytes":serde_jcs::to_vec(&oracle.original).unwrap().len(),"sha256":content_digest(&oracle.original)});
                if self.fault == Fault::WrongSha {
                    row["sha256"] = json!("0".repeat(64));
                }
                if self.fault == Fault::WrongBytes {
                    row["bytes"] = json!(1);
                }
                if self.fault == Fault::UndeclaredKind {
                    row["kind"] = json!("unknown-kind");
                }
                if self.fault != Fault::UnlistedTwinChange || index != 3 {
                    session.pruned.push(row);
                }
            }
            match self.fault {
                Fault::MessageDigest => entry["digest"] = json!("0".repeat(64)),
                Fault::PartDigest => entry["part_digests"][0]["digest"] = json!("0".repeat(64)),
                Fault::UnlistedChange => {
                    entry["message"]["content"][4]["text"] = json!("Unlisted change.")
                }
                Fault::MovedPart => entry["message"]["content"]
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 2),
                _ => {}
            }
        }
        if let Some(highest) = touched.iter().copied().max() {
            let generation = match session.reasoning_pruned {
                None => 1,
                Some(state) if self.fault == Fault::NoGenerationBump => state.generation,
                Some(state) => state.generation + 1,
            };
            let through = session
                .reasoning_pruned
                .map_or(highest, |state| state.through_ordinal.max(highest));
            session.reasoning_pruned = Some(ReasoningPruned::new(
                generation,
                through,
                1_767_225_600_000 + generation,
            ));
            session.setup_due = session.accepts.is_some();
        }
        // The applied view's edits of a pruned message take the fixture's
        // signature rule, as the stored part did: a non-reasoning part
        // loses its `signature`.
        match self.fault {
            Fault::KeepsPrePruneEdit => {}
            Fault::DropsAppliedEdit => session
                .edits
                .retain(|edit| !touched.contains(&edit.ordinal)),
            _ => {
                for edit in session
                    .edits
                    .iter_mut()
                    .filter(|edit| touched.contains(&edit.ordinal))
                {
                    if edit.part_value["type"] != "reasoning" {
                        if let Some(part) = edit.part_value.as_object_mut() {
                            part.remove("signature");
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl RunnerRoute for Route {
    async fn request(&self, method: &str, params: Value) -> Result<Reply, RouteFailure> {
        if method == ops::ROLE_DESCRIBE {
            let caps: Vec<_> = NativeRunner::caps()
                .iter()
                .filter_map(|c| c.group())
                .collect();
            let ops: BTreeSet<_> = cortexkit_role_llm_runner::REQUIRED_OPS
                .iter()
                .copied()
                .chain(caps.iter().flat_map(|c| {
                    cortexkit_role_llm_runner::capabilities::group_ops(c)
                        .unwrap()
                        .iter()
                        .copied()
                }))
                .collect();
            return Ok(Reply::Response(
                json!({"majors":[{"version":"llm-runner/v1","ops":ops,"stability":"alpha"}],"implementation_version":"example","capabilities":caps,"session_capabilities_from":"baseline","max_bytes":{"default":4096,"maximum":8192},"archive_prunes_reasoning":{"kinds":["reasoning","opaque_reasoning","signature"]}}),
            ));
        }
        let mut state = self.runner.state.lock().unwrap();
        let session = state.entry(self.session.clone()).or_default();
        if self.runner.fault == Fault::ExpiresCursor
            && method == ops::SESSION_HISTORY
            && params.get("cursor").is_some()
            && !session.pruned.is_empty()
        {
            return Ok(Reply::Error(subc_protocol::ErrorBody::new(
                "cursor_expired",
                "pruning expired the cursor",
            )));
        }
        let value = match method {
            ops::SESSION_HEAD => {
                let mut head = serde_json::to_value(
                    cortexkit_role_llm_runner::read::HeadMeta::new("example-lineage", 0, 1000),
                )
                .unwrap();
                head["history_version"] = json!(1);
                head["last_run_state"] = json!({"run_id":"example-run","state":if session.active {"active"} else {"completed"}});
                if let Some(state) = &session.reasoning_pruned {
                    head["reasoning_pruned"] = serde_json::to_value(state).unwrap();
                }
                head
            }
            ops::SESSION_SEND => {
                session.active = session.hold_active;
                session.queued = session.hold_queue;
                if self.runner.fault == Fault::ReopenRestoresReasoning && !session.pruned.is_empty()
                {
                    session.messages[0]["message"]["content"][0] = fixture()[0].original.clone();
                }
                if !session.active && !session.queued {
                    session.serve_run(self.runner.fault);
                }
                json!({"state":if session.queued {"pending"} else {"active"},"run_id":"example-run","submission_id":"example-send"})
            }
            ops::SESSION_READ | ops::SESSION_HISTORY => {
                let index = if method == ops::SESSION_READ {
                    params["from_ordinal"].as_u64().unwrap_or(0) as usize
                } else {
                    params["cursor"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(session.messages.len().saturating_sub(1))
                };
                let mut rows: Vec<_> = session.messages.get(index).cloned().into_iter().collect();
                if params["view"] == "model" {
                    for row in &mut rows {
                        let ordinal = row["ordinal"].as_u64();
                        for edit in session
                            .edits
                            .iter()
                            .filter(|edit| ordinal == Some(edit.ordinal))
                        {
                            let index: usize = edit.part.parse().unwrap();
                            row["message"]["content"][index] = edit.part_value.clone();
                        }
                    }
                    rows = rows.into_iter().map(|r| json!({"source":{"kind":"message","ordinal":r["ordinal"],"mid":r["mid"]},"message":r["message"]})).collect();
                } else if params["form"] == "display" {
                    rows = rows.into_iter().map(|r| json!({"ordinal":r["ordinal"],"mid":r["mid"],"written_at":r["written_at"],"tags":r["tags"],"row":{"role":"assistant","text":"Open the blue door.","parts":[],"model":"example/model"}})).collect();
                }
                let mut page = json!({"messages":rows,"lineage_id":"example-lineage"});
                if method == ops::SESSION_HISTORY {
                    page["history_version"] = json!(1);
                    if index > 0 {
                        page["older"] = json!((index - 1).to_string());
                    }
                    if index + 1 < session.messages.len() {
                        page["newer"] = json!((index + 1).to_string());
                    }
                } else {
                    if index + 1 < session.messages.len() {
                        page["next_from_ordinal"] = json!(index + 1);
                    }
                    if let Some(state) = &session.reasoning_pruned {
                        page["reasoning_pruned"] = serde_json::to_value(state).unwrap();
                    }
                    if let (true, Some(initial)) = (params["view"] == "model", &session.rendered) {
                        page["compaction_id"] = json!(initial.compaction_id);
                        page["version"] = json!(initial.version);
                    }
                }
                if !session.pruned.is_empty() {
                    if self.runner.fault != Fault::FirstPageOnly || session.reads == 0 {
                        let listed: Vec<_> = session
                            .pruned
                            .iter()
                            .filter(|p| p["mid"] == session.messages[index]["mid"])
                            .cloned()
                            .collect();
                        if !listed.is_empty() {
                            page["pruned_reasoning"] = json!(listed);
                        }
                    }
                    session.reads += 1;
                }
                page
            }
            ops::SESSION_RESOLVE => {
                let (entry, tag) = session
                    .messages
                    .iter()
                    .find_map(|entry| {
                        entry["tags"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|t| t["tag"] == params["tag"])
                            .map(|tag| (entry, tag))
                    })
                    .unwrap();
                let index: usize = tag["part"].as_str().unwrap().parse().unwrap();
                json!({"tag":params["tag"],"mid":entry["mid"],"ordinal":entry["ordinal"],"part":tag["part"],"content":entry["message"]["content"][index]["text"]})
            }
            _ => json!({}),
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
        Self::caps()
    }
    fn owner_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "example-owner".into(),
            scope: None,
        }
    }
    fn stranger_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "example-stranger".into(),
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
    fn send_fields(&self, _: &str, _: bool) -> serde_json::Map<String, Value> {
        serde_json::Map::new()
    }
    fn tool_provider_module(&self) -> String {
        "example-tools".into()
    }
    async fn install_script(&self, _: &str, _: Script) -> Result<(), HarnessError> {
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
    async fn pause(&self) {}
    async fn archive_pruning_fixture(
        &self,
        name: &str,
        accepts: Option<Vec<String>>,
    ) -> Result<Vec<ArchivePruningPart>, HarnessError> {
        let mut stored = Stored {
            accepts,
            ..Stored::default()
        };
        for ordinal in 0..2 {
            let mut parts: Vec<_> = fixture().into_iter().map(|p| p.original).collect();
            parts.push(json!({"type":"text","text":"An unchanged reminder slot."}));
            stored.messages.push(stored_message(
                ordinal,
                parts,
                json!([{"part":"0"},{"part":"1"},{"part":"2","tag":format!("§{}§",1 + 2 * ordinal)},{"part":"3"},{"part":"4","tag":format!("§{}§",2 + 2 * ordinal)}]),
            ));
        }
        self.state.lock().unwrap().insert(name.into(), stored);
        Ok(fixture())
    }
    async fn prune_archived_session(&self, name: &str) -> Result<(), HarnessError> {
        self.prune(name);
        Ok(())
    }
    async fn install_compaction_setup(
        &self,
        name: &str,
        setup: crate::CompactionSetup,
    ) -> Result<(), HarnessError> {
        let crate::CompactionSetup::Ready(initial) = setup else {
            return Err(HarnessError::new("only ready answers are scripted here"));
        };
        self.state.lock().unwrap().get_mut(name).unwrap().installed = Some(initial);
        Ok(())
    }
    async fn compaction_observation(
        &self,
        name: &str,
    ) -> Result<crate::CompactionObservation, HarnessError> {
        Ok(crate::CompactionObservation {
            notices: self.state.lock().unwrap()[name].notices.clone(),
            ..Default::default()
        })
    }
    async fn publish_compaction_view(
        &self,
        name: &str,
        view: crate::ScriptedView,
    ) -> Result<u64, HarnessError> {
        let mut state = self.state.lock().unwrap();
        let session = state.get_mut(name).unwrap();
        session.published += 1;
        let version = session.published;
        session.pending.push((version, view));
        Ok(version)
    }
    async fn compaction_provider_calls(
        &self,
        name: &str,
    ) -> Result<Vec<crate::ProviderCall>, HarnessError> {
        Ok(self.state.lock().unwrap()[name].calls.clone())
    }
    async fn release_compaction_model(&self, _: &str) -> Result<(), HarnessError> {
        Ok(())
    }
    async fn hold_next_model_step(&self, name: &str) -> Result<(), HarnessError> {
        self.state
            .lock()
            .unwrap()
            .get_mut(name)
            .unwrap()
            .hold_active = true;
        Ok(())
    }
    async fn await_model_step(&self, name: &str) -> Result<(), HarnessError> {
        assert!(self.state.lock().unwrap()[name].active);
        Ok(())
    }
    async fn release_model_step(&self, name: &str) -> Result<(), HarnessError> {
        self.state.lock().unwrap().get_mut(name).unwrap().active = false;
        Ok(())
    }
    async fn hold_retention_queue(&self, name: &str) -> Result<bool, HarnessError> {
        self.state.lock().unwrap().get_mut(name).unwrap().hold_queue = true;
        Ok(true)
    }
    async fn release_retention_queue(&self, name: &str) -> Result<(), HarnessError> {
        self.state.lock().unwrap().get_mut(name).unwrap().queued = false;
        Ok(())
    }
}

async fn exercise(name: &str, fault: Fault) -> Result<Ending, String> {
    let subject = NativeRunner {
        fault,
        ..NativeRunner::default()
    };
    let directory = tempfile::tempdir().unwrap();
    let mut driver = CrashDriver::new(&subject);
    let handle = driver.spawn(directory.path()).await.unwrap();
    let declared = NativeRunner::caps();
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

macro_rules! proof {
    ($positive:ident, $faulty:ident, $case:literal, $fault:ident) => {
        #[tokio::test]
        async fn $positive() {
            assert!(matches!(
                exercise($case, Fault::None).await.unwrap(),
                Ending::Passed
            ));
        }
        #[tokio::test]
        async fn $faulty() {
            assert!(
                exercise($case, Fault::$fault).await.is_err(),
                concat!($case, " must reject ", stringify!($fault))
            );
        }
    };
}

proof!(
    positive_required_text,
    rejects_required_text_removed,
    "archive_pruning_payloads",
    RequiredTextRemoved
);
proof!(
    positive_every_page,
    rejects_first_page_only,
    "archive_pruning_every_page",
    FirstPageOnly
);
proof!(
    positive_message_digest,
    rejects_changed_message_digest,
    "archive_pruning_write_time_digests",
    MessageDigest
);
proof!(
    positive_part_digest,
    rejects_changed_part_digest,
    "archive_pruning_write_time_digests",
    PartDigest
);
proof!(
    positive_original_sha,
    rejects_wrong_original_sha,
    "archive_pruning_original_attestation",
    WrongSha
);
proof!(
    positive_original_bytes,
    rejects_wrong_original_bytes,
    "archive_pruning_original_attestation",
    WrongBytes
);
proof!(
    positive_only_listed,
    rejects_unlisted_change,
    "archive_pruning_only_listed_parts",
    UnlistedChange
);
proof!(
    positive_active_run,
    rejects_pruning_active_run,
    "archive_pruning_active_run",
    ActiveRun
);
proof!(
    positive_queued_send,
    rejects_pruning_queued_send,
    "archive_pruning_queued_send",
    QueuedSend
);
proof!(
    positive_setup_opt_in,
    rejects_pruning_without_setup_acceptance,
    "archive_pruning_setup_opt_in",
    SetupOptIn
);
proof!(
    positive_declared_kinds,
    rejects_undeclared_kind,
    "archive_pruning_declared_kinds",
    UndeclaredKind
);
proof!(
    positive_identity,
    rejects_moved_part,
    "archive_pruning_identity",
    MovedPart
);
proof!(
    positive_opaque_summary,
    rejects_changed_opaque_summary,
    "archive_pruning_payloads",
    SummaryChanged
);
proof!(
    positive_signature_text,
    rejects_changed_signature_text,
    "archive_pruning_payloads",
    SignatureTextChanged
);
proof!(
    positive_no_added_member,
    rejects_added_block_member,
    "archive_pruning_payloads",
    AddedMember
);
proof!(
    positive_reopen,
    rejects_restoring_reasoning_on_reopen,
    "archive_pruning_reopen",
    ReopenRestoresReasoning
);

#[tokio::test]
async fn positive_no_provider() {
    assert!(matches!(
        exercise("archive_pruning_no_provider", Fault::None)
            .await
            .unwrap(),
        Ending::Passed
    ));
}

proof!(
    positive_history_cursor,
    rejects_expiring_cursor_on_prune,
    "archive_pruning_history_cursor",
    ExpiresCursor
);

proof!(
    positive_only_listed_twins,
    rejects_unlisted_identical_part_change,
    "archive_pruning_only_listed_parts",
    UnlistedTwinChange
);

/// A faulty runner fails its case, and the failure names what it broke.
macro_rules! named_proof {
    ($positive:ident, $faulty:ident, $case:literal, $fault:ident, $named:literal) => {
        #[tokio::test]
        async fn $positive() {
            assert!(matches!(
                exercise($case, Fault::None).await.unwrap(),
                Ending::Passed
            ));
        }
        #[tokio::test]
        async fn $faulty() {
            let Err(failure) = exercise($case, Fault::$fault).await else {
                panic!(concat!($case, " must reject ", stringify!($fault)));
            };
            assert!(failure.contains($named), "{failure}");
        }
    };
}

named_proof!(
    positive_generation,
    rejects_prune_without_generation_bump,
    "archive_pruning_generation",
    NoGenerationBump,
    "a later prune did not raise the generation by one"
);
named_proof!(
    positive_restored_view_refused,
    rejects_applying_a_view_that_restores_pruned_content,
    "archive_pruning_restored_view_refused",
    AcceptsRestoringView,
    "a view restoring pruned content was not refused view_refused:1:pruned_reasoning_restored"
);
named_proof!(
    positive_setup_after_prune,
    rejects_skipping_setup_after_prune,
    "archive_pruning_setup_after_prune",
    SkipsSetupAfterPrune,
    "the runner made no call to the compaction provider after the prune"
);
named_proof!(
    positive_setup_initial_replaced,
    rejects_keeping_the_pre_prune_initial,
    "archive_pruning_setup_initial_replaced",
    KeepsPrePruneInitial,
    "the runner still renders the pre-prune one"
);

named_proof!(
    positive_restored_edit_refused,
    rejects_applying_a_part_edit_that_restores_pruned_content,
    "archive_pruning_restored_edit_refused",
    AcceptsRestoringEdit,
    "a part edit restoring pruned content was not refused view_refused:1:pruned_reasoning_restored"
);

named_proof!(
    positive_applied_edit_pruned,
    rejects_keeping_an_applied_edits_pre_prune_signature,
    "archive_pruning_applied_edit_pruned",
    KeepsPrePruneEdit,
    "after the prune, the model view still serves the applied part edit with its pre-prune \
     signature"
);

#[tokio::test]
async fn rejects_dropping_an_applied_edit_on_prune() {
    let Err(failure) = exercise(
        "archive_pruning_applied_edit_pruned",
        Fault::DropsAppliedEdit,
    )
    .await
    else {
        panic!("archive_pruning_applied_edit_pruned must reject DropsAppliedEdit");
    };
    assert!(
        failure.contains("the runner dropped the applied part edit"),
        "{failure}"
    );
}

#[tokio::test]
async fn applied_edit_case_is_not_applicable_without_the_model_view() {
    let subject = NativeRunner::default();
    let directory = tempfile::tempdir().unwrap();
    let mut driver = CrashDriver::new(&subject);
    let handle = driver.spawn(directory.path()).await.unwrap();
    let mut declared = NativeRunner::caps();
    declared.remove(&Capability::ModelView);
    let mint = Mint::default();
    let ending = Case {
        subject: &subject,
        handle: &handle,
        declared: &declared,
        mint: &mint,
    }
    .run("archive_pruning_applied_edit_pruned")
    .await
    .unwrap();
    match ending {
        Ending::Inapplicable(reason) => assert!(reason.contains("model_view"), "{reason}"),
        _ => panic!("the case ran without the model view"),
    }
}

#[test]
fn the_expected_edit_follows_the_declared_profile() {
    // An OpenCode tool part, where call and result share one part: the
    // provider's edit trims the output and keeps the call's signature.
    let tool = |output: &str, google: Value| {
        json!({"id":"prt_tool","type":"tool","callID":"call-example","tool":"open_door",
            "state":{"status":"completed","input":{"door":"blue"},"output":output},
            "metadata":{"google":google}})
    };
    let oracle = ArchivePruningPart {
        kind: "signature".into(),
        original: tool(
            "The blue door is open.",
            json!({"thoughtSignature":"invented-signature"}),
        ),
        served: tool("The blue door is open.", json!({})),
    };
    let edit = provider_edit(&oracle.original);
    assert_eq!(
        edit,
        tool(
            EDIT_PLACEHOLDER,
            json!({"thoughtSignature":"invented-signature"})
        )
    );
    let pruned = tool(EDIT_PLACEHOLDER, json!({}));
    assert_eq!(
        expected_pruned_edit(
            Some(cortexkit_role_llm_runner::serializer_profile::OPENCODE_AISDK),
            &oracle,
            &edit
        ),
        Ok(pruned.clone())
    );
    // Without a profile the rules name, the fixture's served form is
    // edited the same way.
    assert_eq!(
        expected_pruned_edit(None, &oracle, &edit),
        Ok(pruned.clone())
    );
    assert_eq!(
        expected_pruned_edit(Some("future-profile"), &oracle, &edit),
        Ok(pruned)
    );
}
