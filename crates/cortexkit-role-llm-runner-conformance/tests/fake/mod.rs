//! A reference in-process runner, used ONLY to test the suite.
//!
//! Conformance runs against real runners. This fake exists so the suite's
//! own tests can show that each case passes a runner that keeps the
//! contract and fails one that breaks it, so it must be no more capable than
//! a real runner. The property it models:
//!
//! > A runner that hosts sessions from a durable log of records (start,
//! > send, assistant step, dispatch intent, tool result, terminal), rebuilds
//! > every session from that log on restart, and resumes what was cut: a
//! > call with an intent and no result is closed `outcome_unknown` and its
//! > run sealed `interrupted`; a step without an intent either resumes or
//! > is sealed `interrupted`, according to the test's recovery policy.
//!
//! It runs a session's run synchronously inside the request that starts it,
//! stopping only at a call the scripted tool provider holds. Everything it
//! knows reaches the suite only as replies on a route. Its kills are fault
//! hooks inside one process, so it reports [`KillMechanism::FaultHook`];
//! `claim_process_kill` makes it report a real process kill instead, which
//! it is not, so the tests can reach the verdicts that need one.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use cortexkit_role_llm_runner_conformance::{
    harness::{
        Harness, HarnessError, KillMechanism, KillPoint, KillReport, PointDeclaration, RouteStamp,
        Trigger,
    },
    wire::{
        baseline::{Baseline, BaselineItem, BaselineReply},
        capabilities as groups,
        describe::{Major, Retention, RoleDescribe},
        errors, ops, points,
        read::{
            HeadMeta, LastRunState, ReadMessage, ReadMode, ReadPage, ReadRequest, ReadView,
            RunAttribution, ToolCallAttribution,
        },
        run::{FinalMessage, RunResult, RunResultRequest},
        send::{
            delivered_as, delivery_unsupported_detail, Delivered, SendReply, SendRequest,
            DELIVERY_FIELD,
        },
        subscribe::{SubscribeFrom, SubscribeRequest},
        PROVIDES,
    },
    Capability, LlmRunnerSubject, Reply, RouteFailure, RunnerRoute, Script, ScriptedPart,
    SubscribeOutcome, SCRIPTED_TOOL,
};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

mod compaction;
pub use compaction::CompactionDefect;

pub const TOOL_MODULE: &str = "fake.tools";
pub const RETENTION_MAX: u64 = 4;
const RETENTION_DELETE_MS: u64 = 100;
// Self-tests neutralize this switch to prove their named failure assertions
// observe the deliberate runner defects rather than a precomputed verdict.
const RETENTION_DEFECTS_ENABLED: bool = true;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RetentionDefect {
    #[default]
    None,
    Ignore,
    Empty,
    Timestamp,
    ReuseLineage,
    Inherit,
    ShortenIgnored,
    ShortenReplaces,
    EqualRefused,
    LengthenAccepted,
    LateOptInAccepted,
    ZeroAccepted,
    AboveMaxAccepted,
    MaxDetailMissing,
    WithoutGroupIgnored,
    ExpireHeld,
    ClockNotRestarted,
    /// Moves its retention clock forward but never deletes a session's content once it expires.
    NeverPurge,
    TombstoneForgotten,
    TitleLeak,
    PromptLeak,
    ToolLeak,
    Status,
}
const DEFAULT_MAX_BYTES: u64 = 64 * 1024;
const MAXIMUM_MAX_BYTES: u64 = 1024 * 1024;
const DEFAULT_LIMIT: u64 = 100;
const MAXIMUM_LIMIT: u64 = 1000;
const READ_FIELDS: &[&str] = &[
    "from_ordinal",
    "after_mid",
    "lineage_id",
    "limit",
    "max_bytes",
    "include_originals",
    "view",
];

/// Deliberate contract breaks: each field makes the fake violate one rule a
/// case must catch.
#[derive(Clone, Copy, Debug, Default)]
pub struct Defects {
    pub compaction: CompactionDefect,
    /// A read with no cursor answers the oldest page instead of the newest.
    pub tail_returns_oldest: bool,
    /// A call's key is minted from the model's tool_call_id, so a model id
    /// used in two steps gets the same key twice.
    pub call_key_from_model_id: bool,
    /// Resume seals a run cut by a kill `cancelled` instead of
    /// `interrupted`.
    pub cancel_cut_runs: bool,
    /// A never-sent call in a sealed run falsely reports an unknown outcome
    /// through `indeterminate: true`.
    pub sealed_call_indeterminate: bool,
    /// Sealing leaves the never-sent call dangling in the next model history.
    pub sealed_call_without_result: bool,
    /// Sealing hides attribution but leaves the dangling call in the raw message.
    pub sealed_call_without_attribution: bool,
    /// Resumption invokes a never-sent call twice, breaking at-most-once dispatch.
    pub dispatch_twice_on_resume: bool,
    /// role.describe declares `interrupt`, which the runner does not serve
    /// and the subject does not declare.
    pub claim_unserved_group: bool,
    /// A guaranteed runner falsely answers pending for a steer send.
    pub guaranteed_steer_pending: bool,
    /// Answer `pending` to a steer sent into a running turn held by a tool
    /// call, though a guaranteed runner must never answer `pending`.
    pub held_steer_pending: bool,
    /// Answer `pending` only to the re-send of a steer into a held turn; the
    /// first answer stays correct.
    pub held_steer_retry_pending: bool,
    /// Return a different delivery receipt on the re-send of a steer into a
    /// held turn than on its first answer.
    pub held_steer_retry_unstable: bool,
    /// End the held turn before answering a steer into it, so the steer is
    /// answered while no run is in progress.
    pub held_steer_ends_run: bool,
    /// Omit the receipt on the final re-send after the held run ends.
    pub held_steer_final_absent: bool,
    /// Supply a receipt during the hold, then change its reference after release.
    pub held_steer_after_release_unstable: bool,
    /// A re-send of a delivered steer changes its receipt.
    pub resend_steer_unstable: bool,
    /// Every answer to a send names a submission_id of its own, so a retry
    /// names another one than the send was answered with.
    pub retry_new_submission_id: bool,
    /// A queued send is answered `delivered: step`, and its retries
    /// `pending`: a receipt moving backward.
    pub retry_delivered_step_then_pending: bool,
    /// A queued send is answered `delivered: unknown`, and its retries
    /// `step`: a final receipt changing.
    pub retry_delivered_unknown_then_step: bool,
    /// A send_id reused with another prompt or delivery mode is answered as
    /// a retry of the first send, writing nothing, instead of refused
    /// `send_id_reuse`.
    pub reuse_accepted: bool,
    /// Once a queued send's run has ended, its second retry is answered
    /// `active` while the first was answered `finished`: two retries that
    /// differ in state after nothing could have moved.
    pub retry_state_moves_after_end: bool,
}

/// Recovery of a recorded assistant step whose tool calls were never sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepRecovery {
    #[default]
    Resume,
    SealInterrupted,
    SealDropStep,
    SealDropCall,
}

/// What the subject and every module incarnation share: the scripted model
/// and the scripted tool provider live outside the runner, so they survive
/// its kills.
struct World {
    retention_defect: RetentionDefect,
    retention_delete_ms: u64,
    epoch: tokio::time::Instant,
    retention_clock_advanced_ms: AtomicU64,
    held_paused: bool,
    advertise_status: bool,
    defects: Defects,
    step_recovery: StepRecovery,
    queue_receipt_pending_then_unknown: bool,
    steer_receipt_confirm: bool,
    omit_first_steer_receipt: bool,
    early_held_steer_receipt: Option<Delivered>,
    held_steer_receipts: Mutex<Vec<Option<Delivered>>>,
    /// Counts invalid answers produced by the held-turn steer defects,
    /// so a test can prove its defect actually took effect.
    held_steer_break_answers: AtomicUsize,
    groups: Vec<String>,
    scripts: Mutex<BTreeMap<String, Script>>,
    compaction: Mutex<compaction::Providers>,
    invocations: Mutex<BTreeMap<String, usize>>,
    held: Mutex<BTreeSet<String>>,
    released: Mutex<BTreeSet<String>>,
    /// Every `session.read` and `session.head` request received, served or
    /// not.
    transcript_calls: AtomicUsize,
    /// Every `run.result` request received, served or not.
    run_result_calls: AtomicUsize,
}

