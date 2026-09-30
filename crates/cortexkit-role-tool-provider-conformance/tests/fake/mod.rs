//! A reference in-process tool provider, used ONLY to test the runner.
//!
//! Conformance runs against real modules. This fake exists so the runner's
//! own tests can show that each case passes a provider that keeps the
//! contract and fails one that breaks it, so it must be no more capable than
//! a real module. The property it models:
//!
//! > A single tool provider that serves `role.describe`, `tool.catalog` and
//! > four tools, holds approval-gated calls in a durable log keyed
//! > `(carrier, call_key)`, answers `tool.withdraw` from that log, and after a
//! > kill knows only what its log holds.
//!
//! Everything it knows reaches the runner only as frames on a route; it has
//! no side channel into the runner. Its kills are fault hooks inside one
//! process, so it reports [`KillMechanism::FaultHook`], never a real process
//! kill, and a run against it can never pass the real-kill rule.
//!
//! [`Defects`] switch on one contract break each, so a test can show the case
//! that should catch it does.

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
use cortexkit_role_tool_provider_conformance::{
    harness::{
        Harness, HarnessError, KillMechanism, KillPoint, KillReport, PointDeclaration, RouteStamp,
        Trigger,
    },
    wire::{
        call::{check_call_key, KeyedToolCallRequest, CALL_KEY_FIELD},
        errors, ops, points,
        scope::ScopeIdentity,
        withdraw::{
            parse_withdraw_request, resolve_carrier, CallerProblem, CompletedResult, RefusalReason,
            StartedOutcome, WithdrawAnswer,
        },
    },
    CallSpec, Capability, Exchange, ObservedFrame, RouteFailure, ScopedPrincipals,
    ToolProviderSubject, ToolRoute,
};
use serde_json::{json, Value};
use subc_protocol::ErrorBody;

pub const QUICK: &str = "echo";
pub const SLOW: &str = "sleep";
pub const HELD: &str = "effect";
pub const DISABLED: &str = "danger";

/// One contract break each.
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Prepared,
    Authorized,
    Started,
    Settled,
    Withdrawn,
}

#[derive(Clone, Debug)]
struct Record {
    scope: ScopeIdentity,
    marker: PathBuf,
    state: State,
}

struct Killed;

/// One running instance: its log on disk and what it rebuilt from it.
pub struct Module {
    log: PathBuf,
    defects: Defects,
    alive: AtomicBool,
    kill_hook: Mutex<Option<String>>,
    hook_fired: AtomicBool,
    records: Mutex<BTreeMap<(String, String), Record>>,
}

impl Module {
    fn open(root: &Path, defects: Defects) -> Result<Arc<Self>, HarnessError> {
        std::fs::create_dir_all(root).map_err(|e| HarnessError::new(e.to_string()))?;
        let log = root.join("records.jsonl");
        let mut records = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(&log) {
            for line in text.lines() {
                let entry: Value = serde_json::from_str(line)
                    .map_err(|e| HarnessError::new(format!("corrupt log: {e}")))?;
                let key = (
                    entry["carrier"].as_str().unwrap().to_owned(),
                    entry["call_key"].as_str().unwrap().to_owned(),
                );
                let state = match entry["point"].as_str().unwrap() {
                    points::PREPARED => {
                        records.insert(
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
                    other => return Err(HarnessError::new(format!("unknown record {other}"))),
                };
                if let Some(record) = records.get_mut(&key) {
                    record.state = state;
                }
            }
        }
        let module = Arc::new(Self {
            log,
            defects,
            alive: AtomicBool::new(true),
            kill_hook: Mutex::new(None),
            hook_fired: AtomicBool::new(false),
            records: Mutex::new(records),
        });
        if defects.run_held_calls_on_restart {
            let pending: Vec<(String, String)> = module
                .records
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, r)| matches!(r.state, State::Prepared | State::Authorized))
                .map(|(key, _)| key.clone())
                .collect();
            for key in pending {
                let _ = module.run(&key);
            }
        }
        Ok(module)
    }

