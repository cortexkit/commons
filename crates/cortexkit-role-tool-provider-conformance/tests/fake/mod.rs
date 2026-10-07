//! A reference in-process tool provider, used ONLY to test the runner.
//!
//! Conformance runs against real modules. This fake exists so the runner's
//! own tests can show that each case passes a provider that keeps the
//! contract and fails one that breaks it, so it must be no more capable than
//! a real module. The property it models:
//!
//! > A single tool provider that serves `role.describe`, `tool.catalog` and
//! > four tools, holds approval-gated calls in a durable log keyed
//! > `(carrier, call_key)`, answers `tool.withdraw` from that log, keeps a
//! > late-result log per custodian, and after a kill knows only what its log
//! > holds.
//!
//! Everything it knows reaches the runner only as frames on a route; it has
//! no side channel into the runner. Its kills are fault hooks inside one
//! process, so it reports [`KillMechanism::FaultHook`], never a real process
//! kill, and a run against it can never pass the real-kill rule.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use async_trait::async_trait;
use cortexkit_role_tool_provider_conformance::{
    harness::{
        Harness, HarnessError, KillMechanism, KillPoint, KillReport, PointDeclaration, RouteStamp,
        Trigger,
    },
    wire::{
        call::{check_call, ToolCallRequest, CALL_KEY_FIELD},
        catalog::{composition_digest, schema_digest, system_text_digest, CatalogRequest},
        errors,
        late_results::{
            check_since, kinds, reasons, AckRequest, Cursor, LateEntry, LateResultsReply,
            LateResultsRequest,
        },
        ops, points,
        scope::ScopeIdentity,
        withdraw::{
            check_record_scope, parse_withdraw_request, resolve_carrier, CallerProblem,
            CarrierResolution, CompletedResult, StartedOutcome, WithdrawAnswer,
        },
    },
    CallSpec, Capability, Exchange, ObservedFrame, RouteFailure, ScopedPrincipals,
    ToolProviderSubject, ToolRoute,
};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

pub const QUICK: &str = "echo";
pub const SLOW: &str = "sleep";
pub const HELD: &str = "effect";
pub const DISABLED: &str = "danger";
const GENERATION: &str = "fake-gen-1";
/// The only preset the fake defines: its default variant.
const DEFAULT_PRESET: &str = "default";
/// Quotes, non-ASCII text, newlines and trailing spaces must all be hashed
/// exactly as returned, without JSON escaping or whitespace normalization.
pub const SYSTEM_TEXT: &str = "Use \"echo\" for café.\nKeep trailing whitespace. \n";

/// A provider-defined digest of the inputs, deliberately not the text digest.
pub fn system_text_preflight_digest() -> String {
    composition_digest(&json!({ "preset": DEFAULT_PRESET, "template": SYSTEM_TEXT })).unwrap()
}

#[derive(Clone, Copy, Debug)]
pub enum SystemTextDefect {
    MissingAnswer,
    MissingText,
    WrappedDigests,
    WrappedItemDigest,
    WrappedPreflightDigest,
    UppercaseItemDigest,
    UppercasePreflightDigest,
    MissingItemDigest,
    MissingPreflightDigest,
    MissingToolNames,
    UnsortedToolNames,
    DuplicateToolNames,
    ChangingItemDigest,
    ChangingPreflightDigest,
}

/// Deliberate contract breaks: each field makes the fake violate one rule a
/// runner case must catch.
#[derive(Clone, Copy, Debug, Default)]
pub struct Defects {
    /// A cancelled call gets a second terminal frame after its error.
    pub double_terminal_on_cancel: bool,
    /// A key never held is refused instead of answered `unknown_call`.
    pub refuse_unknown_call: bool,
    /// Held calls that were not started are run when the provider restarts.
    pub run_held_calls_on_restart: bool,
    /// The held tool's argument schema is a root-level union.
    pub root_union_schema: bool,
    /// Acks are accepted but forgotten, so acked entries are served again.
    pub forget_acks: bool,
    /// `tool.catalog` answers with its default variant for any preset,
    /// including one it does not define.
    pub accept_any_preset: bool,
    /// The held tool carries an unprefixed capability tag the role document
    /// does not define.
    pub undefined_unprefixed_tag: bool,
    /// Return malformed system text without changing the other catalog cases.
    pub system_text: Option<SystemTextDefect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Prepared,
    Authorized,
    Started,
    Settled,
    Withdrawn,
    NotStarted,
}