impl World {
    fn now(&self) -> u64 {
        1_790_000_000_000
            + self.epoch.elapsed().as_millis() as u64
            + self.retention_clock_advanced_ms.load(Ordering::SeqCst)
    }
    fn serves(&self, group: &str) -> bool {
        self.groups.iter().any(|served| served == group)
    }

    /// The scripted tool provider: count the call, and answer its scripted
    /// result unless the script holds it.
    fn invoke(&self, arguments: &Value) -> Option<Value> {
        let key = arguments.to_string();
        *self
            .invocations
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default() += 1;
        let scripts = self.scripts.lock().unwrap();
        let call = scripts
            .values()
            .flat_map(|script| script.tool_calls())
            .find(|call| &call.arguments == arguments)?;
        if call.hold && !self.released.lock().unwrap().contains(&key) {
            self.held.lock().unwrap().insert(key);
            return None;
        }
        Some(call.result.clone())
    }

    fn result_of(&self, arguments: &Value) -> Value {
        self.scripts
            .lock()
            .unwrap()
            .values()
            .flat_map(|script| script.tool_calls())
            .find(|call| &call.arguments == arguments)
            .map(|call| call.result.clone())
            .unwrap_or(Value::Null)
    }
}

struct Killed;

#[derive(Clone, Debug)]
struct Msg {
    ordinal: u64,
    mid: String,
    body: Value,
    run_id: String,
    /// The keys of the calls an assistant message carries, in order.
    calls: Vec<String>,
}

#[derive(Clone, Debug)]
struct SendRec {
    prompt: String,
    delivery: String,
    run_id: String,
    row_id: String,
    steered_into_running: bool,
    admitted: bool,
    resend_count: usize,
}

#[derive(Clone, Debug)]
struct RunRec {
    run_id: String,
    state: String,
    final_ordinal: Option<u64>,
    final_text: Option<String>,
    error: Option<Value>,
}

#[derive(Clone, Debug)]
struct CallRec {
    tool_call_id: String,
    arguments: Value,
    run_id: String,
    intent: bool,
    result: bool,
}

#[derive(Clone, Debug, Default)]
struct Sess {
    compaction: compaction::State,
    retention: Option<u64>,
    last_activity: u64,
    expired_at: Option<u64>,
    deleted: bool,
    title: Option<String>,
    owner: String,
    lineage: String,
    messages: Vec<Msg>,
    sends: BTreeMap<String, SendRec>,
    runs: Vec<RunRec>,
    calls: BTreeMap<String, CallRec>,
    /// The kind of every record, in order: the durable control events.
    events: Vec<String>,
}

impl Sess {
    fn run(&self, run_id: &str) -> Option<&RunRec> {
        self.runs.iter().find(|run| run.run_id == run_id)
    }

    fn run_mut(&mut self, run_id: &str) -> Option<&mut RunRec> {
        self.runs.iter_mut().find(|run| run.run_id == run_id)
    }

    fn push(&mut self, body: Value, run_id: &str, calls: Vec<String>) -> u64 {
        let ordinal = self.messages.len() as u64;
        self.messages.push(Msg {
            ordinal,
            mid: format!("{}.m{ordinal}", self.lineage),
            body,
            run_id: run_id.to_owned(),
            calls,
        });
        ordinal
    }
}

fn point_of(kind: &str) -> &'static str {
    match kind {
        "start" => points::ADMITTED,
        "send" => points::SEND_RECORDED,
        "step" => points::STEP_RECORDED,
        "intent" => points::DISPATCH_INTENT,
        "result" => points::TOOL_RESULT_RECORDED,
        "terminal" => points::TERMINAL,
        "tombstone" => points::RETENTION_TOMBSTONED,
        "compaction_message" => points::COMPACTION_APPLIED,
        "compaction_fold" => points::FOLD_RECORDED,
        _ => "",
    }
}

fn refuse(code: &str, detail: Option<Value>) -> Reply {
    let mut body = ErrorBody::new(code, format!("refused: {code}"));
    body.detail = detail;
    Reply::Error(body)
}

fn invalid(field: &str) -> Reply {
    refuse(errors::INVALID_PARAMS, Some(json!({ "field": field })))
}

fn respond<T: serde::Serialize>(value: T) -> Reply {
    Reply::Response(serde_json::to_value(value).expect("fake replies serialize"))
}

fn baseline() -> Baseline {
    let mut capabilities = Map::new();
    capabilities.insert(groups::session::MID_SESSION_APPENDS.into(), json!(true));
    Baseline::new(
        vec![BaselineItem::new(TOOL_MODULE, "item-digest-1")],
        "composition-1",
        vec![SCRIPTED_TOOL.to_owned()],
    )
    .with_session_capabilities(capabilities)
}

/// One running incarnation: its log on disk and what it rebuilt from it.
pub struct Module {
    world: Arc<World>,
    log: PathBuf,
    alive: AtomicBool,
    kill_hook: Mutex<Option<String>>,
    hook_fired: AtomicBool,
    sessions: Mutex<BTreeMap<String, Sess>>,
}

impl Module {
    fn sweep(&self, session: &str) -> Result<(), Killed> {
        let state = self.sessions.lock().unwrap().get(session).cloned();
        let Some(state) = state else {
            return Ok(());
        };
        if state.expired_at.is_some() {
            self.finish_deletion(session);
            return Ok(());
        }
        if self.world.retention_defect == RetentionDefect::Ignore {
            return Ok(());
        }
        if self.world.retention_defect != RetentionDefect::ExpireHeld
            && state
                .runs
                .iter()
                .any(|run| run.state == "active" || run.state == "paused")
        {
            return Ok(());
        }
        if let Some(seconds) = state.retention {
            let expiry = state.last_activity + seconds * 1000;
            if self.world.now() >= expiry {
                self.commit(session, json!({"kind":"tombstone", "expired_at_ms":expiry}))?;
                self.finish_deletion(session);
            }
        }
        Ok(())
    }

