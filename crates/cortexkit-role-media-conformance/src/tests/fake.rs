use crate::{
    fixtures::sha256,
    peers::{RecordingRoutingPeer, RecordingVendor},
    subject::*,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use cortexkit_role_media::describe::Engine;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Mutex,
};
mod stream_realtime;

pub(super) struct Fake {
    state: Mutex<State>,
    capabilities: Capabilities,
}
#[derive(Clone)]
struct Job {
    principal: String,
    request: Value,
    state: String,
    output: Value,
    failure: Option<Value>,
    attempt: u64,
    active: bool,
    admission: String,
    pin: Value,
    renewal: u64,
    next_renew: u64,
    deadline: Option<u64>,
    script: Script,
    steps: VecDeque<ScriptStep>,
    last_routing_code: Option<String>,
}
struct Upload {
    mime: String,
    size: usize,
    hash: String,
    bytes: Vec<u8>,
    state: String,
    appended: u64,
}
#[derive(Default)]
struct State {
    extra: stream_realtime::Extras,
    fault: Option<&'static str>,
    engines: Vec<Engine>,
    routing: RecordingRoutingPeer,
    principals: Option<Principals>,
    scripts: BTreeMap<String, Script>,
    held: BTreeSet<String>,
    inputs: BTreeMap<String, Vec<Value>>,
    jobs: BTreeMap<String, Job>,
    uploads: BTreeMap<(String, String), Upload>,
    objects: BTreeMap<(String, String), Vec<u8>>,
    usage: Vec<Value>,
    ack: u64,
    next_id: u64,
    retries: bool,
    survives: bool,
}
impl Fake {
    pub(super) fn with_speech_check(mut self, enabled: bool) -> Self {
        self.capabilities.has_speech_check = enabled;
        self.state.get_mut().unwrap().extra.speech_check = enabled;
        self
    }
    pub(super) fn new(fault: Option<&'static str>, retries: bool, survives: bool) -> Self {
        Self {
            state: Mutex::new(State {
                fault,
                retries,
                survives,
                ..State::default()
            }),
            capabilities: Capabilities::new()
                .with_retries_transient(retries)
                .with_jobs_survive_restart(survives),
        }
    }
}
fn refuse(code: &str, field: Option<&str>) -> SubjectError {
    let mut v = json!({"code":code,"message":"fixture refusal","detail":{"class":"permanent"}});
    if let Some(f) = field {
        v["detail"]["field"] = json!(f);
    }
    SubjectError::refused(v)
}
fn failed(code: &str, class: &str) -> Value {
    json!({"code":code,"class":class,"message":"fixture failure"})
}
impl State {
    fn fault(&self, name: &str) -> bool {
        self.fault == Some(name)
    }
    fn now(&self) -> u64 {
        self.routing.now_ms()
    }
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }
    fn observe(&self, job: &Job, kind: &str) -> SubjectResult<()> {
        if job.request["model"] == "local" {
            return Ok(());
        }
        self.routing.call("route.account_observe",json!({"kind":kind,"admission_id":job.admission,"event_id":format!("{}/{}",job.admission,if kind=="run_finished" {"finished"} else {"started"})}))?;
        Ok(())
    }
    fn admit(&self, id: &str, req: &Value, attempt: u64) -> SubjectResult<Value> {
        if req["model"] == "local" {
            return Ok(Value::Null);
        }
        let mut params = json!({"provider":"fixture","model":req["model"],"conversation_key":id,"admission_id":format!("{id}/{attempt}"),"hold_wait_max_s":0});
        if attempt == 1 {
            params["decision_id"] = req["decision_id"].clone();
        }
        if self.fault("routing_admission_per_attempt") {
            params["hold_wait_max_s"] = json!(900);
        }
        self.routing
            .call("route.account_for", params)
            .map(|v| v["pin"].clone())
            .map_err(|e| self.routing_refusal(e))
    }
    fn routing_refusal(&self, e: SubjectError) -> SubjectError {
        if let Some(raw) = e.refusal {
            let code = raw["code"].as_str().unwrap_or("unknown");
            let class = if matches!(
                code,
                "account_hold" | "account_pool_exhausted" | "account_selection_unavailable"
            ) {
                "transient"
            } else {
                "permanent"
            };
            let mut body = json!({"code":"routing_refused","message":"routing refused","detail":{"class":class,"routing_code":code}});
            if let Some(until) = raw["detail"]["until_ms"]
                .as_u64()
                .or_else(|| raw["detail"]["reset_at_ms"].as_u64())
            {
                body["detail"]["retry_after_ms"] = json!(until.saturating_sub(self.now()));
            }
            if self.fault("refused_submit_not_remembered") {
                body["detail"]["class"] = json!("permanent");
            }
            SubjectError::refused(body)
        } else {
            e
        }
    }
    fn record(&mut self, id: &str, j: &Job, status: &str, failure: Option<Value>) {
        if j.request["model"] == "local" {
            return;
        }
        let record_id = self.id("record");
        let mut ms = j.script.measurements.clone().unwrap_or_default();
        if ms.is_empty() {
            ms.push(json!({"role":"input_audio","unit":"second","quantity":"1","measure":"observed","source":"provider_measured"}));
        }
        if self.fault("usage_absent_is_not_zero") {
            ms.push(json!({"role":"output","unit":"token","quantity":"0","measure":"billed","source":"vendor_reply"}));
        }
        if self.fault("usage_no_rounding") {
            for m in &mut ms {
                m["quantity"] = json!("9007199254740993.0");
            }
        }
        if self.fault("usage_request_derived") {
            for m in &mut ms {
                if m["source"] == "request_derived" {
                    m["source"] = json!("provider_measured");
                }
            }
        }
        for role in ["input_image", "output_image", "output_video", "cache_read"] {
            if self.fault(&format!("usage_{role}")) {
                for m in &mut ms {
                    if m["role"] == role {
                        m["role"] = json!("output");
                    }
                }
            }
        }
        if self.fault("usage_quantity_bounds") {
            ms[0]["quantity"] = json!("0.1234567890");
        }
        if self.fault("usage_quantity_coefficient_bounds") {
            ms[0]["quantity"] = json!("9223372036.854775808");
        }
        let mut r = json!({"record_id":record_id,"principal":j.principal,"job_id":id,"provider":"fixture","model":j.request["model"],"charge_basis":"vendor","credential_ref":"fixture-credential","auth_method":"api_key","started_at_ms":self.now(),"finished_at_ms":self.now(),"status":status,"selectors":{},"measurements":ms});
        if j.attempt == 1 {
            r["decision_id"] = j.request["decision_id"].clone();
        }
        if let Some(v) = &j.script.vendor_request_id {
            r["vendor_request_id"] = json!(if v.chars().count() > 256 {
                format!("sha256:{}", sha256(v.as_bytes()))
            } else {
                v.clone()
            });
        }
        if let Some(mut f) = failure {
            if self.fault("failure_identical_across_surfaces") {
                f["message"] = json!("different");
            }
            r["failure"] = f;
        }
        if self.fault("usage_record_bounds") {
            r["vendor_request_id"] = json!("x".repeat(257));
        }
        if self.fault("usage_record_per_attempt") {
            r["record_id"] = json!("same-record");
            if status == "cancelled" {
                r["status"] = json!("done");
            }
        }
        self.usage
            .push(json!({"seq":self.usage.len()+1,"record":r}));
    }
    fn end(&mut self, id: &str, j: &mut Job, status: &str, f: Option<Value>) -> SubjectResult<()> {
        if j.active {
            self.record(id, j, status, f.clone());
        }
        self.observe(j, "run_finished")?;
        j.active = false;
        j.state = status.into();
        j.failure = f;
        Ok(())
    }
    fn process(&mut self, id: &str, j: &mut Job) -> SubjectResult<()> {
        let model = j.request["model"].as_str().unwrap().to_string();
        if j.state == "queued" {
            if self.held.contains(&model) {
                return Ok(());
            }
            j.state = "running".into();
            j.active = true;
            let input = self.forwarded_input(&j.request["input"]);
            self.inputs.entry(model.clone()).or_default().push(input);
            self.observe(j, "admission_started")?;
        }
        if !j.active {
            return Ok(());
        }
        while let Some(step) = j.steps.pop_front() {
            match step {
                ScriptStep::Pause => return Ok(()),
                ScriptStep::Failure(mut f) => {
                    f["message"] = json!(f["message"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(1024)
                        .collect::<String>());
                    let retry = self.retries
                        && (f["class"] == "transient" || self.fault("refusal_class_decides_retry"))
                        && matches!(
                            f["code"].as_str(),
                            Some("engine_crashed" | "model_loading" | "lane_paused")
                        );
                    self.record(id, j, "failed", Some(f.clone()));
                    self.observe(j, "run_finished")?;
                    j.active = false;
                    if retry {
                        j.attempt += 1;
                        j.admission = format!("{id}/{}", j.attempt);
                        j.pin = self.admit(id, &j.request, j.attempt)?;
                        j.renewal = 0;
                        j.next_renew = self.now() + 750;
                        j.state = "running".into();
                        j.active = true;
                        self.inputs
                            .entry(model.clone())
                            .or_default()
                            .push(j.request["input"].clone());
                        self.observe(j, "admission_started")?;
                    } else {
                        j.state = "failed".into();
                        j.failure = Some(f);
                        return Ok(());
                    }
                }
                ScriptStep::Output(mut out) => {
                    if j.request["input"].get("bias").is_some() {
                        out["bias_applied"] = if self.fault("bias_trim_phrase_limit") {
                            json!({"terms_used":5,"terms_dropped":0,"prompt_used":true})
                        } else {
                            json!({"terms_used":3,"terms_dropped":2,"prompt_used":false})
                        };
                    }
                    if j.script.decoded_duration_ms.is_some_and(|v| v > 2000)
                        && !self.fault("input_too_large")
                    {
                        let mut f = failed("input_too_large", "permanent");
                        f["detail"] = json!({"field":"input.audio","limit":"max_duration_ms"});
                        return self.end(id, j, "failed", Some(f));
                    }
                    let invalid = match j.request["kind"].as_str().unwrap() {
                        "transcribe" => {
                            out["text"].as_str().unwrap_or("").trim().is_empty()
                                && !self.fault("empty_transcript_is_a_failure")
                        }
                        "synthesize" => {
                            out["audio"]["bytes"] == 0 && !self.fault("empty_output_is_a_failure")
                        }
                        "image" => {
                            out["images"]
                                .as_array()
                                .is_none_or(|a| a.is_empty() || a.iter().any(|v| v["bytes"] == 0))
                                && !self.fault("empty_output_is_a_failure")
                        }
                        "video" => {
                            out["video"]["bytes"] == 0 && !self.fault("empty_output_is_a_failure")
                        }
                        _ => false,
                    };
                    if invalid {
                        return self.end(
                            id,
                            j,
                            "failed",
                            Some(failed("engine_empty_result", "transient")),
                        );
                    }
                    self.objectify(&j.principal, &mut out)?;
                    if serde_json::to_vec(&out).unwrap().len() > 3_145_728
                        && !self.fault("oversize_output_is_object")
                    {
                        let bytes = serde_json::to_vec(&out).unwrap();
                        out = json!({"output_object":self.store(&j.principal,&bytes,"application/json")});
                    }
                    if j.state == "cancelled" {
                        return self.end(id, j, "cancelled", None);
                    }
                    j.output = out;
                    return self.end(id, j, "done", None);
                }
                ScriptStep::Progress(_) | ScriptStep::Partial(_) | ScriptStep::Final(_) => (),
            }
        }
        if j.state == "cancelled" && j.active {
            self.end(id, j, "cancelled", None)?;
        }
        Ok(())
    }
    fn store(&mut self, p: &str, bytes: &[u8], mime: &str) -> Value {
        let id = self.id("object");
        self.objects.insert((p.into(), id.clone()), bytes.to_vec());
        json!({"mime":mime,"bytes":bytes.len(),"sha256":sha256(bytes),"object_id":id})
    }
    fn objectify(&mut self, p: &str, v: &mut Value) -> SubjectResult<()> {
        if let Some(data) = v.get("data").and_then(Value::as_str) {
            if v["bytes"].as_u64().unwrap_or(0) > 2_097_152 {
                let bytes = STANDARD.decode(data).unwrap();
                let stored = self.store(p, &bytes, v["mime"].as_str().unwrap());
                v.as_object_mut().unwrap().remove("data");
                v["object_id"] = stored["object_id"].clone();
            }
        } else if let Some(obj) = v.as_object_mut() {
            for child in obj.values_mut() {
                self.objectify(p, child)?;
            }
        } else if let Some(arr) = v.as_array_mut() {
            for child in arr {
                self.objectify(p, child)?;
            }
        }
        Ok(())
    }
    fn submit(&mut self, p: &str, req: Value) -> SubjectResult<Value> {
        if let Some((id, j)) = self
            .jobs
            .iter()
            .find(|(_, j)| j.principal == p && j.request["client_job_id"] == req["client_job_id"])
        {
            for field in ["kind", "input", "model", "decision_id", "deadline_ms"] {
                if j.request[field] != req[field] {
                    return Err(refuse(
                        "client_job_id_reuse",
                        Some(if self.fault("submit_refuses_client_job_id_reuse") {
                            "wrong"
                        } else {
                            field
                        }),
                    ));
                }
            }
            return Ok(
                json!({"job_id":if self.fault("submit_idempotent_on_client_job_id") {"new-id"} else {id}}),
            );
        }
        if !self.fault("client_job_id_bounds")
            && req["client_job_id"]
                .as_str()
                .is_none_or(|v| !(1..=128).contains(&v.chars().count()))
        {
            return Err(refuse("invalid_params", Some("client_job_id")));
        }
        let kind = req["kind"].as_str().unwrap_or("");
        if !self.fault("submit_refuses_outside_declared_facts") {
            if !matches!(kind, "transcribe" | "synthesize" | "image" | "video") {
                return Err(refuse("unsupported", Some("kind")));
            }
            let engine = self
                .engines
                .iter()
                .find(|e| e.kind.as_str() == kind && req["model"] == e.model)
                .ok_or_else(|| refuse("unsupported", Some("model")))?;
            for (field, value, allowed) in [
                (
                    "input.audio.mime",
                    req["input"]["audio"]["mime"].as_str(),
                    Some(&engine.formats),
                ),
                (
                    "input.format",
                    req["input"]["format"].as_str(),
                    Some(&engine.formats),
                ),
                (
                    "input.language",
                    req["input"]["language"].as_str(),
                    engine.languages.as_ref(),
                ),
                (
                    "input.voice",
                    req["input"]["voice"].as_str(),
                    engine.voices.as_ref(),
                ),
                (
                    "input.size",
                    req["input"]["size"].as_str(),
                    engine.sizes.as_ref(),
                ),
                (
                    "input.resolution",
                    req["input"]["resolution"].as_str(),
                    engine.resolutions.as_ref(),
                ),
                (
                    "input.quality",
                    req["input"]["quality"].as_str(),
                    engine.qualities.as_ref(),
                ),
            ] {
                if value.is_some_and(|v| !allowed.is_some_and(|a| a.iter().any(|s| s == v))) {
                    return Err(refuse("unsupported", Some(field)));
                }
            }
            if req["input"].get("bias").is_some() && engine.bias_limit.is_none() {
                return Err(refuse("unsupported", Some("input.bias")));
            }
            if let Some(refs) = req["input"]["references"].as_array() {
                for (i, r) in refs.iter().enumerate() {
                    if !engine
                        .reference_formats
                        .as_ref()
                        .is_some_and(|a| a.iter().any(|s| r["mime"] == *s))
                    {
                        return Err(refuse(
                            "unsupported",
                            Some(&format!("input.references[{i}]")),
                        ));
                    }
                }
            }
        }
        if req["model"] != "local"
            && req.get("decision_id").is_none()
            && !self.fault("remote_submit_requires_decision")
        {
            return Err(refuse("invalid_params", Some("decision_id")));
        }
        if req["input"]["audio"]["duration_ms"]
            .as_u64()
            .is_some_and(|v| v > 2000)
        {
            let mut e = refuse("input_too_large", Some("input.audio"));
            e.refusal.as_mut().unwrap()["detail"]["limit"] = json!("max_duration_ms");
            return Err(e);
        }
        if let Some(object) = req["input"]["audio"]["object_id"].as_str() {
            let u = self
                .uploads
                .get(&(p.into(), object.into()))
                .ok_or_else(|| refuse("unknown_object", None))?;
            if u.state != "committed" {
                return Err(refuse("upload_not_committed", None));
            }
            if (req["input"]["audio"]["mime"] != u.mime
                || req["input"]["audio"]["bytes"] != u.size
                || req["input"]["audio"]["sha256"] != u.hash)
                && !self.fault("committed_upload_is_object")
            {
                return Err(refuse("object_mismatch", Some("input.audio")));
            }
        }
        if req["kind"] == "transcribe"
            && req["input"]["audio"]["bytes"]
                .as_u64()
                .is_some_and(|n| n > 65_536)
        {
            let mut e = refuse("input_too_large", Some("input.audio"));
            e.refusal.as_mut().unwrap()["detail"]["limit"] = json!("max_input_bytes");
            return Err(e);
        }
        let id = self.id("job");
        let pin = self.admit(&id, &req, 1)?;
        let script = self
            .scripts
            .get(req["model"].as_str().unwrap())
            .cloned()
            .unwrap_or_default();
        let mut j = Job {
            principal: p.into(),
            request: req.clone(),
            state: "queued".into(),
            output: Value::Null,
            failure: None,
            attempt: 1,
            active: false,
            admission: format!("{id}/1"),
            pin,
            renewal: 0,
            next_renew: self.now() + 750,
            deadline: req["deadline_ms"].as_u64().map(|v| self.now() + v + 1),
            steps: script.steps.clone().into(),
            script,
            last_routing_code: None,
        };
        self.process(&id, &mut j)?;
        self.jobs.insert(id.clone(), j);
        Ok(json!({"job_id":id}))
    }
}

