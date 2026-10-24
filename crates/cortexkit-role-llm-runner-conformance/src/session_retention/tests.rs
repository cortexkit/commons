//! A small subprocess runner defends the policy cases. It is a suite fixture,
//! not an implementation claiming conformance. Its pending-release test really
//! kills the child and reloads only synced state from the same root.

use super::*;
use crate::route::{RouteFailure, SubscribeOutcome};
use async_trait::async_trait;
use cortexkit_role_harness::{
    Harness, HarnessError, KillMechanism, KillReport, PointDeclaration, RouteStamp, ScopeStamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex},
};

const CONTROL_FAULT: &str = "none";

struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Process {
    fn call(
        &mut self,
        session: &str,
        principal: &str,
        method: &str,
        params: Value,
    ) -> Result<Reply, RouteFailure> {
        writeln!(
            self.input,
            "{}",
            json!({"session":session,"principal":principal,"method":method,"params":params})
        )
        .map_err(|e| RouteFailure::new(e.to_string()))?;
        self.input
            .flush()
            .map_err(|e| RouteFailure::new(e.to_string()))?;
        loop {
            let mut line = String::new();
            if self
                .output
                .read_line(&mut line)
                .map_err(|e| RouteFailure::new(e.to_string()))?
                == 0
            {
                return Err(RouteFailure::new("child exited"));
            }
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if let Some(error) = value.get("error") {
                    return Ok(Reply::Error(serde_json::from_value(error.clone()).unwrap()));
                }
                if let Some(result) = value.get("result") {
                    return Ok(Reply::Response(result.clone()));
                }
            }
        }
    }
}

#[derive(Clone)]
struct Route {
    process: Arc<Mutex<Process>>,
    session: String,
    principal: String,
}
#[async_trait]
impl RunnerRoute for Route {
    async fn request(&self, method: &str, params: Value) -> Result<Reply, RouteFailure> {
        self.process
            .lock()
            .unwrap()
            .call(&self.session, &self.principal, method, params)
    }
    async fn subscribe(&self, _: Value) -> Result<SubscribeOutcome, RouteFailure> {
        Ok(SubscribeOutcome::Events(vec![]))
    }
}

struct Fake {
    fault: &'static str,
    required: bool,
    floor: u32,
    current: Mutex<Option<Arc<Mutex<Process>>>>,
}
impl Fake {
    fn new(fault: &'static str, required: bool) -> Self {
        Self {
            fault,
            required,
            floor: 0,
            current: Mutex::new(None),
        }
    }
    fn control(&self, session: &str, method: &str, params: Value) -> Result<(), HarnessError> {
        let process = self.current.lock().unwrap().as_ref().unwrap().clone();
        let reply = process
            .lock()
            .unwrap()
            .call(session, "starter", method, params)
            .map_err(|e| HarnessError::new(e.message))?;
        expect_ok(method, reply)
            .map(|_| ())
            .map_err(HarnessError::new)
    }
}

#[async_trait]
impl Harness for Fake {
    type Handle = Arc<Mutex<Process>>;
    type Route = Route;
    fn declared_points(&self) -> Vec<PointDeclaration> {
        vec![PointDeclaration {
            point: KillPoint::new(points::RELEASE_PENDING),
            mechanisms: vec![KillMechanism::ProcessSignal],
            note: "child.kill followed by wait; state is loaded from disk".into(),
        }]
    }
    async fn spawn(&self, root: &Path) -> Result<Self::Handle, HarnessError> {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session_retention::tests::process_server",
                "--ignored",
                "--nocapture",
            ])
            .env("CK_RETENTION_FAKE_ROOT", root)
            .env("CK_RETENTION_FAKE_FAULT", self.fault)
            .env("CK_RETENTION_FAKE_REQUIRED", self.required.to_string())
            .env("CK_RETENTION_FAKE_FLOOR", self.floor.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| HarnessError::new(e.to_string()))?;
        let process = Arc::new(Mutex::new(Process {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        }));
        *self.current.lock().unwrap() = Some(process.clone());
        Ok(process)
    }
    async fn route(&self, h: &Self::Handle, stamp: &RouteStamp) -> Result<Route, HarnessError> {
        self.session_route(h, "", stamp).await
    }
    async fn kill_at(
        &self,
        h: Self::Handle,
        point: &KillPoint,
        trigger: Trigger<'_>,
    ) -> Result<KillReport, HarnessError> {
        trigger.await;
        let mut process = h.lock().unwrap();
        process
            .child
            .kill()
            .map_err(|e| HarnessError::new(e.to_string()))?;
        let status = process
            .child
            .wait()
            .map_err(|e| HarnessError::new(e.to_string()))?;
        assert!(
            !status.success(),
            "the child must have been killed, not exited normally"
        );
        Ok(KillReport {
            point: point.clone(),
            mechanism: KillMechanism::ProcessSignal,
        })
    }
    async fn restart(&self, root: &Path) -> Result<Self::Handle, HarnessError> {
        self.spawn(root).await
    }
}