    fn finish_deletion(&self, session: &str) {
        let state = self.sessions.lock().unwrap().get(session).cloned();
        let Some(state) = state else {
            return;
        };
        if state.expired_at.is_none()
            || state.deleted
            || self.world.retention_defect == RetentionDefect::NeverPurge
        {
            return;
        }
        // The fake really removes the session's disk records as well as its
        // replay projection, retaining a content-free tombstone and completion.
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let mut kept: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|r: &Value| r["session"] != json!(session) || r["kind"] == json!("tombstone"))
            .collect();
        let mut record = json!({"kind":"deleted", "session":session, "at":self.world.now()});
        if self.world.retention_defect == RetentionDefect::TitleLeak {
            record["title"] = json!(state.title);
        }
        if self.world.retention_defect == RetentionDefect::PromptLeak {
            record["leaked"] = json!(state
                .messages
                .iter()
                .find(|m| m.body["role"] == "user")
                .map(|m| m.body.clone()));
        }
        if self.world.retention_defect == RetentionDefect::ToolLeak {
            record["leaked"] = json!(state
                .messages
                .iter()
                .find(|m| m.body["role"] == "tool")
                .map(|m| m.body.clone()));
        }
        kept.push(record.clone());
        let mut file = std::fs::File::create(&self.log).unwrap();
        for value in kept {
            writeln!(file, "{value}").unwrap();
        }
        file.sync_all().unwrap();
        self.apply(&record);
    }

    fn expiry_reply(&self, session: &str) -> Option<Reply> {
        let sessions = self.sessions.lock().unwrap();
        let expiry = sessions.get(session)?.expired_at?;
        if self.world.retention_defect == RetentionDefect::Empty {
            return None;
        }
        Some(refuse(
            errors::EXPIRED,
            Some(
                json!({"expired_at_ms":expiry + u64::from(self.world.retention_defect == RetentionDefect::Timestamp)}),
            ),
        ))
    }

    fn listing(&self, session: &str) -> Reply {
        let sessions = self.sessions.lock().unwrap();
        let title = sessions.get(session).and_then(|s| s.title.clone());
        let records: Vec<Value> = std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|r: &Value| r["session"] == json!(session))
            .collect();
        respond(
            json!({"title":title, "leaked":records.iter().filter_map(|r| r.get("leaked")).collect::<Vec<_>>()}),
        )
    }

    fn open(root: &Path, world: Arc<World>) -> Result<Arc<Self>, HarnessError> {
        std::fs::create_dir_all(root).map_err(|e| HarnessError::new(e.to_string()))?;
        let module = Arc::new(Self {
            world,
            log: root.join("records.jsonl"),
            alive: AtomicBool::new(true),
            kill_hook: Mutex::new(None),
            hook_fired: AtomicBool::new(false),
            sessions: Mutex::new(BTreeMap::new()),
        });
        if let Ok(text) = std::fs::read_to_string(&module.log) {
            for line in text.lines() {
                let record: Value = serde_json::from_str(line)
                    .map_err(|e| HarnessError::new(format!("corrupt log: {e}")))?;
                module.apply(&record);
            }
        }
        let _ = module.resume();
        let pending: Vec<String> = module
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, s)| s.expired_at.is_some() && !s.deleted)
            .map(|(n, _)| n.clone())
            .collect();
        for name in pending {
            if module.world.retention_defect == RetentionDefect::TombstoneForgotten {
                let mut sessions = module.sessions.lock().unwrap();
                sessions.get_mut(&name).unwrap().expired_at = None;
                sessions.get_mut(&name).unwrap().retention = None;
            } else {
                module.finish_deletion(&name);
            }
        }
        Ok(module)
    }

    /// Resume every run a kill cut, after writing one resume record per
    /// session it touches.
    fn resume(&self) -> Result<(), Killed> {
        let cut: Vec<(String, String)> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(name, sess)| {
                sess.runs
                    .iter()
                    .filter(|run| run.state == "active")
                    .map(move |run| (name.clone(), run.run_id.clone()))
            })
            .collect();
        for (session, run_id) in cut {
            self.commit(&session, json!({ "kind": "resume" }))?;
            if self.seal_compaction_setup_crash(&session, &run_id)? {
                continue;
            }
            let open: Vec<String> = self.sessions.lock().unwrap()[&session]
                .calls
                .iter()
                .filter(|(_, call)| call.run_id == run_id && call.intent && !call.result)
                .map(|(key, _)| key.clone())
                .collect();
            if open.is_empty() {
                let pending: Vec<(String, Value)> = self.sessions.lock().unwrap()[&session]
                    .calls
                    .iter()
                    .filter(|(_, call)| call.run_id == run_id && !call.intent)
                    .map(|(key, call)| (key.clone(), call.arguments.clone()))
                    .collect();
                if !pending.is_empty() && self.world.step_recovery != StepRecovery::Resume {
                    if matches!(
                        self.world.step_recovery,
                        StepRecovery::SealDropStep | StepRecovery::SealDropCall
                    ) {
                        self.commit(
                            &session,
                            json!({ "kind": "drop_step", "run_id": run_id,
                            "keep_text": self.world.step_recovery == StepRecovery::SealDropCall }),
                        )?;
                    } else if !self.world.defects.sealed_call_without_result {
                        for (key, _) in &pending {
                            self.commit(
                                &session,
                                json!({ "kind": "result", "call_key": key,
                                "content": "not run" }),
                            )?;
                        }
                    }
                    self.commit(
                        &session,
                        json!({ "kind": "terminal", "run_id": run_id,
                        "state": "interrupted" }),
                    )?;
                } else {
                    if self.world.defects.dispatch_twice_on_resume {
                        for (_, arguments) in pending {
                            self.world.invoke(&arguments);
                        }
                    }
                    self.drive(&session, &run_id)?;
                }
                continue;
            }
            for key in open {
                self.commit(
                    &session,
                    json!({ "kind": "result", "call_key": key, "content": null,
                            "reason": errors::tool_result_reasons::OUTCOME_UNKNOWN }),
                )?;
            }
            let state = if self.world.defects.cancel_cut_runs {
                "cancelled"
            } else {
                "interrupted"
            };
            self.commit(
                &session,
                json!({ "kind": "terminal", "run_id": run_id, "state": state }),
            )?;
        }
        Ok(())
    }

    /// Append one durable record, then apply it. A kill armed for the
    /// record's point takes effect right after it is on disk: nothing later
    /// is written or done.
    fn commit(&self, session: &str, mut record: Value) -> Result<(), Killed> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(Killed);
        }
        record["session"] = json!(session);
        record["at"] = json!(self.world.now());
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .expect("fake log opens");
        writeln!(file, "{record}").expect("fake log writes");
        file.sync_all().expect("fake log syncs");
        let point = point_of(record["kind"].as_str().unwrap_or_default());
        if !point.is_empty() && self.kill_hook.lock().unwrap().as_deref() == Some(point) {
            self.alive.store(false, Ordering::SeqCst);
            self.hook_fired.store(true, Ordering::SeqCst);
            return Err(Killed);
        }
        self.apply(&record);
        Ok(())
    }

    fn apply(&self, record: &Value) {
        let text = |key: &str| record[key].as_str().unwrap_or_default().to_owned();
        let session = text("session");
        let kind = text("kind");
        let mut sessions = self.sessions.lock().unwrap();
        if kind == "start" {
            sessions.insert(
                session.clone(),
                Sess {
                    owner: text("owner"),
                    lineage: text("lineage"),
                    ..Sess::default()
                },
            );
        }
        let Some(sess) = sessions.get_mut(&session) else {
            return;
        };
        sess.events.push(kind.clone());
        compaction::apply(&mut sess.compaction, record, self.world.defects.compaction);
        if matches!(kind.as_str(), "send" | "steer" | "step" | "terminal")
            && (self.world.retention_defect != RetentionDefect::ClockNotRestarted
                || sess.last_activity == 0)
        {
            sess.last_activity = record["at"].as_u64().unwrap_or_default();
        }
        match kind.as_str() {
            "tombstone" => {
                sess.expired_at = record["expired_at_ms"].as_u64();
            }
            "deleted" => {
                sess.deleted = true;
                sess.messages.clear();
                sess.calls.clear();
                sess.sends.clear();
                sess.runs.clear();
                sess.title = record["title"].as_str().map(str::to_owned);
            }
            "send" | "steer" => {
                if let Some(retention) = record["retention"].as_u64() {
                    sess.retention = Some(retention);
                }
                if let Some(title) = record["title"].as_str() {
                    sess.title = Some(title.into());
                }
                let run_id = text("run_id");
                let ordinal = sess.push(
                    json!({ "role": "user", "text": text("prompt") }),
                    &run_id,
                    Vec::new(),
                );
                sess.sends.insert(
                    text("send_id"),
                    SendRec {
                        prompt: text("prompt"),
                        delivery: text("delivery"),
                        run_id: run_id.clone(),
                        row_id: sess.messages[ordinal as usize].mid.clone(),
                        steered_into_running: kind == "steer",
                        admitted: record["admitted"] == json!(true),
                        resend_count: 0,
                    },
                );
                if kind == "send" {
                    sess.runs.push(RunRec {
                        run_id,
                        state: "active".into(),
                        final_ordinal: None,
                        final_text: None,
                        error: None,
                    });
                }
            }
            "step" => {
                let run_id = text("run_id");
                let mut keys = Vec::new();
                for call in record["calls"].as_array().into_iter().flatten() {
                    let key = call["call_key"].as_str().unwrap_or_default().to_owned();
                    keys.push(key.clone());
                    sess.calls.insert(
                        key,
                        CallRec {
                            tool_call_id: call["tool_call_id"].as_str().unwrap_or_default().into(),
                            arguments: call["arguments"].clone(),
                            run_id: run_id.clone(),
                            intent: false,
                            result: false,
                        },
                    );
                }
                sess.push(
                    json!({ "role": "assistant", "parts": record["parts"] }),
                    &run_id,
                    keys,
                );
            }
            "drop_step" => {
                let run_id = text("run_id");
                sess.calls.retain(|_, call| call.run_id != run_id);
                if record["keep_text"] == json!(true) {
                    for message in sess
                        .messages
                        .iter_mut()
                        .filter(|m| m.run_id == run_id && !m.calls.is_empty())
                    {
                        message.calls.clear();
                        if let Some(parts) = message.body["parts"].as_array_mut() {
                            parts.retain(|part| part["type"] != "tool_call");
                        }
                    }
                } else {
                    sess.messages
                        .retain(|m| m.run_id != run_id || m.calls.is_empty());
                }
            }
            "intent" => {
                if let Some(call) = sess.calls.get_mut(&text("call_key")) {
                    call.intent = true;
                }
            }
            "result" => {
                let key = text("call_key");
                let run_id = sess
                    .calls
                    .get(&key)
                    .map(|call| call.run_id.clone())
                    .unwrap_or_default();
                if let Some(call) = sess.calls.get_mut(&key) {
                    call.result = true;
                }
                let mut body =
                    json!({ "role": "tool", "call_key": key, "content": record["content"] });
                if let Some(reason) = record.get("reason") {
                    body["reason"] = reason.clone();
                }
                sess.push(body, &run_id, Vec::new());
            }
            "terminal" => {
                let run_id = text("run_id");
                let final_ordinal = sess
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.run_id == run_id && m.body["role"] == "assistant")
                    .map(|m| m.ordinal);
                if let Some(run) = sess.run_mut(&run_id) {
                    run.state = text("state");
                    run.error = record.get("error").cloned();
                    if run.state == "completed" {
                        run.final_ordinal = final_ordinal;
                        run.final_text = record["final_text"].as_str().map(str::to_owned);
                    }
                }
            }
            _ => {}
        }
    }

    /// Continue run `run_id` of `session`: dispatch the calls of its last
    /// step, then ask the model for the next turn, and repeat until the run
    /// ends or the tool provider holds a call.
    fn drive(&self, session: &str, run_id: &str) -> Result<(), Killed> {
        loop {
            let sess = self.sessions.lock().unwrap()[session].clone();
            let mine: Vec<(&String, &CallRec)> = sess
                .calls
                .iter()
                .filter(|(_, call)| call.run_id == run_id)
                .collect();
            if let Some((key, call)) = mine.iter().find(|(_, call)| !call.intent) {
                self.commit(session, json!({ "kind": "intent", "call_key": key }))?;
                match self.world.invoke(&call.arguments) {
                    Some(content) => self.commit(
                        session,
                        json!({ "kind": "result", "call_key": key, "content": content }),
                    )?,
                    None => return Ok(()),
                }
                continue;
            }
            if mine.iter().any(|(_, call)| call.intent && !call.result) {
                return Ok(());
            }
            if sess.run(run_id).is_none_or(|run| run.state != "active") {
                return Ok(());
            }
            // The model consumed a turn even if recovery removed it from history.
            let turn_index = sess.events.iter().filter(|kind| *kind == "step").count();
            if !self.prepare_compaction(session, run_id, turn_index)? {
                return Ok(());
            }
            let turn = self
                .world
                .scripts
                .lock()
                .unwrap()
                .get(session)
                .and_then(|script| script.turns.get(turn_index).cloned());
            let Some(turn) = turn else {
                return self.commit(
                    session,
                    json!({ "kind": "terminal", "run_id": run_id, "state": "error" }),
                );
            };
            let ordinal = sess.messages.len();
            let mut parts = Vec::new();
            let mut calls = Vec::new();
            for (index, part) in turn.parts.iter().enumerate() {
                match part {
                    ScriptedPart::Text(text) => parts.push(json!({ "type": "text", "text": text })),
                    ScriptedPart::Reasoning(text) => {
                        parts.push(json!({ "type": "reasoning", "text": text }))
                    }
                    ScriptedPart::ToolCall(call) => {
                        let key = if self.world.defects.call_key_from_model_id {
                            format!("key-{}", call.tool_call_id)
                        } else {
                            format!("{}.{ordinal}.{index}", sess.lineage)
                        };
                        parts.push(json!({ "type": "tool_call", "id": call.tool_call_id,
                                           "tool": call.tool, "arguments": call.arguments }));
                        calls.push(json!({ "call_key": key, "tool_call_id": call.tool_call_id,
                                           "arguments": call.arguments }));
                    }
                }
            }
            let no_calls = calls.is_empty();
            self.commit(
                session,
                json!({ "kind": "step", "run_id": run_id, "parts": parts, "calls": calls }),
            )?;
            if no_calls {
                let text: String = turn.text_parts().collect();
                return self.commit(
                    session,
                    json!({ "kind": "terminal", "run_id": run_id, "state": "completed",
                            "final_text": text }),
                );
            }
        }
    }

    /// Record the scripted result of the held call with `arguments`, then
    /// continue its run.
    fn complete_held(&self, arguments: &Value) -> Result<(), Killed> {
        let found = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find_map(|(name, sess)| {
                sess.calls
                    .iter()
                    .find(|(_, call)| &call.arguments == arguments && call.intent && !call.result)
                    .map(|(key, call)| (name.clone(), key.clone(), call.run_id.clone()))
            });
        let Some((session, key, run_id)) = found else {
            return Ok(());
        };
        let content = self.world.result_of(arguments);
        self.commit(
            &session,
            json!({ "kind": "result", "call_key": key, "content": content }),
        )?;
        self.drive(&session, &run_id)
    }

    fn handle(
        &self,
        stamp: &RouteStamp,
        session: &str,
        method: &str,
        params: Value,
    ) -> Result<Reply, RouteFailure> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(RouteFailure::new("the module is not running"));
        }
        self.sweep(session)
            .map_err(|Killed| RouteFailure::new("the module was killed"))?;
        if matches!(
            method,
            ops::SESSION_READ | ops::SESSION_HEAD | ops::RUN_RESULT | "run.status"
        ) {
            if let Some(reply) = self.expiry_reply(session) {
                if method != "run.status" || self.world.retention_defect != RetentionDefect::Status
                {
                    return Ok(reply);
                }
            }
        }
        let reply = match method {
            "session.list" | "session.export" => Ok(self.listing(session)),
            "run.status" if self.world.advertise_status => Ok(self.run_result(session, params)),
            ops::SESSION_READ | ops::SESSION_HEAD => {
                self.world.transcript_calls.fetch_add(1, Ordering::SeqCst);
                if !self.world.serves(groups::TRANSCRIPT_READS) {
                    // A runner that does not declare transcript_reads does
                    // not serve its ops.
                    Ok(refuse("unknown_method", Some(json!({ "method": method }))))
                } else if method == ops::SESSION_READ {
                    Ok(self.read(session, &params))
                } else {
                    Ok(self.head(session, &params))
                }
            }
            ops::ROLE_DESCRIBE => Ok(self.describe()),
            ops::SESSION_BASELINE => Ok(self.baseline(stamp, session, &params)),
            ops::SESSION_SEND => self.send(stamp, session, params),
            ops::RUN_RESULT => {
                self.world.run_result_calls.fetch_add(1, Ordering::SeqCst);
                if self.world.serves(groups::RUN_OPS) {
                    Ok(self.run_result(session, params))
                } else {
                    // A runner that does not declare run_ops does not
                    // serve its ops.
                    Ok(refuse("unknown_method", Some(json!({ "method": method }))))
                }
            }
            other => Ok(refuse("unknown_method", Some(json!({ "method": other })))),
        };
        reply.map_err(|Killed| RouteFailure::new("the module was killed"))
    }

    fn describe(&self) -> Reply {
        let mut capabilities = self.world.groups.clone();
        if self.world.defects.claim_unserved_group {
            capabilities.push(groups::INTERRUPT.to_owned());
        }
        let mut served: Vec<String> = vec![ops::ROLE_DESCRIBE.into(), ops::SESSION_BASELINE.into()];
        if self.world.serves(groups::RETENTION) {
            served.extend(["session.list".into(), "session.export".into()]);
        }
        if self.world.advertise_status {
            served.push("run.status".into());
        }
        for group in &capabilities {
            for op in groups::group_ops(group).unwrap_or_default() {
                if !served.iter().any(|s| s == op) {
                    served.push((*op).to_owned());
                }
            }
        }
        let mut describe =
            RoleDescribe::new(vec![Major::new(PROVIDES, served, "alpha")], "fake-runner-1")
                .with_capabilities(capabilities)
                .with_session_capabilities_from(groups::source::ADMISSION)
                .with_max_bytes(DEFAULT_MAX_BYTES, MAXIMUM_MAX_BYTES);
        if self.world.steer_receipt_confirm {
            describe = describe.with_steer_receipt("confirm");
        }
        if self.world.serves(groups::RETENTION) {
            describe = describe.with_retention(RETENTION_MAX, self.world.retention_delete_ms);
        }
        respond(describe)
    }

    fn baseline(&self, stamp: &RouteStamp, session: &str, params: &Value) -> Reply {
        if let Some(field) = unknown_field(params, &[]) {
            return invalid(&field);
        }
        let sessions = self.sessions.lock().unwrap();
        match sessions.get(session) {
            None => respond(BaselineReply::not_yet()),
            Some(sess) if sess.owner != stamp.principal => refuse(
                errors::SCOPE_OWNER_MISMATCH,
                Some(json!({ "accepted": sess.owner })),
            ),
            Some(_) => respond(BaselineReply::ready(baseline())),
        }
    }

    fn send_reply(&self, session: &str, send: &SendRec) -> Reply {
        let sessions = self.sessions.lock().unwrap();
        let run = sessions[session]
            .run(&send.run_id)
            .expect("a send's run exists");
        // The defect answers a run that has ended as still active, on the
        // second retry only, so the first and second retries differ.
        let state_moved = self.world.defects.retry_state_moves_after_end
            && send.delivery == groups::QUEUE
            && send.resend_count == 2;
        let mut reply = if run.state == "active" || state_moved {
            SendReply::new("active").with_run_id(&run.run_id)
        } else {
            SendReply::new("finished")
                .with_run_id(&run.run_id)
                .with_reason(&run.state)
        };
        if send.admitted {
            reply = reply.with_baseline(baseline());
        }
        let defects = &self.world.defects;
        if defects.retry_new_submission_id {
            reply = reply.with_submission_id(format!("sub-{}-{}", run.run_id, send.resend_count));
        }
        if send.delivery == groups::QUEUE {
            // The send itself is answered with the first receipt, every
            // retry with the second.
            let receipts = if defects.retry_delivered_step_then_pending {
                Some((delivered_as::STEP, delivered_as::PENDING))
            } else if defects.retry_delivered_unknown_then_step {
                Some((delivered_as::UNKNOWN, delivered_as::STEP))
            } else if self.world.queue_receipt_pending_then_unknown {
                Some((delivered_as::PENDING, delivered_as::UNKNOWN))
            } else {
                None
            };
            if let Some((sent, retried)) = receipts {
                let r#as = if send.resend_count == 0 {
                    sent
                } else {
                    retried
                };
                reply = reply.with_delivered(Delivered::new(r#as));
            }
        }
        if send.delivery == groups::STEER {
            let held_turn = send.steered_into_running
                && run.state == "active"
                && sessions[session]
                    .calls
                    .values()
                    .any(|call| call.run_id == run.run_id && call.intent && !call.result);
            let held_pending = held_turn
                && ((defects.held_steer_pending && send.resend_count == 0)
                    || (defects.held_steer_retry_pending && send.resend_count > 0));
            let held_unstable =
                held_turn && defects.held_steer_retry_unstable && send.resend_count > 0;
            let after_release = send.steered_into_running && run.state != "active";
            let final_absent = after_release && defects.held_steer_final_absent;
            let final_unstable = after_release && defects.held_steer_after_release_unstable;
            if held_pending || held_unstable || final_absent || final_unstable {
                self.world
                    .held_steer_break_answers
                    .fetch_add(1, Ordering::SeqCst);
            }
            if held_pending || self.world.defects.guaranteed_steer_pending {
                reply = reply.with_delivered(Delivered::new(delivered_as::PENDING));
            } else if final_absent {
                // Deliberately leave the settled re-send without a receipt.
            } else if held_unstable || final_unstable {
                reply = reply.with_delivered(
                    Delivered::new(delivered_as::STEP)
                        .with_ref(format!("{}-unstable", send.row_id)),
                );
            } else if held_turn {
                // A real guaranteed runner cannot yet know which step or turn
                // will render the steer. The unstable-receipt fixtures seed
                // a receipt early so the check must compare its reference.
                if defects.held_steer_retry_unstable || defects.held_steer_after_release_unstable {
                    reply = reply
                        .with_delivered(Delivered::new(delivered_as::STEP).with_ref(&send.row_id));
                } else if let Some(receipt) = &self.world.early_held_steer_receipt {
                    reply = reply.with_delivered(receipt.clone());
                }
            } else if let Some(receipt) = self
                .world
                .early_held_steer_receipt
                .as_ref()
                .filter(|_| send.steered_into_running)
            {
                reply = reply.with_delivered(receipt.clone());
            } else if !send.steered_into_running
                && send.resend_count > 1
                && self.world.defects.resend_steer_unstable
            {
                reply = reply.with_delivered(
                    Delivered::new(delivered_as::TURN).with_ref(format!("{}-unstable", run.run_id)),
                );
            } else if !self.world.omit_first_steer_receipt || send.resend_count > 0 {
                let receipt = if send.steered_into_running {
                    Delivered::new(delivered_as::STEP).with_ref(&send.row_id)
                } else {
                    Delivered::new(delivered_as::TURN).with_ref(&run.run_id)
                };
                reply = reply.with_delivered(receipt);
            }
            if send.steered_into_running {
                self.world
                    .held_steer_receipts
                    .lock()
                    .unwrap()
                    .push(reply.delivered.clone());
            }
        }
        respond(reply)
    }

    fn send(&self, stamp: &RouteStamp, session: &str, params: Value) -> Result<Reply, Killed> {
        if params
            .get("retention")
            .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
        {
            return Ok(invalid("retention"));
        }
        if let Some(delivery) = params.get(DELIVERY_FIELD) {
            if !matches!(delivery.as_str(), Some("queue" | "steer" | "interrupt")) {
                return Ok(invalid(DELIVERY_FIELD));
            }
        }
        let mut request: SendRequest = match serde_json::from_value(params) {
            Ok(request) => request,
            Err(_) => return Ok(invalid("prompt")),
        };
        let delivery = request.delivery().as_str().to_owned();
        let old = self.sessions.lock().unwrap().get(session).cloned();
        let mut existing = old.clone().filter(|s| s.expired_at.is_none());
        let defect = self.world.retention_defect;
        if defect == RetentionDefect::WithoutGroupIgnored && !self.world.serves(groups::RETENTION) {
            request.retention = None;
        }
        let cap = self
            .world
            .serves(groups::RETENTION)
            .then(|| Retention::new(RETENTION_MAX, self.world.retention_delete_ms));
        let previous = existing.as_ref().map(|s| s.retention);
        let accept_bad = match defect {
            RetentionDefect::ZeroAccepted => request.retention == Some(0),
            RetentionDefect::AboveMaxAccepted => {
                request.retention.is_some_and(|n| n > RETENTION_MAX)
            }
            RetentionDefect::LengthenAccepted => previous
                .flatten()
                .zip(request.retention)
                .is_some_and(|(a, b)| b > a),
            RetentionDefect::LateOptInAccepted => previous == Some(None),
            _ => false,
        };
        if !accept_bad {
            if let Err(mut detail) = request.check_retention(cap.as_ref(), previous) {
                if defect == RetentionDefect::MaxDetailMissing {
                    detail.max_seconds = None;
                }
                return Ok(refuse(
                    errors::INVALID_PARAMS,
                    Some(serde_json::to_value(detail).unwrap()),
                ));
            }
        }
        if defect == RetentionDefect::EqualRefused
            && request.retention.is_some()
            && request.retention == previous.flatten()
        {
            return Ok(invalid("retention"));
        }
        if defect == RetentionDefect::ShortenIgnored && existing.is_some() {
            request.retention = None;
        }
        if defect == RetentionDefect::ShortenReplaces
            && request
                .retention
                .zip(previous.flatten())
                .is_some_and(|(a, b)| a < b)
        {
            existing = None;
        }
        if defect == RetentionDefect::Inherit
            && old.as_ref().is_some_and(|s| s.expired_at.is_some())
            && request.retention.is_none()
        {
            request.retention = old.as_ref().and_then(|s| s.retention);
        }
        if let Some(sess) = &existing {
            if sess.owner != stamp.principal {
                return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
            }
            if let Some(send) = sess.sends.get(&request.send_id) {
                if send.prompt != request.prompt && !self.world.defects.reuse_accepted {
                    return Ok(refuse(
                        errors::SEND_ID_REUSE,
                        Some(json!({ "field": "prompt" })),
                    ));
                }
                if send.delivery != delivery && !self.world.defects.reuse_accepted {
                    return Ok(refuse(
                        errors::SEND_ID_REUSE,
                        Some(json!({ "field": DELIVERY_FIELD })),
                    ));
                }
                let mut sessions = self.sessions.lock().unwrap();
                let sess = sessions.get_mut(session).unwrap();
                let send = sess.sends.get_mut(&request.send_id).unwrap();
                send.resend_count += 1;
                let send = send.clone();
                drop(sessions);
                return Ok(self.send_reply(session, &send));
            }
            if let Some(run) = sess.runs.iter().find(|run| run.state == "active") {
                if delivery != groups::STEER || !self.world.serves(groups::STEER) {
                    return Ok(refuse(errors::TRANSIENT, None));
                }
                // A steer accepted during a run waits for a step boundary.
                // Unlike a send to an idle session, accepting it must not
                // start a turn of its own or claim it has already rendered.
                self.commit(
                    session,
                    json!({ "kind": "steer", "send_id": request.send_id,
                            "prompt": request.prompt, "delivery": delivery,
                            "run_id": run.run_id, "admitted": false, "retention":request.retention }),
                )?;
                if self.world.defects.held_steer_ends_run {
                    self.world
                        .held_steer_break_answers
                        .fetch_add(1, Ordering::SeqCst);
                    self.commit(
                        session,
                        json!({ "kind": "terminal", "run_id": run.run_id, "state": "interrupted" }),
                    )?;
                }
                let send = self.sessions.lock().unwrap()[session].sends[&request.send_id].clone();
                return Ok(self.send_reply(session, &send));
            }
        }
        if let Err(mode) = request.check_delivery(&self.world.groups) {
            return Ok(refuse(
                errors::DELIVERY_UNSUPPORTED,
                Some(delivery_unsupported_detail(mode)),
            ));
        }
        let admitted = existing.is_none();
        if admitted {
            let lineage = if old.is_some() && defect != RetentionDefect::ReuseLineage {
                format!("lin-{session}-{}", self.world.now())
            } else {
                format!("lin-{session}")
            };
            self.commit(
                session,
                json!({ "kind": "start", "owner": stamp.principal, "lineage": lineage }),
            )?;
        }
        let run_id = format!(
            "run-{}",
            self.sessions.lock().unwrap()[session].runs.len() + 1
        );
        self.commit(
            session,
            json!({ "kind": "send", "send_id": request.send_id, "prompt": request.prompt,
                    "delivery": delivery, "run_id": run_id, "admitted": admitted,
                    "retention":request.retention, "title":request.runner_params.get("title") }),
        )?;
        self.drive(session, &run_id)?;
        let send = self.sessions.lock().unwrap()[session].sends[&request.send_id].clone();
        Ok(self.send_reply(session, &send))
    }

    fn read(&self, session: &str, params: &Value) -> Reply {
        if let Some(field) = unknown_field(params, READ_FIELDS) {
            return invalid(&field);
        }
        let request: ReadRequest = match serde_json::from_value(params.clone()) {
            Ok(request) => request,
            Err(_) => return invalid("view"),
        };
        let mode = match request.mode() {
            Ok(mode) => mode,
            Err(field) => return invalid(field),
        };
        if request.view == Some(ReadView::Model) && !self.world.serves(groups::MODEL_VIEW) {
            return invalid("view");
        }
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            if request.lineage_id.is_some() {
                return refuse(errors::LINEAGE_CHANGED, None);
            }
            return if request.view == Some(ReadView::Model) {
                respond(cortexkit_role_llm_runner::read::ModelPage::no_lineage())
            } else {
                respond(ReadPage::no_lineage())
            };
        };
        if request
            .lineage_id
            .as_ref()
            .is_some_and(|lineage| lineage != &sess.lineage)
        {
            return refuse(errors::LINEAGE_CHANGED, None);
        }
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAXIMUM_LIMIT) as usize;
        if request.view == Some(ReadView::Model) {
            return self.model_read(sess, mode, &request);
        }
        let cap = request
            .max_bytes
            .unwrap_or(DEFAULT_MAX_BYTES)
            .min(MAXIMUM_MAX_BYTES) as usize;
        let size = |m: &Msg| m.body.to_string().len();
        let all = &sess.messages;
        let (start, end) = match mode {
            ReadMode::Tail if !self.world.defects.tail_returns_oldest => {
                let mut start = all.len();
                let mut bytes = 0;
                while start > 0 && all.len() - start < limit {
                    let next = size(&all[start - 1]);
                    if start < all.len() && bytes + next > cap {
                        break;
                    }
                    bytes += next;
                    start -= 1;
                }
                (start, all.len())
            }
            ReadMode::Tail | ReadMode::Range { .. } | ReadMode::After { .. } => {
                let start = match mode {
                    ReadMode::Range { from_ordinal } => all
                        .iter()
                        .position(|m| m.ordinal >= from_ordinal)
                        .unwrap_or(all.len()),
                    ReadMode::After { mid, .. } => match all.iter().position(|m| m.mid == mid) {
                        Some(index) => index + 1,
                        None => return refuse(errors::UNKNOWN_MID, None),
                    },
                    _ => 0,
                };
                let mut end = start;
                let mut bytes = 0;
                while end < all.len() && end - start < limit {
                    let next = size(&all[end]);
                    if end > start && bytes + next > cap {
                        break;
                    }
                    bytes += next;
                    end += 1;
                }
                (start, end)
            }
        };
        let messages = all[start..end]
            .iter()
            .map(|m| self.read_message(sess, m))
            .collect();
        let mut page = ReadPage::new(&sess.lineage, messages).with_head(cursor(sess.events.len()));
        if end < all.len() {
            page = page.with_next_from_ordinal(all[end].ordinal);
        }
        respond(page)
    }

    fn read_message(&self, sess: &Sess, m: &Msg) -> ReadMessage {
        let run = sess.run(&m.run_id);
        let is_final = run.is_some_and(|run| run.final_ordinal == Some(m.ordinal));
        let tool_calls = m
            .calls
            .iter()
            .filter(|key| {
                !(self.world.defects.sealed_call_without_attribution
                    && run.is_some_and(|run| run.state == "interrupted")
                    && !sess.calls[*key].intent)
            })
            .map(|key| {
                let call = &sess.calls[key];
                let mut entry = ToolCallAttribution::new(&call.tool_call_id)
                    .with_call_key(key)
                    .with_indeterminate(
                        (call.intent && !call.result)
                            || (self.world.defects.sealed_call_indeterminate
                                && !call.intent
                                && run.is_some_and(|run| run.state == "interrupted")),
                    );
                if call.intent {
                    entry = entry.with_dispatched_to(TOOL_MODULE);
                }
                entry
            })
            .collect();
        ReadMessage::new(m.ordinal, &m.mid, m.body.clone())
            .with_run(RunAttribution {
                run_id: m.run_id.clone(),
                episode: "episode-1".into(),
                is_final,
            })
            .with_tool_calls(tool_calls)
    }

    fn head(&self, session: &str, params: &Value) -> Reply {
        if let Some(field) = unknown_field(params, &[]) {
            return invalid(&field);
        }
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            return respond(HeadMeta::no_lineage());
        };
        if sess.expired_at.is_some() && self.world.retention_defect == RetentionDefect::Empty {
            return respond(HeadMeta::no_lineage());
        }
        let mut head =
            HeadMeta::new(&sess.lineage, sess.last_activity).with_head(cursor(sess.events.len()));
        if let Some(last) = sess.messages.last() {
            head = head.with_last_ordinal(last.ordinal);
        }
        if let Some(run) = sess.runs.last() {
            head = head.with_last_run_state(LastRunState {
                run_id: run.run_id.clone(),
                state: if self.world.held_paused
                    && sess.retention.is_some()
                    && run.state == "active"
                {
                    "paused".into()
                } else {
                    run.state.clone()
                },
                reason: None,
            });
        }
        respond(head)
    }

    fn run_result(&self, session: &str, params: Value) -> Reply {
        let request: RunResultRequest = match serde_json::from_value(params.clone()) {
            Ok(request) => request,
            Err(_) => {
                return invalid(&unknown_field(&params, &["run_id"]).unwrap_or("run_id".into()))
            }
        };
        let sessions = self.sessions.lock().unwrap();
        let Some(run) = sessions.get(session).and_then(|s| s.run(&request.run_id)) else {
            return refuse(errors::UNKNOWN_RUN, None);
        };
        let mut result = RunResult::new(
            &run.run_id,
            if self.world.held_paused
                && sessions[session].retention.is_some()
                && run.state == "active"
            {
                "paused"
            } else {
                &run.state
            },
        );
        if run.state == "completed" {
            let ordinal = run.final_ordinal.unwrap_or_default();
            let mid = sessions[session].messages[ordinal as usize].mid.clone();
            result = result.with_final_message(FinalMessage {
                ordinal,
                mid,
                text: run.final_text.clone().unwrap_or_default(),
            });
        }
        if let Some(error) = &run.error {
            result = result.with_error(error.clone());
        }
        respond(result)
    }

    fn subscribe(&self, session: &str, params: Value) -> SubscribeOutcome {
        let request: SubscribeRequest = match serde_json::from_value(params) {
            Ok(request) => request,
            Err(_) => {
                let Reply::Error(error) = invalid("from") else {
                    unreachable!()
                };
                return SubscribeOutcome::Refused(error);
            }
        };
        let sessions = self.sessions.lock().unwrap();
        let events = sessions
            .get(session)
            .map(|s| s.events.clone())
            .unwrap_or_default();
        let after = match request.attach_point() {
            SubscribeFrom::Start => 0,
            SubscribeFrom::Live => events.len(),
            SubscribeFrom::After(position) => {
                position.get("seq").and_then(Value::as_u64).unwrap_or(0) as usize
            }
        };
        SubscribeOutcome::Events(
            events
                .iter()
                .enumerate()
                .skip(after)
                .flat_map(|(index, kind)| {
                    [
                        json!({ "kind": "display", "delta": kind }),
                        json!({ "kind": "control", "cursor": cursor(index + 1), "record": kind }),
                    ]
                })
                .collect(),
        )
    }
}