impl State {
    fn forwarded_input(&self, input: &Value) -> Value {
        let mut input = input.clone();
        if input.get("bias").is_some() && !self.fault("bias_trim_phrase_limit") {
            let terms = input["bias"]["terms"].as_array().unwrap();
            input["bias"] = json!({"terms":terms.iter().take(3).cloned().collect::<Vec<_>>()});
        }
        input
    }
    fn call(&mut self, p: &str, method: &str, params: Value) -> SubjectResult<Value> {
        match method {
            "media.stream_audio" => {
                let id = params["stream_id"].as_str().unwrap_or("").to_string();
                self.stream_frame(p, &id, params)?;
                Ok(json!({}))
            }
            "context.for_agent"
            | "realtime.token"
            | "realtime.call_started"
            | "realtime.call_ended" => self.extra_call(p, method, params),
            "role.describe" => {
                let mut v = json!({"groups":["jobs","usage_feed","stream_transcribe","realtime","bias_packs"],"jobs_survive_restart":self.survives,"token_max_ttl_ms":500,"call_silence_timeout_s":3,"engines":self.engines});
                if self.fault("wire_extension") {
                    for engine in v["engines"].as_array_mut().unwrap() {
                        if engine["kind"] == "future_kind" {
                            engine["kind"] = json!("transcribe");
                        }
                    }
                }
                if self.fault("describe_checker_codes") {
                    v["jobs_survive_restart"] = Value::Null;
                }
                Ok(v)
            }
            "media.submit" => self.submit(p, params),
            "media.status" | "media.result" | "media.cancel" => {
                if params
                    .as_object()
                    .is_some_and(|p| p.keys().any(|k| k != "job_id"))
                    && !self.fault("wire_unknown_member")
                {
                    return Err(refuse("invalid_params", Some("future_member")));
                }
                let id = params["job_id"].as_str().unwrap_or("");
                let foreign = self.fault("foreign_job_is_unknown");
                let mut j = self
                    .jobs
                    .get(id)
                    .filter(|j| j.principal == p || foreign)
                    .cloned()
                    .ok_or_else(|| refuse("unknown_job", None))?;
                let answer = match method {
                    "media.status" => {
                        let mut v = json!({"status":j.state});
                        if let Some(f) = &j.failure {
                            v["failure"] = f.clone();
                        }
                        v
                    }
                    "media.result" => {
                        if j.state != "done" {
                            let mut e = refuse("job_not_done", None);
                            e.refusal.as_mut().unwrap()["detail"]["status"] = json!(j.state);
                            return Err(e);
                        }
                        j.output.clone()
                    }
                    _ => {
                        if (j.state == "queued" || j.state == "running")
                            || self.fault("cancel_after_done_reports_done")
                        {
                            if !j.active {
                                self.observe(&j, "run_finished")?;
                            }
                            j.state = "cancelled".into();
                            j.failure = None;
                        }
                        if self.fault("cancel_never_changes_terminal_status") {
                            j.state = "done".into();
                        }
                        json!({"status":j.state})
                    }
                };
                self.jobs.insert(id.into(), j);
                Ok(answer)
            }
            "usage.read" | "usage.ack" => {
                if self
                    .principals
                    .as_ref()
                    .is_none_or(|pr| pr.usage_reader != p)
                    && !self.fault("usage_feed_admits_only_reader")
                {
                    return Err(refuse("feed_not_admitted", None));
                }
                if method == "usage.ack" {
                    let seq = params["through_seq"].as_u64().unwrap();
                    if seq > self.usage.len() as u64
                        && !self.fault("usage_ack_beyond_written_refused")
                    {
                        return Err(refuse("invalid_params", Some("through_seq")));
                    }
                    self.ack = self.ack.max(seq);
                    return Ok(
                        json!({"acked_through_seq":if self.fault("usage_ack_answer") {seq+1} else {self.ack}}),
                    );
                }
                let mut entries: Vec<Value> = self
                    .usage
                    .iter()
                    .filter(|v| v["seq"].as_u64().unwrap() > self.ack)
                    .cloned()
                    .collect();
                if self.fault("usage_feed_redelivers_until_ack") {
                    self.ack = self.usage.len() as u64;
                }
                if self.fault("usage_feed_oversize_entry_acks") {
                    for e in &mut entries {
                        e["raw"]["sha256"] = json!("wrong");
                    }
                }
                let mut out = json!({"entries":entries});
                if let Some(last) = entries.last() {
                    out["last_seq"] = last["seq"].clone();
                }
                Ok(out)
            }
            _ => self.object_call(p, method, params),
        }
    }
}