#[async_trait]
impl LlmRunnerSubject for Fake {
    fn runner_id(&self) -> &str {
        "runner-a"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        [
            Capability::SessionRetention,
            Capability::Retention,
            Capability::Adopt,
            Capability::Queue,
            Capability::TranscriptReads,
            Capability::RunOps,
            Capability::Attachments,
            Capability::RetentionClock,
        ]
        .into()
    }
    fn owner_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "starter".into(),
            scope: None,
        }
    }
    fn stranger_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "parent".into(),
            scope: None,
        }
    }
    fn retention_scope_stamps(&self) -> Option<(RouteStamp, RouteStamp)> {
        let scope = Some(ScopeStamp {
            owner: "parent".into(),
            scope_ref: "parent-scope".into(),
            scope_epoch: 1,
        });
        Some((
            RouteStamp {
                principal: "starter".into(),
                scope: scope.clone(),
            },
            RouteStamp {
                principal: "parent".into(),
                scope,
            },
        ))
    }
    async fn session_route(
        &self,
        h: &Self::Handle,
        session: &str,
        stamp: &RouteStamp,
    ) -> Result<Route, HarnessError> {
        Ok(Route {
            process: h.clone(),
            session: session.into(),
            principal: stamp.principal.clone(),
        })
    }
    fn send_fields(&self, _: &str, _: bool) -> Map<String, Value> {
        Map::new()
    }
    fn tool_provider_module(&self) -> String {
        "scripted-tool".into()
    }
    async fn install_script(&self, session: &str, script: Script) -> Result<(), HarnessError> {
        self.control(session, "fixture.script", json!({"answers":script.turns.iter().map(|turn| turn.text_parts().collect::<Vec<_>>().join("")).collect::<Vec<_>>()}))
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
        tokio::task::yield_now().await;
    }
    async fn prepare_runner_retention(
        &self,
        _: &Self::Handle,
        session: &str,
    ) -> Result<bool, HarnessError> {
        self.control(session, "fixture.runner", json!({}))?;
        Ok(true)
    }
    fn request_dumps_unsupported(&self) -> bool {
        true
    }
    fn host_plan(&self, _: &str, _: bool) -> Option<Map<String, Value>> {
        Some(Map::new())
    }
    async fn start_host_session(
        &self,
        _: &Self::Handle,
        session: &str,
        _: &Map<String, Value>,
        _: Option<&str>,
    ) -> Result<(), HarnessError> {
        self.control(session, "fixture.runner", json!({}))
    }
    async fn hold_retention_queue(&self, session: &str) -> Result<bool, HarnessError> {
        self.control(session, "fixture.queue", json!({}))?;
        Ok(true)
    }
    async fn release_retention_queue(&self, session: &str) -> Result<(), HarnessError> {
        self.control(session, "fixture.drain", json!({}))
    }
    async fn hold_next_model_step(&self, session: &str) -> Result<(), HarnessError> {
        if self.fault == "unavailable_hold" {
            return Err(HarnessError::new("held model fixture is unavailable"));
        }
        self.control(session, "fixture.hold", json!({}))
    }
    async fn await_model_step(&self, session: &str) -> Result<(), HarnessError> {
        self.control(session, "fixture.held", json!({}))
    }
    async fn release_model_step(&self, session: &str) -> Result<(), HarnessError> {
        self.control(session, "fixture.finish", json!({}))
    }
    async fn advance_retention_clock(&self, session: &str, ms: u64) -> Result<(), HarnessError> {
        self.control(session, "fixture.clock", json!({"ms":ms}))
    }
    async fn model_invocations(&self, session: &str) -> Result<usize, HarnessError> {
        let process = self.current.lock().unwrap().as_ref().unwrap().clone();
        let reply = process
            .lock()
            .unwrap()
            .call(session, "starter", "fixture.invocations", json!({}))
            .map_err(|e| HarnessError::new(e.message))?;
        Ok(expect_ok("model invocations", reply)
            .map_err(HarnessError::new)?
            .as_u64()
            .unwrap() as usize)
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Stored {
    policy: Option<Value>,
    source: String,
    owner: String,
    incarnation: u64,
    sends: u64,
    active: bool,
    queued: bool,
    pending: bool,
    released_at: Option<u64>,
    media: Option<String>,
    activity: u64,
    runner_origin: bool,
    queue: Vec<Value>,
    log: Vec<Value>,
    archive: Vec<Value>,
    started: u64,
    completed: u64,
}
#[derive(Default, Serialize, Deserialize)]
struct Disk {
    sessions: BTreeMap<String, Stored>,
    next: u64,
    clock: u64,
    // Scripted-provider state is independent of the runner-owned session data.
    scripts: BTreeMap<String, Vec<String>>,
    observed: BTreeMap<String, Vec<Value>>,
    leaked_media: BTreeSet<String>,
}

fn append(s: &mut Stored, role: &str, text: &str) {
    let ordinal = s.log.len() as u64;
    s.log.push(json!({"ordinal":ordinal,"mid":format!("{}-{ordinal}",s.incarnation),"message":{"role":role,"content":[{"type":"text","text":text}]}}));
}

fn complete(s: &mut Stored, script: &[String]) {
    if s.active {
        append(
            s,
            "assistant",
            script
                .get(s.completed as usize)
                .map(String::as_str)
                .unwrap_or("policy-answer"),
        );
        s.completed += 1;
        s.active = false;
    }
}

fn start(s: &mut Stored, params: Value, script: &[String], observed: &mut Vec<Value>, hold: bool) {
    if !s.archive.is_empty() {
        s.log = std::mem::take(&mut s.archive);
    }
    append(s, "user", params["prompt"].as_str().unwrap());
    observed.push(params);
    s.started += 1;
    s.active = true;
    if !hold {
        complete(s, script);
    }
}

fn failure(code: &str, detail: Value) -> Value {
    json!({"error":{"code":code,"message":code,"detail":detail}})
}

fn handle(
    disk: &mut Disk,
    held: &mut BTreeSet<String>,
    queue_held: &mut BTreeSet<String>,
    fault: &str,
    required: bool,
    floor: u32,
    request: &Value,
) -> Value {
    let name = request["session"].as_str().unwrap();
    let principal = request["principal"].as_str().unwrap();
    let method = request["method"].as_str().unwrap();
    let params = &request["params"];
    let answer = |v| json!({"result":v});
    if method == ops::ROLE_DESCRIBE {
        let mut result = json!({"majors":[{"version":"llm-runner/v1","ops":["role.describe","session.baseline","session.send","session.adopt","session.head","session.read","session.release","run.result","session.upload_begin","session.upload_append","session.upload_commit","session.upload_status","session.media_read"],"stability":"alpha"}],"implementation_version":"fixture","capabilities":["retention","session_retention","adopt","queue","transcript_reads","run_ops","attachments"],"max_bytes":{"default":1048576,"maximum":1048576},"session_capabilities_from":"admission","retention":{"max_seconds":60,"delete_within_ms":1},"session_retention":{"max_days":3650,"max_unreleased_hold_ms":10,"required":required}});
        if floor > 0 {
            result["session_retention"]["min_hold_days"] = json!(floor);
        }
        if fault == "invalid_floor" {
            result["session_retention"]["min_hold_days"] = json!(3651);
        }
        if fault == "undeclared" {
            result["capabilities"] = json!(["retention"]);
        }
        return answer(result);
    }
    if method == "fixture.script" {
        disk.scripts.insert(
            name.into(),
            serde_json::from_value(params["answers"].clone()).unwrap(),
        );
        return answer(json!({}));
    }
    if method == "fixture.invocations" {
        return answer(json!(disk.observed.get(name).map_or(0, Vec::len)));
    }
    if method == "fixture.hold" {
        held.insert(name.into());
        return answer(json!({}));
    }
    if method == "fixture.queue" {
        queue_held.insert(name.into());
        return answer(json!({}));
    }
    if method == "fixture.held" {
        assert!(disk.sessions[name].active);
        assert!(!disk.observed[name].is_empty());
        return answer(json!({}));
    }
    if method == "fixture.finish" || method == "fixture.drain" {
        held.remove(name);
        queue_held.remove(name);
        if let Some(s) = disk.sessions.get_mut(name) {
            let script = disk.scripts.get(name).cloned().unwrap_or_default();
            complete(s, &script);
            for params in std::mem::take(&mut s.queue) {
                start(
                    s,
                    params,
                    &script,
                    disk.observed.entry(name.into()).or_default(),
                    false,
                );
            }
            s.queued = false;
            s.activity = disk.clock;
            if s.pending && floor == 0 {
                disk.sessions.remove(name);
            }
        }
        return answer(json!({}));
    }
    if method == "fixture.runner" {
        disk.next += 1;
        disk.sessions.insert(
            name.into(),
            Stored {
                policy: Some(policy(json!({"days":2}), false)),
                source: "runner".into(),
                incarnation: disk.next,
                runner_origin: true,
                ..Stored::default()
            },
        );
        return answer(json!({}));
    }
    if method == "fixture.clock" {
        disk.clock += params["ms"].as_u64().unwrap();
        let due = disk.sessions.get(name).is_some_and(|s| {
            let p = s.policy.as_ref().unwrap();
            let ms = if p["keep"] == "none" {
                Some(10)
            } else {
                p["keep"]["days"].as_u64().map(|d| d * 86_400_000)
            };
            let floor_ms = if fault == "early_floor" {
                0
            } else {
                u64::from(floor) * DAY_MS
            };
            let deadline = if s.pending {
                Some(s.activity.max(s.released_at.unwrap()) + floor_ms)
            } else {
                ms.map(|ms| {
                    s.activity
                        + if fault == "add_floor" {
                            ms + floor_ms
                        } else {
                            ms.max(floor_ms)
                        }
                })
            };
            deadline.is_some_and(|deadline| disk.clock >= deadline)
                && ((!s.active && !s.queued)
                    || fault == "delete_active" && s.active
                    || fault == "delete_queue" && s.queued)
                && fault != "ignore"
                && !(fault == "ignore_days" && p["keep"].is_object())
        });
        if due {
            disk.sessions.remove(name);
        } else if let Some(s) = disk.sessions.get_mut(name) {
            let archive_days = s
                .policy
                .as_ref()
                .and_then(|p| p["archive_after_days"].as_u64())
                .unwrap_or(0);
            if archive_days != 0
                && disk.clock - s.activity >= archive_days * 86_400_000
                && !s.active
                && !s.queued
                && !s.log.is_empty()
            {
                s.archive = std::mem::take(&mut s.log);
            }
        }
        return answer(json!({}));
    }
    if method == ops::SESSION_SEND || method == ops::SESSION_ADOPT {
        let adopting = method == ops::SESSION_ADOPT;
        let p = params.get("retention_policy");
        if p.is_some() && fault == "undeclared" {
            return failure(errors::INVALID_PARAMS, json!({"field":"retention_policy"}));
        }
        if p.is_some() && params.get("retention").is_some() {
            return failure(errors::INVALID_PARAMS, json!({"field":"retention_policy"}));
        }
        let first =
            !disk.sessions.contains_key(name) || adopting && disk.sessions[name].owner.is_empty();
        if first
            && p.is_none()
            && (required != (fault == "required" || fault == "adopt_required" && adopting))
            && !(fault == "legacy_required"
                && !adopting
                && required
                && params.get("retention").is_some())
        {
            return failure(errors::RETENTION_REQUIRED, json!({}));
        }
        let decoded =
            p.map(cortexkit_role_llm_runner::session_retention::RetentionPolicy::from_value);
        if let Some(Err(field)) = &decoded {
            if !(fault == "backup_none"
                && field == "retention_policy.backup"
                && params["retention_policy"]["keep"] == "none"
                && params["retention_policy"]["backup"] == true)
            {
                return failure(errors::INVALID_PARAMS, json!({"field":field}));
            }
        }
        if let Some(s) = disk.sessions.get(name) {
            if s.pending {
                return failure(
                    errors::SESSION_RELEASED,
                    if fault == "missing_deadline" {
                        json!({})
                    } else {
                        json!({"delete_after_ms":s.activity.max(s.released_at.unwrap()) + u64::from(floor) * DAY_MS})
                    },
                );
            }
            if s.source == "starter"
                && p.is_some()
                && s.policy.as_ref() != p
                && fault != "conflict"
                && !(fault == "replace_twice" && s.runner_origin)
                && !(fault == "adopt_conflict" && adopting)
            {
                return failure(errors::RETENTION_CONFLICT, json!({"frozen":s.policy}));
            }
        }
        if !disk.sessions.contains_key(name) {
            disk.next += 1;
            disk.sessions.insert(
                name.into(),
                Stored {
                    owner: principal.into(),
                    incarnation: disk.next,
                    ..Stored::default()
                },
            );
        }
        let s = disk.sessions.get_mut(name).unwrap();
        if s.owner.is_empty() {
            s.owner = principal.into();
        }
        if let Some(p) = p {
            s.policy = Some(p.clone());
            s.source = "starter".into();
            if fault == "rewrite_dumps" {
                s.policy.as_mut().unwrap()["request_dumps"] = json!(false);
            }
        }
        s.activity = disk.clock;
        if adopting {
            return answer(json!({"owner_bound":true,"applied_plan_digest":"host-plan"}));
        }
        s.sends += 1;
        let queued = s.active || queue_held.contains(name);
        if queued {
            s.queue.push(params.clone());
            s.queued = true;
        } else {
            start(
                s,
                params.clone(),
                disk.scripts
                    .get(name)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                disk.observed.entry(name.into()).or_default(),
                held.contains(name),
            );
        }
        if let Some(content) = params["content"].as_array() {
            if !content.is_empty() {
                s.media = Some("referenced".into());
            }
        }
        let mut result = if queued {
            json!({"state":"pending","submission_id":format!("send-{}",s.sends)})
        } else {
            json!({"state":if s.active {"active"} else {"finished"},"run_id":format!("run-{}", s.started),"reason":"completed"})
        };
        if first {
            result["baseline"] =
                serde_json::to_value(cortexkit_role_llm_runner::baseline::Baseline::default())
                    .unwrap();
        }
        return answer(result);
    }
    if method == ops::SESSION_RELEASE {
        let Some(s) = disk.sessions.get_mut(name) else {
            return answer(
                json!({"state":if fault == "nonidempotent" {"deleted"} else {"absent"}}),
            );
        };
        if principal != s.owner && fault != "nonowner" {
            return failure(errors::SCOPE_OWNER_MISMATCH, json!({}));
        }
        if principal == s.owner && fault == "deny_owner" {
            return failure(errors::SCOPE_OWNER_MISMATCH, json!({}));
        }
        let repeated = s.pending;
        s.pending = true;
        s.released_at.get_or_insert(disk.clock);
        if (s.active
            || s.queued
            || disk.clock < s.activity.max(s.released_at.unwrap()) + u64::from(floor) * DAY_MS)
            && !(fault == "repeat_deleted" && repeated)
        {
            return answer(json!({"state":"pending"}));
        }
        if fault == "media_deleted" && s.media.is_some() {
            disk.leaked_media.insert(name.into());
        }
        if fault != "readable_deleted" {
            disk.sessions.remove(name);
        } else {
            disk.sessions.get_mut(name).unwrap().pending = true;
        }
        return answer(json!({"state":"deleted"}));
    }
    let Some(s) = disk.sessions.get(name) else {
        if method == ops::SESSION_MEDIA_READ && disk.leaked_media.contains(name) {
            return answer(json!({"data":"iQ==","size":68,"mime":"image/png"}));
        }
        return failure(
            if method == ops::SESSION_MEDIA_READ {
                errors::UNKNOWN_MEDIA
            } else {
                errors::UNKNOWN_SESSION
            },
            json!({}),
        );
    };
    if method == ops::SESSION_HEAD {
        let mut result = json!({"lineage_id":format!("lineage-{}",s.incarnation),"first_ordinal":0,"history_version":1,"updated_at":s.activity,"incarnation":format!("incarnation-{}",s.incarnation),"last_run_state":{"run_id":format!("run-{}",s.started),"state":if s.active {"running"} else {"completed"}},"retention_policy":s.policy,"retention_state":{"source":s.source}});
        if s.pending && fault != "omit_released" && fault != "omit_zero_released" {
            let at = s.released_at.unwrap();
            result["released"] = json!({"at_ms":at,"delete_after_ms":s.activity.max(at) + u64::from(floor) * DAY_MS + u64::from(fault == "wrong_deadline")});
            if fault == "release_from_activity" {
                result["released"]["delete_after_ms"] =
                    json!(s.activity + u64::from(floor) * DAY_MS);
            }
        }
        if s.started == 0 {
            result.as_object_mut().unwrap().remove("last_run_state");
        }
        if s.policy
            .as_ref()
            .is_some_and(|p| p["request_dumps"] == true)
        {
            result["retention_state"]["request_dumps_unsupported"] = json!(true);
        }
        return answer(result);
    }
    if method == ops::SESSION_READ {
        if s.pending && fault == "released_unreadable" {
            return failure(errors::UNKNOWN_SESSION, json!({}));
        }
        return answer(
            json!({"lineage_id":format!("lineage-{}",s.incarnation),"messages":if s.archive.is_empty() {&s.log} else {&s.archive}}),
        );
    }
    if method == ops::SESSION_UPLOAD_BEGIN {
        return answer(json!({"upload_id":"upload"}));
    }
    if method == ops::SESSION_BASELINE {
        return answer(
            json!({"state":"ready","baseline":cortexkit_role_llm_runner::baseline::Baseline::default()}),
        );
    }
    if method == ops::RUN_RESULT {
        return answer(
            json!({"run_id":params["run_id"],"state":if s.active {"running"} else {"completed"},"final_message":{"mid":"answer","text":"policy-answer"}}),
        );
    }
    if method == ops::SESSION_UPLOAD_STATUS {
        return answer(json!({"state":"committed","received":68}));
    }
    if method == ops::SESSION_UPLOAD_APPEND {
        return answer(json!({"received":1}));
    }
    if method == ops::SESSION_UPLOAD_COMMIT {
        return answer(json!({"state":"committed"}));
    }
    if method == ops::SESSION_MEDIA_READ {
        return if s.media.is_some() {
            answer(json!({"data":"iQ==","size":68,"mime":"image/png"}))
        } else {
            failure(errors::UNKNOWN_MEDIA, json!({}))
        };
    }
    failure(errors::INVALID_PARAMS, json!({}))
}

#[test]
#[ignore = "spawned as the policy fake's child process"]
fn process_server() {
    let root = std::path::PathBuf::from(std::env::var("CK_RETENTION_FAKE_ROOT").unwrap());
    let path = root.join("state.json");
    let fault = std::env::var("CK_RETENTION_FAKE_FAULT").unwrap();
    let required = std::env::var("CK_RETENTION_FAKE_REQUIRED").unwrap() == "true";
    let floor: u32 = std::env::var("CK_RETENTION_FAKE_FLOOR")
        .unwrap()
        .parse()
        .unwrap();
    let mut disk: Disk = std::fs::read(&path)
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or_default();
    disk.sessions.retain(|_, s| {
        !(s.pending
            && !s.active
            && !s.queued
            && disk.clock >= s.activity.max(s.released_at.unwrap()) + u64::from(floor) * DAY_MS)
    });
    let mut held = BTreeSet::new();
    let mut queue_held = BTreeSet::new();
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let response = handle(
            &mut disk,
            &mut held,
            &mut queue_held,
            &fault,
            required,
            floor,
            &request,
        );
        if fault == "immediate_none" {
            // Simulate a valid short hold elapsing before the caller can read
            // the terminal record, never while work is active or queued.
            let due = |s: &Stored| {
                s.policy.as_ref().is_some_and(|p| p["keep"] == "none") && !s.active && !s.queued
            };
            if disk.sessions.values().any(due) {
                disk.clock = disk.clock.saturating_add(10);
            }
            disk.sessions.retain(|_, s| !due(s));
        }
        let mut saved: Disk = serde_json::from_value(serde_json::to_value(&disk).unwrap()).unwrap();
        if fault == "memory_pending" {
            for s in saved.sessions.values_mut() {
                s.pending = false;
            }
        }
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&serde_json::to_vec(&saved).unwrap())
            .unwrap();
        file.sync_all().unwrap();
        println!("{response}");
        std::io::stdout().flush().unwrap();
    }
}