#[derive(Clone, Debug)]
struct Record {
    scope: ScopeIdentity,
    marker: PathBuf,
    state: State,
}

struct Killed;

type Key = (String, String);

/// One running instance: its log on disk and what it rebuilt from it.
pub struct Module {
    log: PathBuf,
    defects: Defects,
    incarnation: String,
    alive: AtomicBool,
    kill_hook: Mutex<Option<String>>,
    hook_fired: AtomicBool,
    records: Mutex<BTreeMap<Key, Record>>,
    /// The late-result log of this incarnation: `(seq, entry)`, seq from 1.
    late: Mutex<Vec<(u64, LateEntry)>>,
    acked: Mutex<BTreeSet<String>>,
    system_text_fetches: AtomicU64,
}

impl Module {
    fn open(root: &Path, defects: Defects) -> Result<Arc<Self>, HarnessError> {
        std::fs::create_dir_all(root).map_err(|e| HarnessError::new(e.to_string()))?;
        let starts = root.join("starts");
        let start = std::fs::read_to_string(&starts)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        std::fs::write(&starts, start.to_string()).map_err(|e| HarnessError::new(e.to_string()))?;
        let module = Arc::new(Self {
            log: root.join("records.jsonl"),
            defects,
            incarnation: format!("fake-inc-{start}"),
            alive: AtomicBool::new(true),
            kill_hook: Mutex::new(None),
            hook_fired: AtomicBool::new(false),
            records: Mutex::new(BTreeMap::new()),
            late: Mutex::new(Vec::new()),
            acked: Mutex::new(BTreeSet::new()),
            system_text_fetches: AtomicU64::new(0),
        });
        module.replay()?;
        let pending: Vec<Key> = module
            .records
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| matches!(r.state, State::Prepared | State::Authorized))
            .map(|(key, _)| key.clone())
            .collect();
        for key in pending {
            if defects.run_held_calls_on_restart {
                let _ = module.run(&key);
            } else if module
                .append(json!({ "point": "NotStarted", "carrier": key.0, "call_key": key.1 }))
                .is_ok()
            {
                module.set_state(&key, State::NotStarted);
                module.publish(&key, kinds::NOT_STARTED);
            }
        }
        Ok(module)
    }

    fn replay(&self) -> Result<(), HarnessError> {
        let Ok(text) = std::fs::read_to_string(&self.log) else {
            return Ok(());
        };
        for line in text.lines() {
            let entry: Value = serde_json::from_str(line)
                .map_err(|e| HarnessError::new(format!("corrupt log: {e}")))?;
            let point = entry["point"].as_str().unwrap_or_default();
            if point == "Acked" {
                self.acked
                    .lock()
                    .unwrap()
                    .insert(entry["event_id"].as_str().unwrap().to_owned());
                continue;
            }
            let key = (
                entry["carrier"].as_str().unwrap().to_owned(),
                entry["call_key"].as_str().unwrap().to_owned(),
            );
            let state = match point {
                points::PREPARED => {
                    self.records.lock().unwrap().insert(
                        key,
                        Record {
                            scope: serde_json::from_value(entry["scope"].clone()).unwrap(),
                            marker: PathBuf::from(entry["marker"].as_str().unwrap()),
                            state: State::Prepared,
                        },
                    );
                    continue;
                }
                points::AUTHORIZED => State::Authorized,
                points::DISPATCH_STARTED => State::Started,
                "Settled" => State::Settled,
                "Withdrawn" => State::Withdrawn,
                "NotStarted" => State::NotStarted,
                other => return Err(HarnessError::new(format!("unknown record {other}"))),
            };
            self.set_state(&key, state);
            match state {
                State::Settled => self.publish(&key, kinds::RESULT),
                State::NotStarted => self.publish(&key, kinds::NOT_STARTED),
                _ => {}
            }
        }
        Ok(())
    }

    /// Append one durable record. A kill armed for this record's point takes
    /// effect right after the record is on disk: nothing later is written or
    /// done.
    fn append(&self, entry: Value) -> Result<(), Killed> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(Killed);
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .expect("fake log opens");
        writeln!(file, "{entry}").expect("fake log writes");
        file.sync_all().expect("fake log syncs");
        if self.kill_hook.lock().unwrap().as_deref() == entry["point"].as_str() {
            self.alive.store(false, Ordering::SeqCst);
            self.hook_fired.store(true, Ordering::SeqCst);
            return Err(Killed);
        }
        Ok(())
    }

    fn mark(&self, point: &str, key: &Key) -> Result<(), Killed> {
        self.append(json!({ "point": point, "carrier": key.0, "call_key": key.1 }))
    }

    fn set_state(&self, key: &Key, state: State) {
        if let Some(record) = self.records.lock().unwrap().get_mut(key) {
            record.state = state;
        }
    }

    /// Add a late-result entry for `key`, unless it was acked before.
    fn publish(&self, key: &Key, kind: &str) {
        let event_id = format!("{}|{}|{kind}", key.0, key.1);
        if self.acked.lock().unwrap().contains(&event_id) {
            return;
        }
        let scope = self.records.lock().unwrap()[key].scope.clone();
        let mut entry = json!({
            "kind": kind, "owner": scope.owner, "ref": scope.scope_ref,
            "scope_epoch": scope.scope_epoch, "custodian": scope.owner,
            "call_key": key.1, "event_id": event_id, "settled_at": 1_790_000_000_000u64,
            "reduced": false,
        });
        if kind == kinds::NOT_STARTED {
            entry["reason"] = json!(reasons::RESTART_BEFORE_DISPATCH);
        } else {
            entry["result"] = json!({ "content": "ran" });
        }
        let mut late = self.late.lock().unwrap();
        let seq = late.len() as u64 + 1;
        late.push((seq, serde_json::from_value(entry).unwrap()));
    }

    /// Dispatch a held call: DispatchStarted, the action, Settled.
    fn run(&self, key: &Key) -> Result<(), Killed> {
        let marker = self.records.lock().unwrap()[key].marker.clone();
        self.mark(points::DISPATCH_STARTED, key)?;
        self.set_state(key, State::Started);
        std::fs::write(&marker, b"ran").expect("marker writes");
        self.mark("Settled", key)?;
        self.set_state(key, State::Settled);
        self.publish(key, kinds::RESULT);
        Ok(())
    }

    fn find(&self, call_key: &str) -> Option<Key> {
        self.records
            .lock()
            .unwrap()
            .keys()
            .find(|(_, k)| k == call_key)
            .cloned()
    }

    fn approve(&self, call_key: &str) -> Result<(), HarnessError> {
        let key = self
            .find(call_key)
            .ok_or_else(|| HarnessError::new(format!("no question filed for {call_key}")))?;
        if self.mark(points::AUTHORIZED, &key).is_err() {
            return Ok(());
        }
        self.set_state(&key, State::Authorized);
        let _ = self.run(&key);
        Ok(())
    }

    fn describe() -> Value {
        json!({
            "majors": [{
                "version": "tool-provider/v1", "stability": "alpha",
                "ops": [ops::ROLE_DESCRIBE, ops::TOOL_CATALOG, ops::TOOL_WITHDRAW,
                        ops::LATE_RESULTS, ops::LATE_RESULTS_ACK],
            }],
            "implementation_version": "fake-0",
            "capabilities": []
        })
    }

    /// The served tools, each with its schema digest computed from its schema.
    fn tools(&self) -> Vec<Value> {
        let object = json!({ "type": "object", "description": "Echo text back.",
                             "properties": { "text": { "type": "string", "description": "The text." } } });
        let held_schema = if self.defects.root_union_schema {
            json!({ "anyOf": [
                { "type": "object", "properties": { "marker": { "type": "string" } } },
                { "type": "object", "properties": { "path": { "type": "string" } } }
            ]})
        } else {
            json!({ "type": "object", "properties": { "marker": { "type": "string" } }, "required": ["marker"] })
        };
        let held_capabilities = if self.defects.undefined_unprefixed_tag {
            json!(["fake:effect/v1", "code.refactor/v1"])
        } else {
            json!(["fake:effect/v1", "code.edit/v1"])
        };
        let mut tools = vec![
            json!({ "name": QUICK, "semantics": 1, "input_schema": object }),
            json!({ "name": SLOW, "semantics": 1, "input_schema": { "type": "object" } }),
            json!({ "name": HELD, "semantics": 2, "result_ops": ["prepend", "append"],
                    "capabilities": held_capabilities, "input_schema": held_schema }),
        ];
        for tool in &mut tools {
            tool["schema_digest"] = json!(schema_digest(&tool["input_schema"]).unwrap());
        }
        tools
    }

    fn catalog(&self, arguments: &Value) -> Result<Value, ErrorBody> {
        let request: CatalogRequest = serde_json::from_value(arguments.clone())
            .map_err(|e| errors::invalid_request("arguments", e.to_string()))?;
        // An absent preset means the default variant; any other name is
        // refused rather than mapped to the closest variant.
        if let Some(preset) = request.preset.as_deref() {
            if preset != DEFAULT_PRESET && !self.defects.accept_any_preset {
                return Err(errors::invalid_request(
                    "preset",
                    format!("no preset named {preset}"),
                ));
            }
        }
        if let Some(item) = &request.system_text {
            if item.preset != DEFAULT_PRESET {
                return Err(errors::invalid_request(
                    "system_text.preset",
                    format!("no system-text preset named {}", item.preset),
                ));
            }
        }
        let catalog_digest = if request.system_text.is_some() {
            "fake-catalog-with-system-text-1"
        } else {
            "fake-catalog-1"
        };
        if request.digest_only == Some(true) {
            return Ok(json!({ "generation": GENERATION, "catalog_digest": catalog_digest }));
        }
        let mut answer = json!({
            "generation": GENERATION,
            "catalog_digest": catalog_digest,
            "capabilities": { "late_results": true },
            "tools": self.tools(),
        });
        if request.system_text.is_some()
            && !matches!(
                self.defects.system_text,
                Some(SystemTextDefect::MissingAnswer)
            )
        {
            answer["system_text"] = self.system_text();
        }
        Ok(answer)
    }

    fn system_text(&self) -> Value {
        let digest = system_text_digest(SYSTEM_TEXT);
        let preflight = system_text_preflight_digest();
        let fetch = self.system_text_fetches.fetch_add(1, Ordering::SeqCst);
        let mut item = json!({
            "text": SYSTEM_TEXT,
            "item_digest": digest,
            "preflight_digest": preflight,
            "tool_names": [QUICK, HELD, SLOW],
        });
        let wrapped = || composition_digest(&json!({ "text": SYSTEM_TEXT })).unwrap();
        match self.defects.system_text {
            Some(SystemTextDefect::MissingText) => {
                item.as_object_mut().unwrap().remove("text");
            }
            Some(SystemTextDefect::WrappedDigests) => {
                item["item_digest"] = json!(wrapped());
                item["preflight_digest"] = json!(wrapped());
            }
            Some(SystemTextDefect::WrappedItemDigest) => item["item_digest"] = json!(wrapped()),
            Some(SystemTextDefect::WrappedPreflightDigest) => {
                item["preflight_digest"] = json!(wrapped());
            }
            Some(SystemTextDefect::UppercaseItemDigest) => {
                item["item_digest"] = json!(digest.to_uppercase());
            }
            Some(SystemTextDefect::UppercasePreflightDigest) => {
                item["preflight_digest"] = json!(preflight.to_uppercase());
            }
            Some(SystemTextDefect::MissingItemDigest) => {
                item.as_object_mut().unwrap().remove("item_digest");
            }
            Some(SystemTextDefect::MissingPreflightDigest) => {
                item.as_object_mut().unwrap().remove("preflight_digest");
            }
            Some(SystemTextDefect::MissingToolNames) => {
                item.as_object_mut().unwrap().remove("tool_names");
            }
            Some(SystemTextDefect::UnsortedToolNames) => {
                item["tool_names"] = json!([SLOW, QUICK, HELD]);
            }
            Some(SystemTextDefect::DuplicateToolNames) => {
                item["tool_names"] = json!([QUICK, QUICK, HELD, SLOW]);
            }
            Some(SystemTextDefect::ChangingItemDigest) => {
                if fetch > 0 {
                    item["item_digest"] =
                        json!(system_text_digest(&format!("{SYSTEM_TEXT}{fetch}")));
                }
            }
            Some(SystemTextDefect::ChangingPreflightDigest) => {
                item["preflight_digest"] = json!(composition_digest(&json!({
                    "preset": DEFAULT_PRESET, "template": SYSTEM_TEXT, "fetch": fetch,
                }))
                .unwrap());
            }
            None | Some(SystemTextDefect::MissingAnswer) => {}
        }
        item
    }

    fn handle(&self, stamp: &RouteStamp, body: Value) -> Result<Vec<ObservedFrame>, RouteFailure> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(RouteFailure::new("module is gone"));
        }
        let request: ToolCallRequest = match serde_json::from_value(body) {
            Ok(request) => request,
            Err(e) => return Ok(error(errors::invalid_request("body", e.to_string()))),
        };
        let name = request.name.as_str();
        let reply = |result: Result<Value, ErrorBody>| match result {
            Ok(body) => vec![ObservedFrame::Response(body)],
            Err(refusal) => error(refusal),
        };
        match name {
            ops::TOOL_WITHDRAW => {
                return Ok(reply(
                    self.withdraw(stamp, &request).map(|answer| answer.encode()),
                ))
            }
            ops::LATE_RESULTS => return Ok(reply(self.late_results(stamp, &request.arguments))),
            ops::LATE_RESULTS_ACK => return Ok(reply(self.ack(stamp, &request.arguments))),
            _ => {}
        }
        let pin = match check_call(&request) {
            Ok(pin) => pin,
            Err(refusal) => return Ok(error(refusal)),
        };
        let current = self.tools().into_iter().find(|tool| tool["name"] == name);
        if let (Some(pin), Some(current)) = (&pin, current) {
            if pin.schema_digest != current["schema_digest"] {
                return Ok(error(ErrorBody::new(
                    errors::TOOL_SCHEMA_CHANGED,
                    "the schema moved",
                )));
            }
            if json!(pin.semantics) != current["semantics"] {
                return Ok(error(ErrorBody::new(
                    errors::TOOL_SEMANTICS_CHANGED,
                    "semantics moved",
                )));
            }
        }
        Ok(match name {
            ops::ROLE_DESCRIBE => vec![ObservedFrame::Response(Self::describe())],
            ops::TOOL_CATALOG => reply(self.catalog(&request.arguments)),
            QUICK | SLOW => vec![ObservedFrame::Response(
                json!({ "content": request.arguments }),
            )],
            DISABLED => error(errors::tool_disabled(DISABLED)),
            HELD => self.hold(stamp, &request),
            other => error(errors::unknown_tool(other)),
        })
    }

    fn hold(&self, stamp: &RouteStamp, request: &ToolCallRequest) -> Vec<ObservedFrame> {
        let (Some(call_key), Some(scope)) = (&request.call_key, scope_of(stamp)) else {
            return error(errors::invalid_request(
                CALL_KEY_FIELD,
                "held calls need a call_key and a scope",
            ));
        };
        let key = (stamp.principal.clone(), call_key.clone());
        if !self.records.lock().unwrap().contains_key(&key) {
            let marker = request.arguments["marker"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            self.records.lock().unwrap().insert(
                key.clone(),
                Record {
                    scope: scope.clone(),
                    marker: PathBuf::from(&marker),
                    state: State::Prepared,
                },
            );
            let entry = json!({ "point": points::PREPARED, "carrier": key.0, "call_key": key.1,
                                "scope": scope, "marker": marker });
            if self.append(entry).is_err() {
                return Vec::new();
            }
        }
        vec![ObservedFrame::Response(json!({ "pending": true }))]
    }

    fn withdraw(
        &self,
        stamp: &RouteStamp,
        request: &ToolCallRequest,
    ) -> Result<WithdrawAnswer, ErrorBody> {
        let arguments = parse_withdraw_request(request)?;
        let scope = scope_of(stamp);
        let resolution =
            resolve_carrier(&stamp.principal, scope.as_ref(), &arguments).map_err(|p| p.error())?;
        let (carrier, owner_as_carrier) = match resolution {
            CarrierResolution::Carrier(carrier) => (carrier, false),
            CarrierResolution::OwnerAsCarrier(owner) => (owner, true),
        };
        let key = (carrier, arguments.call_key);
        let Some(record) = self.records.lock().unwrap().get(&key).cloned() else {
            if owner_as_carrier {
                return Err(CallerProblem::CarrierRequired.error());
            }
            return Ok(if self.defects.refuse_unknown_call {
                WithdrawAnswer::Refused { reason: cortexkit_role_tool_provider_conformance::wire::withdraw::RefusalReason::Denied }
            } else {
                WithdrawAnswer::UnknownCall
            });
        };
        check_record_scope(Some(&record.scope), scope.as_ref()).map_err(|p| p.error())?;
        Ok(match record.state {
            State::Prepared | State::Authorized => {
                if self.mark("Withdrawn", &key).is_ok() {
                    self.set_state(&key, State::Withdrawn);
                }
                WithdrawAnswer::Withdrawn
            }
            State::Withdrawn | State::NotStarted => WithdrawAnswer::Withdrawn,
            State::Started => WithdrawAnswer::AlreadyStarted {
                outcome: StartedOutcome::Unknown,
            },
            State::Settled => WithdrawAnswer::Completed {
                result: CompletedResult::OutcomeOnly("ok".into()),
            },
        })
    }

    fn head(&self) -> u64 {
        self.late.lock().unwrap().len() as u64
    }

    fn late_results(&self, stamp: &RouteStamp, arguments: &Value) -> Result<Value, ErrorBody> {
        let request: LateResultsRequest = serde_json::from_value(arguments.clone())
            .map_err(|e| errors::invalid_request("arguments", e.to_string()))?;
        check_since(&self.incarnation, self.head(), request.since.as_ref())?;
        let after = request.since.as_ref().map_or(0, |c| c.seq);
        let limit = request.limit.map_or(usize::MAX, |l| l as usize);
        let acked = self.acked.lock().unwrap().clone();
        let mine: Vec<(u64, LateEntry)> = self
            .late
            .lock()
            .unwrap()
            .iter()
            .filter(|(seq, e)| {
                *seq > after && e.custodian == stamp.principal && !acked.contains(&e.event_id)
            })
            .cloned()
            .collect();
        let taken: Vec<(u64, LateEntry)> = mine.iter().take(limit).cloned().collect();
        let cursor = Cursor {
            provider_incarnation: self.incarnation.clone(),
            seq: taken.last().map_or(after, |(seq, _)| *seq),
        };
        let reply = LateResultsReply {
            entries: taken.into_iter().map(|(_, e)| e).collect(),
            cursor,
            more: mine.len() > limit,
        };
        Ok(serde_json::to_value(reply).unwrap())
    }

    fn ack(&self, stamp: &RouteStamp, arguments: &Value) -> Result<Value, ErrorBody> {
        let request: AckRequest = serde_json::from_value(arguments.clone())
            .map_err(|e| errors::invalid_request("arguments", e.to_string()))?;
        check_since(&self.incarnation, self.head(), Some(&request.through))?;
        if self.defects.forget_acks {
            return Ok(json!({}));
        }
        let events: Vec<String> = self
            .late
            .lock()
            .unwrap()
            .iter()
            .filter(|(seq, e)| *seq <= request.through.seq && e.custodian == stamp.principal)
            .map(|(_, e)| e.event_id.clone())
            .collect();
        for event in events {
            if self
                .append(json!({ "point": "Acked", "event_id": event }))
                .is_ok()
            {
                self.acked.lock().unwrap().insert(event);
            }
        }
        Ok(json!({}))
    }
}