impl State {
    fn object_call(&mut self, p: &str, method: &str, params: Value) -> SubjectResult<Value> {
        match method {
            "media.upload_begin" => {
                let id = self.id("upload");
                self.uploads.insert(
                    (p.into(), id.clone()),
                    Upload {
                        mime: params["mime"].as_str().unwrap().into(),
                        size: params["bytes"].as_u64().unwrap() as usize,
                        hash: params["sha256"].as_str().unwrap().into(),
                        bytes: Vec::new(),
                        state: "open".into(),
                        appended: self.now(),
                    },
                );
                Ok(json!({"upload_id":id}))
            }
            "media.upload_status" | "media.upload_append" | "media.upload_commit" => {
                let id = params["upload_id"].as_str().unwrap();
                let now = self.now();
                let bad_resume = self.fault("upload_resumes_at_received");
                let u = self
                    .uploads
                    .get_mut(&(p.into(), id.into()))
                    .ok_or_else(|| refuse("unknown_upload", None))?;
                if method == "media.upload_status" {
                    return Ok(json!({"state":u.state,"received":u.bytes.len()}));
                }
                if u.state == "gone" {
                    return Err(refuse("upload_gone", None));
                }
                if method == "media.upload_append" {
                    if u.state == "committed" {
                        return Err(refuse("invalid_params", Some("upload_id")));
                    }
                    if params["offset"] != u.bytes.len() {
                        let mut e = refuse("upload_offset_mismatch", None);
                        e.refusal.as_mut().unwrap()["detail"]["received"] =
                            json!(if bad_resume { 0 } else { u.bytes.len() });
                        return Err(e);
                    }
                    let bytes = STANDARD.decode(params["data"].as_str().unwrap()).unwrap();
                    if bytes.len() > 2_097_152 {
                        return Err(refuse("upload_chunk_too_large", None));
                    }
                    u.bytes.extend(bytes);
                    u.appended = now;
                    return Ok(json!({"received":u.bytes.len()}));
                }
                if u.bytes.len() != u.size {
                    return Err(refuse("upload_size_mismatch", None));
                }
                if sha256(&u.bytes) != u.hash {
                    return Err(refuse("upload_sha256_mismatch", None));
                }
                u.state = "committed".into();
                self.objects.insert((p.into(), id.into()), u.bytes.clone());
                Ok(json!({"state":"committed","sha256":u.hash,"mime":u.mime,"bytes":u.size}))
            }
            "media.read" => {
                let id = params["object_id"].as_str().unwrap();
                let bytes = self
                    .objects
                    .get(&(p.into(), id.into()))
                    .ok_or_else(|| refuse("unknown_object", None))?;
                let offset = params["offset"].as_u64().unwrap() as usize;
                let length = params["length"].as_u64().unwrap() as usize;
                if !(1..=2_097_152).contains(&length) {
                    return Err(refuse("invalid_params", Some("length")));
                }
                if offset > bytes.len() {
                    return Err(refuse("invalid_params", Some("offset")));
                }
                let mut slice = bytes[offset..bytes.len().min(offset + length)].to_vec();
                if self.fault("chunked_result_checksum") && !slice.is_empty() {
                    slice[0] ^= 1;
                }
                Ok(json!({"data":STANDARD.encode(slice)}))
            }
            _ => Err(SubjectError::new(format!(
                "unknown fixture method {method}"
            ))),
        }
    }

