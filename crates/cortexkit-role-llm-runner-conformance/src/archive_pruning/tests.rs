use super::*;
use crate::{
    drive::Mint,
    route::{Reply, RouteFailure, SubscribeOutcome},
};
use async_trait::async_trait;
use cortexkit_role_harness::{
    CrashDriver, Harness, HarnessError, KillPoint, KillReport, PointDeclaration, RouteStamp,
    Trigger,
};
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
        for entry in &mut session.messages {
            for (index, oracle) in fixture().iter().enumerate() {
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
                head
            }
            ops::SESSION_SEND => {
                session.active = session.hold_active;
                session.queued = session.hold_queue;
                if self.runner.fault == Fault::ReopenRestoresReasoning && !session.pruned.is_empty()
                {
                    session.messages[0]["message"]["content"][0] = fixture()[0].original.clone();
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
                } else if index + 1 < session.messages.len() {
                    page["next_from_ordinal"] = json!(index + 1);
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
            let message = json!({"role":"assistant","content":parts});
            let digests: Vec<_> = parts
                .iter()
                .enumerate()
                .map(|(i, p)| json!({"part":i.to_string(),"digest":content_digest(p)}))
                .collect();
            stored.messages.push(json!({"ordinal":ordinal,"mid":format!("message-{ordinal}"),"message":message,"digest":content_digest(&message),"part_digests":digests,"tags":[{"part":"0"},{"part":"1"},{"part":"2","tag":format!("§{}§",1 + 2 * ordinal)},{"part":"3"},{"part":"4","tag":format!("§{}§",2 + 2 * ordinal)}],"written_at":1000 + ordinal,"role":"assistant"}));
        }
        self.state.lock().unwrap().insert(name.into(), stored);
        Ok(fixture())
    }
    async fn prune_archived_session(&self, name: &str) -> Result<(), HarnessError> {
        self.prune(name);
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