fn scope_of(stamp: &RouteStamp) -> Option<ScopeIdentity> {
    stamp.scope.as_ref().map(|s| ScopeIdentity {
        owner: s.owner.clone(),
        scope_ref: s.scope_ref.clone(),
        scope_epoch: s.scope_epoch,
    })
}

fn error(body: ErrorBody) -> Vec<ObservedFrame> {
    vec![ObservedFrame::Error(body)]
}

pub struct FakeHandle(Arc<Module>);

impl Drop for FakeHandle {
    fn drop(&mut self) {
        self.0.alive.store(false, Ordering::SeqCst);
    }
}

pub struct FakeRoute {
    module: Arc<Module>,
    stamp: RouteStamp,
}

#[async_trait]
impl ToolRoute for FakeRoute {
    async fn request(&self, body: Value) -> Result<Exchange, RouteFailure> {
        Ok(Exchange {
            frames: self.module.handle(&self.stamp, body)?,
        })
    }

    async fn request_then_cancel(&self, body: Value) -> Result<Exchange, RouteFailure> {
        if body["name"] != SLOW {
            return self.request(body).await;
        }
        let mut frames = vec![
            ObservedFrame::Data(json!({ "progress": 0 })),
            ObservedFrame::Error(ErrorBody::new("cancelled", "cancelled by the caller")),
        ];
        if self.module.defects.double_terminal_on_cancel {
            frames.push(ObservedFrame::Response(
                json!({ "content": "finished anyway" }),
            ));
        }
        Ok(Exchange { frames })
    }
}

