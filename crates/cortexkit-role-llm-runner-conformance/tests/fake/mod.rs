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
//! > run sealed `interrupted`; anything else carries on.
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
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
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
        describe::{Major, RoleDescribe},
        errors, ops, points,
        read::{
            HeadMeta, LastRunState, ReadMessage, ReadMode, ReadPage, ReadRequest, ReadView,
            RunAttribution, ToolCallAttribution,
        },
        run::{FinalMessage, RunResult, RunResultRequest},
        send::{delivery_unsupported_detail, SendReply, SendRequest, DELIVERY_FIELD},
        subscribe::{SubscribeFrom, SubscribeRequest},
        PROVIDES,
    },
    Capability, LlmRunnerSubject, Reply, RouteFailure, RunnerRoute, Script, ScriptedPart,
    SubscribeOutcome, SCRIPTED_TOOL,
};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

pub const TOOL_MODULE: &str = "fake.tools";
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
    /// A read with no cursor answers the oldest page instead of the newest.
    pub tail_returns_oldest: bool,
    /// A call's key is minted from the model's tool_call_id, so a model id
    /// used in two steps gets the same key twice.
    pub call_key_from_model_id: bool,
    /// Resume seals a run cut by a kill `cancelled` instead of
    /// `interrupted`.
    pub cancel_cut_runs: bool,
    /// role.describe declares `interrupt`, which the runner does not serve
    /// and the subject does not declare.
    pub claim_unserved_group: bool,
}

/// What the subject and every module incarnation share: the scripted model
/// and the scripted tool provider live outside the runner, so they survive
/// its kills.
struct World {
    defects: Defects,
    groups: Vec<String>,
    scripts: Mutex<BTreeMap<String, Script>>,
    invocations: Mutex<BTreeMap<String, usize>>,
    held: Mutex<BTreeSet<String>>,
    released: Mutex<BTreeSet<String>>,
}

impl World {
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
    admitted: bool,
}