    fn restart(&mut self) -> SubjectResult<()> {
        self.extra_restart()?;
        for u in self.uploads.values_mut() {
            if u.state == "open" && self.fault != Some("upload_gone_after_restart") {
                u.state = "gone".into();
            }
        }
        let jobs = std::mem::take(&mut self.jobs);
        for (id, mut j) in jobs {
            if j.active {
                let cancelled = j.state == "cancelled";
                if cancelled && self.fault("cancel_then_restart_records_cancelled") {
                    self.end(
                        &id,
                        &mut j,
                        "failed",
                        Some(failed("module_restarted", "transient")),
                    )?;
                } else if cancelled {
                    self.end(&id, &mut j, "cancelled", None)?;
                } else if self.fault("running_restart_records_failure") {
                    j.state = "failed".into();
                    j.active = false;
                    j.failure = Some(failed("module_restarted", "transient"));
                } else {
                    self.end(
                        &id,
                        &mut j,
                        "failed",
                        Some(failed("module_restarted", "transient")),
                    )?;
                }
            } else if j.state == "queued" && !self.survives {
                self.end(
                    &id,
                    &mut j,
                    "failed",
                    Some(failed("module_restarted", "transient")),
                )?;
            }
            if self.fault("restart_keeps_terminal_jobs") && j.state == "done" {
                j.output = Value::Null;
            }
            self.jobs.insert(id, j);
        }
        Ok(())
    }
    fn advance(&mut self, ms: u64) -> SubjectResult<()> {
        let target = self.now() + ms;
        while self.now() < target {
            let next = self
                .jobs
                .values()
                .filter(|j| j.state == "queued" || j.active)
                .flat_map(|j| {
                    let mut times = vec![j.next_renew];
                    if let Some(d) = j.deadline {
                        times.push(d);
                    }
                    times
                })
                .chain(self.extra_times())
                .filter(|v| *v > self.now())
                .min()
                .unwrap_or(target)
                .min(target);
            self.routing.advance_clock(next - self.now());
            let jobs = std::mem::take(&mut self.jobs);
            for (id, mut j) in jobs {
                if !(j.state == "queued" || j.active) {
                    self.jobs.insert(id, j);
                    continue;
                }
                if j.deadline.is_some_and(|d| d <= self.now())
                    && !self.fault("deadline_counts_queue_time")
                {
                    self.end(
                        &id,
                        &mut j,
                        "failed",
                        Some(failed("deadline_exceeded", "transient")),
                    )?;
                } else if j.request["model"] != "local" && j.next_renew <= self.now() {
                    if j.pin["lease_valid_until_ms"]
                        .as_u64()
                        .is_some_and(|v| v <= self.now())
                        && !self.fault("renewal_lapse_ends_work")
                    {
                        let mut f = failed("routing_refused", "transient");
                        f["detail"] = json!({"routing_code":j.last_routing_code.as_deref().unwrap_or("account_hold")});
                        self.end(&id, &mut j, "failed", Some(f))?;
                    } else {
                        j.renewal += 1;
                        let params = json!({"provider":"fixture","model":j.request["model"],"conversation_key":id,"admission_id":format!("{}/renew/{}",j.admission,j.renewal),"mode":"renew","expected_pin":{"id":j.pin["id"],"generation":j.pin["generation"]},"hold_wait_max_s":0});
                        match self.routing.call("route.account_for", params) {
                            Ok(v) => {
                                j.pin = v["pin"].clone();
                                j.next_renew = self.now() + 750;
                            }
                            Err(e) => {
                                j.last_routing_code = e
                                    .refusal
                                    .and_then(|v| v["code"].as_str().map(str::to_string));
                                j.next_renew = (self.now() + 100)
                                    .min(j.pin["lease_valid_until_ms"].as_u64().unwrap());
                            }
                        }
                    }
                }
                self.jobs.insert(id, j);
            }
            self.extra_advance()?;
        }
        for u in self.uploads.values_mut() {
            if u.state == "open" && target.saturating_sub(u.appended) >= 3_600_000 {
                u.state = "gone".into();
            }
        }
        if self.fault("objects_retained_for_one_hour") {
            self.objects.clear();
        }
        Ok(())
    }
}