    /// Append one durable record. A kill armed for this point takes effect
    /// right after the record is on disk: nothing later is written or done.
    fn append(
        &self,
        point: &str,
        carrier: &str,
        call_key: &str,
        extra: Value,
    ) -> Result<(), Killed> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(Killed);
        }
        let mut entry = json!({ "point": point, "carrier": carrier, "call_key": call_key });
        if let Value::Object(extra) = extra {
            entry.as_object_mut().unwrap().extend(extra);
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .expect("fake log opens");
        writeln!(file, "{entry}").expect("fake log writes");
        file.sync_all().expect("fake log syncs");
        if self.kill_hook.lock().unwrap().as_deref() == Some(point) {
            self.alive.store(false, Ordering::SeqCst);
            self.hook_fired.store(true, Ordering::SeqCst);
            return Err(Killed);
        }
        Ok(())
    }

    fn set_state(&self, key: &(String, String), state: State) {
        if let Some(record) = self.records.lock().unwrap().get_mut(key) {
            record.state = state;
        }
    }

    /// Dispatch a held call: DispatchStarted, the action, Settled.
    fn run(&self, key: &(String, String)) -> Result<(), Killed> {
        let marker = self.records.lock().unwrap()[key].marker.clone();
        self.append(points::DISPATCH_STARTED, &key.0, &key.1, json!({}))?;
        self.set_state(key, State::Started);
        std::fs::write(&marker, b"ran").expect("marker writes");
        self.append("Settled", &key.0, &key.1, json!({}))?;
        self.set_state(key, State::Settled);
        Ok(())
    }

    fn approve(&self, call_key: &str) -> Result<(), HarnessError> {
        let key = self
            .records
            .lock()
            .unwrap()
            .keys()
            .find(|(_, k)| k == call_key)
            .cloned()
            .ok_or_else(|| HarnessError::new(format!("no question filed for {call_key}")))?;
        if self
            .append(points::AUTHORIZED, &key.0, &key.1, json!({}))
            .is_err()
        {
            return Ok(());
        }
        self.set_state(&key, State::Authorized);
        let _ = self.run(&key);
        Ok(())
    }

    fn holds(&self, call_key: &str) -> bool {
        self.records
            .lock()
            .unwrap()
            .keys()
            .any(|(_, k)| k == call_key)
    }

    fn describe() -> Value {
        json!({
            "role": "tool-provider", "version": "v1", "stability": "alpha",
            "implementation_version": "fake-0",
            "ops": [ops::ROLE_DESCRIBE, ops::TOOL_CATALOG, ops::TOOL_WITHDRAW],
            "capabilities": []
        })
    }

    fn catalog(&self) -> Value {
        let object = json!({ "type": "object", "properties": { "text": { "type": "string" } } });
        let held_schema = if self.defects.root_union_schema {
            json!({ "anyOf": [
                { "type": "object", "properties": { "marker": { "type": "string" } } },
                { "type": "object", "properties": { "path": { "type": "string" } } }
            ]})
        } else {
            json!({ "type": "object", "properties": { "marker": { "type": "string" } }, "required": ["marker"] })
        };
        json!({
            "generation": 1,
            "tools": [
                { "name": QUICK, "schema_digest": "fake:echo:1", "capabilities": [], "input_schema": object },
                { "name": SLOW, "schema_digest": "fake:sleep:1", "capabilities": [], "input_schema": { "type": "object" } },
                { "name": HELD, "schema_digest": "fake:effect:1", "result_ops": ["prepend", "append"],
                  "capabilities": ["fake:effect/v1"], "input_schema": held_schema }
            ]
        })
    }

    fn handle(&self, stamp: &RouteStamp, body: Value) -> Result<Vec<ObservedFrame>, RouteFailure> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(RouteFailure::new("module is gone"));
        }
        let request: KeyedToolCallRequest = match serde_json::from_value(body) {
            Ok(request) => request,
            Err(e) => return Ok(error(errors::invalid_request("body", e.to_string()))),
        };
        let name = request.request.name.as_str();
        if name == ops::TOOL_WITHDRAW {
            return Ok(match self.withdraw(stamp, &request) {
                Ok(answer) => vec![ObservedFrame::Response(answer.encode())],
                Err(refusal) => error(refusal),
            });
        }
        if let Err(refusal) = check_call_key(&request) {
            return Ok(error(refusal));
        }
        Ok(match name {
            ops::ROLE_DESCRIBE => vec![ObservedFrame::Response(Self::describe())],
            ops::TOOL_CATALOG => vec![ObservedFrame::Response(self.catalog())],
            QUICK | SLOW => vec![ObservedFrame::Response(
                json!({ "content": request.request.arguments }),
            )],
            DISABLED => error(errors::tool_disabled(DISABLED)),
            HELD => self.hold(stamp, &request),
            other => error(ErrorBody::new("unknown_tool", format!("no tool {other}"))),
        })
    }

    fn hold(&self, stamp: &RouteStamp, request: &KeyedToolCallRequest) -> Vec<ObservedFrame> {
        let (Some(call_key), Some(scope)) = (&request.call_key, &stamp.scope) else {
            return error(errors::invalid_request(
                CALL_KEY_FIELD,
                "held calls need a call_key and a scope",
            ));
        };
        let key = (stamp.principal.clone(), call_key.clone());
        if !self.records.lock().unwrap().contains_key(&key) {
            let scope = ScopeIdentity {
                owner: scope.owner.clone(),
                scope_ref: scope.scope_ref.clone(),
                scope_epoch: scope.scope_epoch,
            };
            let marker = request.request.arguments["marker"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let extra = json!({ "scope": scope, "marker": marker });
            self.records.lock().unwrap().insert(
                key.clone(),
                Record {
                    scope,
                    marker: PathBuf::from(marker),
                    state: State::Prepared,
                },
            );
            if self
                .append(points::PREPARED, &key.0, &key.1, extra)
                .is_err()
            {
                return Vec::new();
            }
        }
        vec![ObservedFrame::Response(json!({ "pending": true }))]
    }

    fn withdraw(
        &self,
        stamp: &RouteStamp,
        request: &KeyedToolCallRequest,
    ) -> Result<WithdrawAnswer, ErrorBody> {
        let arguments = parse_withdraw_request(request)?;
        let scope = stamp
            .scope
            .as_ref()
            .map(|s| ScopeIdentity {
                owner: s.owner.clone(),
                scope_ref: s.scope_ref.clone(),
                scope_epoch: s.scope_epoch,
            })
            .ok_or_else(|| errors::withdraw_not_permitted("tool.withdraw needs a scoped route"))?;
        let carrier =
            resolve_carrier(&stamp.principal, &scope, &arguments).map_err(
                |problem| match problem {
                    CallerProblem::ScopeMismatch => {
                        errors::invalid_request("scope", "differs from the route's scope")
                    }
                    CallerProblem::OwnerMustNameCarrier => {
                        errors::invalid_request("carrier", "the owner must name the carrier")
                    }
                    CallerProblem::CarrierMismatch { named } => errors::withdraw_not_permitted(
                        format!("{} may not withdraw {named}'s calls", stamp.principal),
                    ),
                },
            )?;
        let key = (carrier, arguments.call_key);
        let Some(record) = self.records.lock().unwrap().get(&key).cloned() else {
            return Ok(if self.defects.refuse_unknown_call {
                WithdrawAnswer::Refused {
                    reason: RefusalReason::Denied,
                }
            } else {
                WithdrawAnswer::UnknownCall
            });
        };
        if record.scope != scope {
            return Err(errors::withdraw_not_permitted(
                "the call is under another scope",
            ));
        }
        Ok(match record.state {
            State::Prepared | State::Authorized => {
                if self.append("Withdrawn", &key.0, &key.1, json!({})).is_ok() {
                    self.set_state(&key, State::Withdrawn);
                }
                WithdrawAnswer::Withdrawn
            }
            State::Withdrawn => WithdrawAnswer::Withdrawn,
            State::Started => WithdrawAnswer::AlreadyStarted {
                outcome: StartedOutcome::Unknown,
            },
            State::Settled => WithdrawAnswer::Completed {
                result: CompletedResult::OutcomeOnly("ok".into()),
            },
        })
    }
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
    live: Mutex<Option<Arc<Module>>>,
}

impl FakeSubject {
    pub fn new(defects: Defects) -> Self {
        Self {
            defects,
            capabilities: [
                Capability::ScopeStamp,
                Capability::CallKey,
                Capability::HeldCalls,
                Capability::DisableTool,
                Capability::Cancellation,
                Capability::ApprovalExecution,
            ]
            .into(),
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
        Some(ScopedPrincipals {
            scope: ScopeIdentity {
                owner: "reserved:owner".into(),
                scope_ref: "scope-1".into(),
                scope_epoch: 1,
            },
            carrier: "reserved:broca".into(),
            other_carrier: "reserved:thalamus".into(),
            stranger: "reserved:stranger".into(),
        })
    }

    fn catalog_arguments(&self) -> Value {
        json!({ "params": {} })
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
            if self.live()?.holds(call_key) {
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