async fn exercise(name: &str, fault: &'static str, required: bool) -> Result<Ending, String> {
    let floor = if matches!(
        name,
        "session_retention_floor" | "session_retention_release_hold"
    ) {
        7
    } else {
        0
    };
    exercise_floor(name, fault, required, floor).await
}

async fn exercise_floor(
    name: &str,
    fault: &'static str,
    required: bool,
    floor: u32,
) -> Result<Ending, String> {
    let temp = tempfile::tempdir().unwrap();
    let mut fake = Fake::new(fault, required);
    fake.floor = floor;
    let mut driver = CrashDriver::new(&fake);
    let handle = driver.spawn(temp.path()).await.unwrap();
    let mut declared = fake.capabilities();
    declared.insert(Capability::KillAt(points::RELEASE_PENDING));
    let mint = Mint::default();
    let result = if name == "session_retention_pending_durable" {
        crash(&fake, &mut driver, temp.path(), &declared, &mint).await
    } else {
        run(
            &Case {
                subject: &fake,
                handle: &handle,
                declared: &declared,
                mint: &mint,
            },
            name,
        )
        .await
    };
    // Drop routes and handles before removing the state root.
    *fake.current.lock().unwrap() = None;
    drop(handle);
    result
}

macro_rules! defended {
    ($name:ident, $fault:literal) => {
        #[tokio::test]
        async fn $name() {
            let name = stringify!($name);
            assert!(matches!(
                exercise(name, CONTROL_FAULT, false)
                    .await
                    .unwrap_or_else(|e| panic!("{name}: {e}")),
                Ending::Passed
            ));
            let error = exercise(name, $fault, false)
                .await
                .err()
                .unwrap_or_else(|| panic!("{name}: faulty runner escaped"));
            let expected = match $fault {
                "ignore" | "ignore_days" | "readable_deleted" | "media_deleted" => "not refused",
                "required" => "retention_required",
                "conflict" | "nonowner" | "backup_none" | "replace_twice" | "adopt_conflict" => {
                    "not refused"
                }
                "deny_owner" => "scope_owner_mismatch",
                "nonidempotent" => "release expected absent",
                "delete_active" | "delete_queue" => "unknown_session",
                "rewrite_dumps" => "rewrote or ignored",
                "memory_pending" => "recorded release did not delete",
                "invalid_floor" => "invalid_session_retention",
                "early_floor" | "released_unreadable" => "unknown_session",
                "omit_released" | "omit_zero_released" => "omitted released",
                "wrong_deadline" => "incorrect delete_after_ms",
                "missing_deadline" => "detail.delete_after_ms",
                "repeat_deleted" => "release expected pending",
                "add_floor" => "not refused",
                _ => panic!("missing expected failure for {}", $fault),
            };
            assert!(
                error.contains(expected),
                "{name}: wrong failure for {}: {error}",
                $fault
            );
            println!("{name}: faulty runner rejected: {error}");
        }
    };
}
defended!(session_retention_floor_describe, "invalid_floor");
defended!(session_retention_floor, "early_floor");
defended!(session_retention_release_hold, "released_unreadable");