pub struct FakeSubject {
    pub defects: Defects,
    pub capabilities: BTreeSet<Capability>,
    pub system_text_arguments: Option<Value>,
    live: Mutex<Option<Arc<Module>>>,
}

impl FakeSubject {
    pub fn new(defects: Defects) -> Self {
        Self {
            defects,
            capabilities: [
                Capability::ScopeStamp,
                Capability::CallKey,
                Capability::SchemaPin,
                Capability::HeldCalls,
                Capability::DisableTool,
                Capability::Cancellation,
                Capability::ApprovalExecution,
                Capability::LateResults,
                Capability::SystemText,
            ]
            .into(),
            system_text_arguments: Some(json!({
                "params": {}, "preset": DEFAULT_PRESET,
                "system_text": { "preset": DEFAULT_PRESET, "params": {} },
            })),
            live: Mutex::new(None),
        }
    }

    fn live(&self) -> Result<Arc<Module>, HarnessError> {
        self.live
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| HarnessError::new("no live module"))
    }

    fn start(&self, root: &Path) -> Result<FakeHandle, HarnessError> {
        let module = Module::open(root, self.defects)?;
        *self.live.lock().unwrap() = Some(module.clone());
        Ok(FakeHandle(module))
    }
}

#[async_trait]
impl Harness for FakeSubject {
    type Handle = FakeHandle;
    type Route = FakeRoute;