/// The opaque position after the `seq`-th durable event.
fn cursor(seq: usize) -> Map<String, Value> {
    let mut position = Map::new();
    position.insert("seq".into(), json!(seq));
    position
}

/// The first member of `params` not in `allowed`.
fn unknown_field(params: &Value, allowed: &[&str]) -> Option<String> {
    params
        .as_object()?
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
        .cloned()
}

pub struct FakeHandle(Arc<Module>);

impl Drop for FakeHandle {
    fn drop(&mut self) {
        self.0.alive.store(false, Ordering::SeqCst);
    }
}

pub struct FakeRoute {
    module: Arc<Module>,
    session: String,
    stamp: RouteStamp,
}

#[async_trait]
impl RunnerRoute for FakeRoute {
    async fn request(&self, method: &str, params: Value) -> Result<Reply, RouteFailure> {
        self.module
            .handle(&self.stamp, &self.session, method, params)
    }

    async fn subscribe(&self, params: Value) -> Result<SubscribeOutcome, RouteFailure> {
        if !self.module.alive.load(Ordering::SeqCst) {
            return Err(RouteFailure::new("the module is not running"));
        }
        Ok(self.module.subscribe(&self.session, params))
    }
}

pub struct FakeSubject {
    pub retention_defect: RetentionDefect,
    pub retention_delete_ms: u64,
    pub held_paused: bool,
    pub advertise_status: bool,
    pub defects: Defects,
    pub step_recovery: StepRecovery,
    pub capabilities: BTreeSet<Capability>,
    pub seal_compaction_setup_crash: bool,
    pub kill_points: Vec<&'static str>,
    /// Report every kill as a real process kill, which this in-process fake
    /// cannot make. Only for tests of the verdict.
    pub claim_process_kill: bool,
    /// Answer a queued send `delivered: pending` and its retries
    /// `delivered: unknown`: a receipt moving forward, which is allowed.
    pub queue_receipt_pending_then_unknown: bool,
    /// Declare `steer_receipt: confirm` (a steer's receipt may be `pending` or
    /// `unknown` until delivery is confirmed) instead of guaranteed delivery,
    /// where every steer receipt is `step` or `turn`.
    pub steer_receipt_confirm: bool,
    /// Exercise the contract's optional receipt on a steer's first answer.
    pub omit_first_steer_receipt: bool,
    /// Supply a final receipt even during the hold, preserving it after release.
    pub early_held_steer_receipt: Option<Delivered>,
    /// How long `pause` waits between the suite's polls. `None` only yields,
    /// so the suite's poll bound passes in no time; a duration makes that
    /// bound real time, as on a runner that polls a live process.
    pub pause_for: Option<Duration>,
    world: Mutex<Option<Arc<World>>>,
    /// Every module started, so a released call reaches the one holding it.
    modules: Mutex<Vec<Arc<Module>>>,
}