#[tokio::test]
async fn session_retention_release_after_idle() {
    let name = "session_retention_release_after_idle";
    for floor in [1, 7] {
        assert!(matches!(
            exercise_floor(name, CONTROL_FAULT, false, floor)
                .await
                .unwrap_or_else(|e| panic!("{name}: {e}")),
            Ending::Passed
        ));
        let error = exercise_floor(name, "release_from_activity", false, floor)
            .await
            .err()
            .unwrap_or_else(|| panic!("{name}: faulty runner escaped"));
        assert!(
            error.contains("incorrect delete_after_ms"),
            "{name}: wrong failure: {error}"
        );
        println!("{name}: faulty runner rejected: {error}");
    }
    assert!(matches!(
        exercise_floor(name, CONTROL_FAULT, false, 0).await.unwrap(),
        Ending::Inapplicable(_)
    ));
}
defended!(session_retention_zero_floor_drain, "omit_zero_released");

#[tokio::test]
async fn session_retention_release_hold_faults() {
    for (fault, expected) in [
        ("omit_released", "omitted released"),
        ("wrong_deadline", "incorrect delete_after_ms"),
        ("missing_deadline", "detail.delete_after_ms"),
        ("repeat_deleted", "release expected pending"),
    ] {
        let error = exercise("session_retention_release_hold", fault, false)
            .await
            .err()
            .unwrap();
        assert!(error.contains(expected), "{fault}: {error}");
    }
    assert!(exercise("session_retention_floor", "add_floor", false)
        .await
        .is_err());
}