#[async_trait]
impl MediaSubject for Fake {
    async fn configure_voice_front(
        &self,
        agent_id: &str,
        reason: Option<&str>,
    ) -> SubjectResult<()> {
        let mut state = self.state.lock().unwrap();
        if let Some(reason) = reason {
            state
                .extra
                .front_unavailable
                .insert(agent_id.into(), reason.into());
        } else {
            state.extra.front_unavailable.remove(agent_id);
        }
        Ok(())
    }
    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }
    async fn configure_engines(&self, engines: Vec<Engine>) -> SubjectResult<()> {
        self.state.lock().unwrap().engines = engines;
        Ok(())
    }
    async fn engine_declaration(&self) -> SubjectResult<Vec<Engine>> {
        let st = self.state.lock().unwrap();
        let mut es = st.engines.clone();
        if st.fault("describe_matches_engine_declaration") {
            es[0].formats.reverse();
        }
        if st.fault("describe_declaration_omits_provider") {
            es.pop();
        }
        if st.fault("describe_declaration_duplicates_provider") {
            let last = es.len() - 1;
            es[last] = es[0].clone();
        }
        es.reverse();
        Ok(es)
    }
    async fn configure_peers(
        &self,
        routing: RecordingRoutingPeer,
        vendor: RecordingVendor,
    ) -> SubjectResult<()> {
        let mut st = self.state.lock().unwrap();
        st.routing = routing;
        st.extra.vendor = vendor;
        Ok(())
    }
    async fn configure_principals(&self, principals: Principals) -> SubjectResult<()> {
        self.state.lock().unwrap().principals = Some(principals);
        Ok(())
    }
    async fn call_as(&self, principal: &str, request: Request) -> SubjectResult<Value> {
        let mut st = self.state.lock().unwrap();
        let answer = st.call(principal, &request.method, request.params)?;
        if st.fault("reply_envelope") {
            Ok(answer)
        } else if st.fault("reply_envelope_extra_keys") {
            Ok(json!({"result":answer,"extra":true}))
        } else {
            Ok(json!({"result":answer}))
        }
    }
    async fn configure_pack(
        &self,
        agent_id: &str,
        purpose: &str,
        pack: Value,
    ) -> SubjectResult<()> {
        self.state
            .lock()
            .unwrap()
            .extra
            .packs
            .insert((agent_id.into(), purpose.into()), pack);
        Ok(())
    }
    async fn open_stream(&self, p: &str, first: Value) -> SubjectResult<()> {
        self.state.lock().unwrap().open_stream(p, first)
    }
    async fn open_watch(&self, p: &str, id: &str) -> SubjectResult<()> {
        self.state.lock().unwrap().watch_open(p, id)
    }
    async fn poll_watch(&self, p: &str, id: &str) -> SubjectResult<Vec<Value>> {
        self.state.lock().unwrap().watch_poll(p, id)
    }
    async fn stream_audio_inputs(&self, p: &str, id: &str) -> SubjectResult<Vec<Vec<u8>>> {
        self.state.lock().unwrap().stream_audio_inputs(p, id)
    }
    async fn poll_stream(&self, p: &str, id: &str) -> SubjectResult<StreamObservation> {
        self.state.lock().unwrap().stream_poll(p, id)
    }
    async fn close_stream(&self, p: &str, id: &str) -> SubjectResult<()> {
        self.state.lock().unwrap().extra_close(p, id)
    }
    async fn refuse_next_stream(&self, code: &str) -> SubjectResult<()> {
        self.state.lock().unwrap().refuse_stream(code)
    }
    async fn script_engine(&self, model: &str, script: Script) -> SubjectResult<()> {
        self.state
            .lock()
            .unwrap()
            .scripts
            .insert(model.into(), script);
        Ok(())
    }
    async fn engine_inputs(&self, model: &str) -> SubjectResult<Vec<Value>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .inputs
            .get(model)
            .cloned()
            .unwrap_or_default())
    }
    async fn hold_engine(&self, model: &str) -> SubjectResult<()> {
        self.state.lock().unwrap().held.insert(model.into());
        Ok(())
    }
    async fn release_engine(&self, model: &str) -> SubjectResult<()> {
        let mut st = self.state.lock().unwrap();
        st.held.remove(model);
        st.extra_release(model)?;
        let jobs = std::mem::take(&mut st.jobs);
        for (id, mut j) in jobs {
            if j.request["model"] == model {
                if j.state == "queued" {
                    if let Some(script) = st.scripts.get(model) {
                        j.script = script.clone();
                        j.steps = script.steps.clone().into();
                    }
                }
                st.process(&id, &mut j)?;
            }
            st.jobs.insert(id, j);
        }
        Ok(())
    }
    async fn restart(&self) -> SubjectResult<()> {
        self.state.lock().unwrap().restart()
    }
    async fn advance_clock(&self, ms: u64) -> SubjectResult<()> {
        self.state.lock().unwrap().advance(ms)
    }
    async fn seed_usage_entry(&self, bytes: Vec<u8>) -> SubjectResult<()> {
        let mut st = self.state.lock().unwrap();
        let raw = if bytes.len() > 262_144 {
            json!({"bytes_b64":"","reason":"oversize","bytes":bytes.len(),"sha256":sha256(&bytes)})
        } else {
            json!({"bytes_b64":STANDARD.encode(bytes),"reason":"undecodable"})
        };
        let seq = st.usage.len() + 1;
        st.usage.push(json!({"seq":seq,"raw":raw}));
        Ok(())
    }
}