/// The capabilities the conforming fake declares: every group it serves,
/// and held tool calls. It leaves `interrupt` and `session_change` undeclared.
pub fn served() -> BTreeSet<Capability> {
    [
        Capability::TranscriptReads,
        Capability::RunOps,
        Capability::DispatchAttribution,
        Capability::Streaming,
        Capability::Compaction,
        Capability::ModelView,
        Capability::Steer,
        Capability::Queue,
        Capability::HoldToolCalls,
    ]
    .into()
}

/// The kill points the fake can reach.
pub const KILL_POINTS: &[&str] = &[
    points::ADMITTED,
    points::SEND_RECORDED,
    points::STEP_RECORDED,
    points::DISPATCH_INTENT,
    points::TOOL_RESULT_RECORDED,
    points::TERMINAL,
    points::RETENTION_TOMBSTONED,
    points::COMPACTION_APPLIED,
    points::FOLD_RECORDED,
];

impl FakeSubject {
    pub fn new(defects: Defects) -> Self {
        Self {
            retention_defect: RetentionDefect::None,
            retention_delete_ms: RETENTION_DELETE_MS,
            held_paused: false,
            advertise_status: false,
            defects,
            step_recovery: StepRecovery::Resume,
            capabilities: served(),
            seal_compaction_setup_crash: false,
            kill_points: KILL_POINTS.to_vec(),
            claim_process_kill: false,
            queue_receipt_pending_then_unknown: false,
            steer_receipt_confirm: false,
            omit_first_steer_receipt: false,
            early_held_steer_receipt: None,
            pause_for: None,
            world: Mutex::new(None),
            modules: Mutex::new(Vec::new()),
        }
    }