defended!(session_retention_honoured, "ignore");
defended!(session_retention_days_and_archive, "ignore_days");
defended!(session_retention_adopt_policy, "adopt_conflict");
defended!(session_retention_queued_never_deleted, "delete_queue");
defended!(session_retention_required, "required");
defended!(session_retention_conflict, "conflict");
defended!(session_retention_owner_can_release, "deny_owner");
defended!(session_retention_parent_cannot_release, "nonowner");
defended!(session_retention_release_idempotent, "nonidempotent");
defended!(session_retention_deletion_observable, "readable_deleted");
defended!(session_retention_media_deleted, "media_deleted");
defended!(session_retention_active_never_deleted, "delete_active");
defended!(session_retention_backup_none_refused, "backup_none");
defended!(session_retention_dumps_preserved, "rewrite_dumps");
defended!(session_retention_runner_replaced_once, "replace_twice");
defended!(session_retention_pending_durable, "memory_pending");

#[tokio::test]
async fn session_retention_existing_cases_honour_floor() {
    assert!(matches!(
        exercise_floor(
            "session_retention_zero_floor_drain",
            "unavailable_hold",
            false,
            0
        )
        .await
        .unwrap(),
        Ending::Inapplicable(_)
    ));
    for name in [
        "session_retention_honoured",
        "session_retention_days_and_archive",
        "session_retention_owner_can_release",
        "session_retention_release_idempotent",
        "session_retention_deletion_observable",
        "session_retention_media_deleted",
        "session_retention_active_never_deleted",
        "session_retention_queued_never_deleted",
        "session_retention_adopt_policy",
        "session_retention_pending_durable",
    ] {
        assert!(matches!(
            exercise_floor(name, CONTROL_FAULT, false, 7)
                .await
                .unwrap_or_else(|e| panic!("{name}: {e}")),
            Ending::Passed
        ));
    }
    assert!(matches!(
        exercise_floor("session_retention_release_hold", CONTROL_FAULT, false, 0)
            .await
            .unwrap(),
        Ending::Passed
    ));
    assert!(matches!(
        exercise_floor("session_retention_floor", CONTROL_FAULT, false, 0)
            .await
            .unwrap(),
        Ending::Inapplicable(_)
    ));
    assert!(matches!(
        exercise_floor(
            "session_retention_zero_floor_drain",
            CONTROL_FAULT,
            false,
            7
        )
        .await
        .unwrap(),
        Ending::Inapplicable(_)
    ));
}