#[derive(Clone, Debug)]
struct RunRec {
    run_id: String,
    state: String,
    final_ordinal: Option<u64>,
    final_text: Option<String>,
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
            let open: Vec<String> = self.sessions.lock().unwrap()[&session]
                .calls
                .iter()
                .filter(|(_, call)| call.run_id == run_id && call.intent && !call.result)
                .map(|(key, _)| key.clone())
                .collect();
            if open.is_empty() {
                self.drive(&session, &run_id)?;
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
        match kind.as_str() {
            "send" => {
                let run_id = text("run_id");
                sess.push(
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
                        admitted: record["admitted"] == json!(true),
                    },
                );
                sess.runs.push(RunRec {
                    run_id,
                    state: "active".into(),
                    final_ordinal: None,
                    final_text: None,
                });
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
            let turn_index = sess
                .messages
                .iter()
                .filter(|m| m.body["role"] == "assistant")
                .count();
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
        let reply = match method {
            ops::ROLE_DESCRIBE => Ok(self.describe()),
            ops::SESSION_BASELINE => Ok(self.baseline(stamp, session, &params)),
            ops::SESSION_SEND => self.send(stamp, session, params),
            ops::SESSION_READ => Ok(self.read(session, &params)),
            ops::SESSION_HEAD => Ok(self.head(session, &params)),
            ops::RUN_RESULT => Ok(self.run_result(session, params)),
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
        for group in &capabilities {
            for op in groups::group_ops(group).unwrap_or_default() {
                if !served.iter().any(|s| s == op) {
                    served.push((*op).to_owned());
                }
            }
        }
        respond(
            RoleDescribe::new(vec![Major::new(PROVIDES, served, "alpha")], "fake-runner-1")
                .with_capabilities(capabilities)
                .with_session_capabilities_from(groups::source::ADMISSION)
                .with_max_bytes(DEFAULT_MAX_BYTES, MAXIMUM_MAX_BYTES),
        )
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
        let mut reply = if run.state == "active" {
            SendReply::new("active").with_run_id(&run.run_id)
        } else {
            SendReply::new("finished")
                .with_run_id(&run.run_id)
                .with_reason(&run.state)
        };
        if send.admitted {
            reply = reply.with_baseline(baseline());
        }
        respond(reply)
    }

    fn send(&self, stamp: &RouteStamp, session: &str, params: Value) -> Result<Reply, Killed> {
        if let Some(delivery) = params.get(DELIVERY_FIELD) {
            if !matches!(delivery.as_str(), Some("queue" | "steer" | "interrupt")) {
                return Ok(invalid(DELIVERY_FIELD));
            }
        }
        let request: SendRequest = match serde_json::from_value(params) {
            Ok(request) => request,
            Err(_) => return Ok(invalid("prompt")),
        };
        let delivery = request.delivery().as_str().to_owned();
        let existing = self.sessions.lock().unwrap().get(session).cloned();
        if let Some(sess) = &existing {
            if sess.owner != stamp.principal {
                return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
            }
            if let Some(send) = sess.sends.get(&request.send_id) {
                if send.prompt != request.prompt {
                    return Ok(refuse(
                        errors::SEND_ID_REUSE,
                        Some(json!({ "field": "prompt" })),
                    ));
                }
                if send.delivery != delivery {
                    return Ok(refuse(
                        errors::SEND_ID_REUSE,
                        Some(json!({ "field": DELIVERY_FIELD })),
                    ));
                }
                return Ok(self.send_reply(session, send));
            }
            if sess.runs.iter().any(|run| run.state == "active") {
                return Ok(refuse(errors::TRANSIENT, None));
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
            self.commit(
                session,
                json!({ "kind": "start", "owner": stamp.principal, "lineage": format!("lin-{session}") }),
            )?;
        }
        let run_id = format!(
            "run-{}",
            self.sessions.lock().unwrap()[session].runs.len() + 1
        );
        self.commit(
            session,
            json!({ "kind": "send", "send_id": request.send_id, "prompt": request.prompt,
                    "delivery": delivery, "run_id": run_id, "admitted": admitted }),
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
        if request.view == Some(ReadView::Model) {
            return invalid("view");
        }
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            if request.lineage_id.is_some() {
                return refuse(errors::LINEAGE_CHANGED, None);
            }
            return respond(ReadPage::no_lineage());
        };
        if request
            .lineage_id
            .as_ref()
            .is_some_and(|lineage| lineage != &sess.lineage)
        {
            return refuse(errors::LINEAGE_CHANGED, None);
        }
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAXIMUM_LIMIT) as usize;
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
            .map(|key| {
                let call = &sess.calls[key];
                let mut entry = ToolCallAttribution::new(&call.tool_call_id)
                    .with_call_key(key)
                    .with_indeterminate(call.intent && !call.result);
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
        let mut head = HeadMeta::new(&sess.lineage, 1_790_000_000_000 + sess.events.len() as u64)
            .with_head(cursor(sess.events.len()));
        if let Some(last) = sess.messages.last() {
            head = head.with_last_ordinal(last.ordinal);
        }
        if let Some(run) = sess.runs.last() {
            head = head.with_last_run_state(LastRunState {
                run_id: run.run_id.clone(),
                state: run.state.clone(),
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
        let mut result = RunResult::new(&run.run_id, &run.state);
        if run.state == "completed" {
            let ordinal = run.final_ordinal.unwrap_or_default();
            let mid = sessions[session].messages[ordinal as usize].mid.clone();
            result = result.with_final_message(FinalMessage {
                ordinal,
                mid,
                text: run.final_text.clone().unwrap_or_default(),
            });
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
    pub defects: Defects,
    pub capabilities: BTreeSet<Capability>,
    pub kill_points: Vec<&'static str>,
    /// Report every kill as a real process kill, which this in-process fake
    /// cannot make. Only for tests of the verdict.
    pub claim_process_kill: bool,
    world: Mutex<Option<Arc<World>>>,
    /// Every module started, so a released call reaches the one holding it.
    modules: Mutex<Vec<Arc<Module>>>,
}

/// The capabilities the conforming fake declares: every group it serves,
/// and held tool calls. It leaves `interrupt`, `model_view`, `compaction`
/// and `session_change` undeclared.
pub fn served() -> BTreeSet<Capability> {
    [
        Capability::TranscriptReads,
        Capability::RunOps,
        Capability::DispatchAttribution,
        Capability::Streaming,
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
];

impl FakeSubject {
    pub fn new(defects: Defects) -> Self {
        Self {
            defects,
            capabilities: served(),
            kill_points: KILL_POINTS.to_vec(),
            claim_process_kill: false,
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
                    defects: self.defects,
                    groups: self
                        .capabilities
                        .iter()
                        .filter_map(|c| c.group())
                        .map(str::to_owned)
                        .collect(),
                    scripts: Mutex::new(BTreeMap::new()),
                    invocations: Mutex::new(BTreeMap::new()),
                    held: Mutex::new(BTreeSet::new()),
                    released: Mutex::new(BTreeSet::new()),
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

    fn send_fields(&self, _: &str, first: bool) -> Map<String, Value> {
        let mut fields = Map::new();
        if first {
            fields.insert("plan".into(), json!({ "tools": [SCRIPTED_TOOL] }));
            fields.insert("model".into(), json!("fake-model"));
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

    async fn pause(&self) {
        tokio::task::yield_now().await;
    }
}