    /// The shared world, built on first use from the configuration as it
    /// stands then.
    fn world(&self) -> Arc<World> {
        self.world
            .lock()
            .unwrap()
            .get_or_insert_with(|| {
                Arc::new(World {
                    retention_defect: if RETENTION_DEFECTS_ENABLED {
                        self.retention_defect
                    } else {
                        RetentionDefect::None
                    },
                    retention_delete_ms: self.retention_delete_ms,
                    epoch: tokio::time::Instant::now(),
                    retention_clock_advanced_ms: AtomicU64::new(0),
                    held_paused: self.held_paused,
                    advertise_status: self.advertise_status,
                    defects: self.defects,
                    step_recovery: self.step_recovery,
                    queue_receipt_pending_then_unknown: self.queue_receipt_pending_then_unknown,
                    steer_receipt_confirm: self.steer_receipt_confirm,
                    omit_first_steer_receipt: self.omit_first_steer_receipt,
                    early_held_steer_receipt: self.early_held_steer_receipt.clone(),
                    held_steer_receipts: Mutex::new(Vec::new()),
                    held_steer_break_answers: AtomicUsize::new(0),
                    groups: self
                        .capabilities
                        .iter()
                        .filter_map(|c| c.group())
                        .map(str::to_owned)
                        .collect(),
                    scripts: Mutex::new(BTreeMap::new()),
                    compaction: Mutex::new(compaction::Providers::new(
                        self.seal_compaction_setup_crash,
                    )),
                    invocations: Mutex::new(BTreeMap::new()),
                    held: Mutex::new(BTreeSet::new()),
                    released: Mutex::new(BTreeSet::new()),
                    transcript_calls: AtomicUsize::new(0),
                    run_result_calls: AtomicUsize::new(0),
                })
            })
            .clone()
    }