#[tokio::test]
async fn session_retention_required_true() {
    let name = "session_retention_required";
    assert!(matches!(
        exercise(name, CONTROL_FAULT, true).await.unwrap(),
        Ending::Passed
    ));
    assert!(exercise(name, "required", true).await.is_err());
}

#[tokio::test]
async fn session_retention_adopt_required_true() {
    let name = "session_retention_adopt_policy";
    assert!(matches!(
        exercise(name, CONTROL_FAULT, true).await.unwrap(),
        Ending::Passed
    ));
    assert!(exercise(name, "adopt_required", true).await.is_err());
}

#[tokio::test]
async fn session_retention_legacy_required() {
    let name = "session_retention_legacy_required";
    assert!(matches!(
        exercise(name, CONTROL_FAULT, true).await.unwrap(),
        Ending::Passed
    ));
    assert!(exercise(name, "legacy_required", true).await.is_err());
    for required in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let fake = Fake::new("none", required);
        let mut driver = CrashDriver::new(&fake);
        let handle = driver.spawn(temp.path()).await.unwrap();
        let declared = fake.capabilities();
        let mint = Mint::default();
        let case = Case {
            subject: &fake,
            handle: &handle,
            declared: &declared,
            mint: &mint,
        };
        assert_eq!(
            inapplicable(&case, "retention_honoured").await.unwrap(),
            if required {
                Some(
                    "session_retention required: legacy retention-only first sends are refused"
                        .into(),
                )
            } else {
                None
            }
        );
    }
}