    fn declared_points(&self) -> Vec<PointDeclaration> {
        points::ALL
            .iter()
            .map(|point| PointDeclaration {
                point: KillPoint::new(*point),
                mechanisms: vec![KillMechanism::FaultHook],
                note: "in-process fake: stops after the record syncs and discards its memory; there is no process to kill".into(),
            })
            .collect()
    }

    async fn spawn(&self, state_root: &Path) -> Result<FakeHandle, HarnessError> {
        self.start(state_root)
    }

    async fn route(
        &self,
        handle: &FakeHandle,
        stamp: &RouteStamp,
    ) -> Result<FakeRoute, HarnessError> {
        Ok(FakeRoute {
            module: handle.0.clone(),
            stamp: stamp.clone(),
        })
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
        *self.live.lock().unwrap() = None;
        if !reached {
            return Err(HarnessError::new(format!(
                "the trigger finished without reaching {point}"
            )));
        }
        Ok(KillReport {
            point: point.clone(),
            mechanism: KillMechanism::FaultHook,
        })
    }

    async fn restart(&self, state_root: &Path) -> Result<FakeHandle, HarnessError> {
        self.start(state_root)
    }
}

#[async_trait]
impl ToolProviderSubject for FakeSubject {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }

    fn plain_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "direct".into(),
            scope: None,
        }
    }

    fn scoped_principals(&self) -> Option<ScopedPrincipals> {
        let scope = ScopeIdentity {
            owner: "reserved:owner".into(),
            scope_ref: "scope-1".into(),
            scope_epoch: 1,
        };
        Some(ScopedPrincipals {
            other_scope: ScopeIdentity {
                scope_ref: "scope-2".into(),
                ..scope.clone()
            },
            scope,
            carrier: "reserved:broca".into(),
            other_carrier: "reserved:thalamus".into(),
        })
    }

    fn catalog_arguments(&self) -> Value {
        json!({ "params": Map::new(), "preset": DEFAULT_PRESET })
    }

    fn system_text_catalog_arguments(&self) -> Option<Value> {
        self.system_text_arguments.clone()
    }

    fn quick_call(&self) -> CallSpec {
        (QUICK.into(), json!({ "text": "hi" }))
    }

    fn slow_call(&self) -> Option<CallSpec> {
        Some((SLOW.into(), json!({})))
    }

    fn disabled_tool(&self) -> Option<String> {
        Some(DISABLED.into())
    }

    fn held_call(&self, marker: &Path) -> Option<CallSpec> {
        Some((HELD.into(), json!({ "marker": marker })))
    }

    async fn await_held(&self, call_key: &str) -> Result<(), HarnessError> {
        for _ in 0..100 {
            if self.live()?.find(call_key).is_some() {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        Err(HarnessError::new(format!("{call_key} was never held")))
    }

    async fn approve(&self, call_key: &str) -> Result<(), HarnessError> {
        self.await_held(call_key).await?;
        self.live()?.approve(call_key)
    }

    async fn settle(&self) {
        tokio::task::yield_now().await;
    }
}