    fn start(&self, root: &Path) -> Result<FakeHandle, HarnessError> {
        let module = Module::open(root, self.world())?;
        self.modules.lock().unwrap().push(module.clone());
        Ok(FakeHandle(module))
    }

    fn mechanism(&self) -> KillMechanism {
        if self.claim_process_kill {
            KillMechanism::FaultHookThenProcessKill
        } else {
            KillMechanism::FaultHook
        }
    }

    /// How many `session.read` and `session.head` requests the runner has
    /// received, served or not.
    pub fn transcript_calls(&self) -> usize {
        self.world().transcript_calls.load(Ordering::SeqCst)
    }

    /// How many `run.result` requests the runner has received, served or
    /// not.
    pub fn run_result_calls(&self) -> usize {
        self.world().run_result_calls.load(Ordering::SeqCst)
    }

    pub fn held_steer_break_answers(&self) -> usize {
        self.world().held_steer_break_answers.load(Ordering::SeqCst)
    }

    pub fn held_steer_receipts(&self) -> Vec<Option<Delivered>> {
        self.world().held_steer_receipts.lock().unwrap().clone()
    }

    pub fn enable_retention(&mut self) {
        self.capabilities.insert(Capability::Retention);
        self.capabilities.insert(Capability::RetentionClock);
        self.advertise_status = true;
    }
}