#[tokio::test]
async fn session_retention_starter_rollout() {
    for (fault, required) in [("none", false), ("none", true), ("undeclared", false)] {
        let temp = tempfile::tempdir().unwrap();
        let fake = Fake::new(fault, required);
        let mut driver = CrashDriver::new(&fake);
        let handle = driver.spawn(temp.path()).await.unwrap();
        let route = fake
            .session_route(handle.inner(), "default", &fake.owner_stamp())
            .await
            .unwrap();
        expect_ok(
            "starter first",
            crate::drive::call(
                &route,
                ops::SESSION_SEND,
                json!({"send_id":"s1","prompt":"p"}),
            )
            .await
            .unwrap(),
        )
        .unwrap();
        let head = ok(&route, ops::SESSION_HEAD, json!({})).await.unwrap();
        assert_eq!(
            head["retention_policy"],
            if fault == "undeclared" {
                Value::Null
            } else {
                policy(json!("forever"), false)
            }
        );
        if fault != "undeclared" {
            let legacy = fake
                .session_route(handle.inner(), "prefer-new", &fake.owner_stamp())
                .await
                .unwrap();
            expect_ok(
                "prefer new",
                crate::drive::call(
                    &legacy,
                    ops::SESSION_SEND,
                    json!({"send_id":"s1","prompt":"p","retention":5}),
                )
                .await
                .unwrap(),
            )
            .unwrap();
            state(&legacy, &policy(json!("forever"), false), "starter")
                .await
                .unwrap();
            let hosted = fake
                .session_route(handle.inner(), "hosted", &fake.owner_stamp())
                .await
                .unwrap();
            fake.start_host_session(handle.inner(), "hosted", &Map::new(), None)
                .await
                .unwrap();
            expect_ok(
                "starter adopt",
                crate::drive::call(
                    &hosted,
                    ops::SESSION_ADOPT,
                    json!({"send_id":"a1","plan":{}}),
                )
                .await
                .unwrap(),
            )
            .unwrap();
            state(&hosted, &policy(json!("forever"), false), "starter")
                .await
                .unwrap();
            let route = fake
                .session_route(handle.inner(), "custom", &fake.owner_stamp())
                .await
                .unwrap();
            let custom = policy(json!({"days":2}), true);
            ok(
                &route,
                ops::SESSION_SEND,
                json!({"send_id":"s1","prompt":"p","retention_policy":custom}),
            )
            .await
            .unwrap();
            expect_ok(
                "starter repeat",
                crate::drive::call(
                    &route,
                    ops::SESSION_SEND,
                    json!({"send_id":"s2","prompt":"p"}),
                )
                .await
                .unwrap(),
            )
            .unwrap();
            state(&route, &custom, "starter").await.unwrap();
        }
    }
}

#[tokio::test]
async fn session_retention_short_fallback_can_delete_before_terminal_read() {
    for name in [
        "session_retention_honoured",
        "session_retention_release_idempotent",
        "session_retention_active_never_deleted",
    ] {
        assert!(matches!(
            exercise(name, "immediate_none", false)
                .await
                .unwrap_or_else(|e| panic!("{name}: {e}")),
            Ending::Passed
        ));
    }
}