#[async_trait]
impl Harness for FakeSubject {
    type Handle = FakeHandle;
    type Route = FakeRoute;

    fn declared_points(&self) -> Vec<PointDeclaration> {
        self.kill_points
            .iter()
            .map(|point| PointDeclaration {
                point: KillPoint::new(*point),
                mechanisms: vec![self.mechanism()],
                note: "in-process fake: stops after the record syncs and discards its memory"
                    .into(),
            })
            .collect()
    }

    async fn spawn(&self, state_root: &Path) -> Result<FakeHandle, HarnessError> {
        self.start(state_root)
    }

    async fn route(&self, _: &FakeHandle, _: &RouteStamp) -> Result<FakeRoute, HarnessError> {
        Err(HarnessError::new(
            "every route of this role is session-bound",
        ))
    }

    async fn kill_at(
        &self,
        handle: FakeHandle,
        point: &KillPoint,
        trigger: Trigger<'_>,
    ) -> Result<KillReport, HarnessError> {
        *handle.0.kill_hook.lock().unwrap() = Some(point.as_str().to_owned());
        trigger.await;
        let reached = handle.0.hook_fired.load(Ordering::SeqCst);
        drop(handle);
        if !reached {
            return Err(HarnessError::new(format!(
                "the trigger finished without reaching {point}"
            )));
        }
        Ok(KillReport {
            point: point.clone(),
            mechanism: self.mechanism(),
        })
    }

    async fn restart(&self, state_root: &Path) -> Result<FakeHandle, HarnessError> {
        self.start(state_root)
    }
}

#[async_trait]
impl LlmRunnerSubject for FakeSubject {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }

    fn owner_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "reserved:owner".into(),
            scope: None,
        }
    }

    fn stranger_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "reserved:stranger".into(),
            scope: None,
        }
    }

    async fn session_route(
        &self,
        handle: &FakeHandle,
        session: &str,
        stamp: &RouteStamp,
    ) -> Result<FakeRoute, HarnessError> {
        Ok(FakeRoute {
            module: handle.0.clone(),
            session: session.to_owned(),
            stamp: stamp.clone(),
        })
    }

    fn send_fields(&self, session: &str, first: bool) -> Map<String, Value> {
        let mut fields = Map::new();
        if first {
            fields.insert("plan".into(), json!({ "tools": [SCRIPTED_TOOL] }));
            fields.insert("model".into(), json!("fake-model"));
            if self.has_compaction_script(session) {
                fields.get_mut("plan").unwrap()["compaction_item"] =
                    json!({"provider":"fake.compaction"});
            }
        }
        fields
    }

    fn tool_provider_module(&self) -> String {
        TOOL_MODULE.into()
    }

    async fn install_script(&self, session: &str, script: Script) -> Result<(), HarnessError> {
        self.world()
            .scripts
            .lock()
            .unwrap()
            .insert(session.to_owned(), script);
        Ok(())
    }

    async fn tool_invocations(&self, arguments: &Value) -> usize {
        self.world()
            .invocations
            .lock()
            .unwrap()
            .get(&arguments.to_string())
            .copied()
            .unwrap_or(0)
    }

    async fn install_compaction_script(
        &self,
        session: &str,
        script: cortexkit_role_llm_runner_conformance::CompactionScript,
    ) -> Result<(), HarnessError> {
        self.install_compaction(session, script);
        Ok(())
    }
    async fn compaction_observation(
        &self,
        session: &str,
    ) -> Result<cortexkit_role_llm_runner_conformance::CompactionObservation, HarnessError> {
        Ok(self.observe_compaction(session))
    }
    async fn advance_compaction_clock(
        &self,
        session: &str,
        milliseconds: u64,
    ) -> Result<(), HarnessError> {
        self.advance_compaction(session, milliseconds)
    }
    async fn advance_retention_clock(
        &self,
        session: &str,
        milliseconds: u64,
    ) -> Result<(), HarnessError> {
        self.world()
            .retention_clock_advanced_ms
            .fetch_add(milliseconds, Ordering::SeqCst);
        for module in self.modules.lock().unwrap().clone() {
            if module.alive.load(Ordering::SeqCst) {
                module.sweep(session).map_err(|Killed| {
                    HarnessError::new("module was killed while advancing retention clock")
                })?;
            }
        }
        Ok(())
    }
    async fn answer_compaction(
        &self,
        session: &str,
        call_index: usize,
        request_id: Option<String>,
        answer: cortexkit_role_llm_runner_conformance::CompactionAnswer,
    ) -> Result<(), HarnessError> {
        self.deliver_compaction(session, call_index, request_id, answer)
    }
    async fn release_compaction_model(&self, session: &str) -> Result<(), HarnessError> {
        self.release_model(session)
    }
    async fn durable_compaction_setup(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<Option<cortexkit_role_llm_runner_conformance::CompactionMessage>, HarnessError>
    {
        self.durable_setup(handle, session)
    }

    async fn await_tool_call(&self, arguments: &Value) -> Result<(), HarnessError> {
        for _ in 0..100 {
            if self
                .world()
                .held
                .lock()
                .unwrap()
                .contains(&arguments.to_string())
            {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        Err(HarnessError::new("the call was never held"))
    }

    async fn release_tool_call(&self, arguments: &Value) -> Result<(), HarnessError> {
        let world = self.world();
        let key = arguments.to_string();
        if !world.held.lock().unwrap().remove(&key) {
            return Err(HarnessError::new("the call is not held"));
        }
        world.released.lock().unwrap().insert(key);
        let modules = self.modules.lock().unwrap().clone();
        for module in modules {
            if module.alive.load(Ordering::SeqCst) {
                module
                    .complete_held(arguments)
                    .map_err(|Killed| HarnessError::new("the module was killed"))?;
            }
        }
        Ok(())
    }

    async fn retention_deletion_finished(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<bool, HarnessError> {
        handle
            .0
            .sweep(session)
            .map_err(|Killed| HarnessError::new("killed during deletion inspection"))?;
        let state = handle.0.sessions.lock().unwrap().get(session).cloned();
        let disk = std::fs::read_to_string(&handle.0.log).unwrap_or_default();
        let content_records = disk
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|r| {
                r["session"] == json!(session)
                    && !matches!(r["kind"].as_str(), Some("tombstone" | "deleted"))
            });
        Ok(
            state.is_some_and(|s| s.deleted && s.messages.is_empty() && s.calls.is_empty())
                && !content_records,
        )
    }

    fn retention_probe_params(
        &self,
        _session: &str,
        method: &str,
    ) -> Result<Option<Vec<Value>>, HarnessError> {
        match method {
            "session.list" | "session.export" => Ok(Some(vec![json!({})])),
            _ => Err(HarnessError::new(format!("unclassified op: {method}"))),
        }
    }

    async fn pause(&self) {
        match self.pause_for {
            None => tokio::task::yield_now().await,
            Some(pause) => tokio::time::sleep(pause).await,
        }
    }
}
