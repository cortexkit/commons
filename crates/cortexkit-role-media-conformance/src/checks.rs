use base64::{engine::general_purpose::STANDARD, Engine as _};
use cortexkit_role_media::{
    describe::{check_describe, Describe, Engine},
    jobs::{check_status, StatusAnswer},
    usage::{check_usage_answer, check_usage_record, UsageReadAnswer, UsageRecord},
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

mod stream_realtime;
mod wire;
pub use stream_realtime::CASES as STREAM_REALTIME_CASES;

/// Wall-clock ceiling for asynchronous scripted work; never advances provider time.
pub const STATUS_POLL_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

use crate::{
    fixtures::{sha256, AudioFixture},
    peers::{RecordingRoutingPeer, RecordingVendor},
    subject::{Agent, MediaSubject, Principals, Request, Script, ScriptStep},
};

pub const CASES: &[&str] = &[
    "reply_envelope",
    "stream_reply_envelope",
    "wire_extension",
    "realtime_watch",
    "submit_idempotent_on_client_job_id",
    "submit_refuses_client_job_id_reuse",
    "refused_submit_not_remembered",
    "submit_refuses_outside_declared_facts",
    "input_too_large",
    "remote_submit_requires_decision",
    "client_job_id_bounds",
    "describe_matches_engine_declaration",
    "describe_checker_codes",
    "deadline_counts_queue_time",
    "refusal_class_decides_retry",
    "empty_transcript_is_a_failure",
    "empty_output_is_a_failure",
    "usage_absent_is_not_zero",
    "usage_no_rounding",
    "usage_quantity_bounds",
    "usage_quantity_coefficient_bounds",
    "usage_record_bounds",
    "usage_feed_redelivers_until_ack",
    "usage_feed_admits_only_reader",
    "usage_ack_beyond_written_refused",
    "usage_ack_answer",
    "usage_feed_oversize_entry_acks",
    "failure_identical_across_surfaces",
    "usage_record_per_attempt",
    "restart_keeps_terminal_jobs",
    "cancel_then_restart_records_cancelled",
    "running_restart_records_failure",
    "chunked_result_checksum",
    "upload_resumes_at_received",
    "upload_gone_after_restart",
    "committed_upload_is_object",
    "oversize_output_is_object",
    "objects_retained_for_one_hour",
    "cancel_after_done_reports_done",
    "cancel_never_changes_terminal_status",
    "foreign_job_is_unknown",
    "routing_admission_per_attempt",
    "renewal_lapse_ends_work",
    "stream_input_limits",
    "stream_id_reuse_refused",
    "stream_first_frame_refused",
    "stream_restart",
    "stream_oversize_final_splits",
    "stream_partials_replace_finals_append",
    "stream_empty_result_needs_speech_check",
    "stream_remote_requires_decision",
    "stream_id_bounds",
    "bias_trim_phrase_limit",
    "context_pack_admission",
    "context_pack_if_version",
    "realtime_token_uses_requested_model",
    "realtime_token_requires_decision",
    "realtime_token_refuses_unadmitted_principal",
    "realtime_token_bound_to_call",
    "realtime_token_within_max_ttl",
    "realtime_token_refuses_recorded_call",
    "call_started_replay_after_expiry",
    "call_started_refuses_swapped_vendor_call",
    "call_started_refuses_other_principal",
    "calls_scoped_by_principal",
    "call_ended_unrecorded",
    "ended_call_never_reattaches",
    "call_end_writes_one_record",
    "call_id_bounds",
    "stream_realtime_admission_lifecycle",
    "stream_realtime_renewal_lapse",
];

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    Passed,
    NotApplicable(String),
    Failed(String),
}

/// Run one named case against a fresh subject, reporting the case on all failures.
pub async fn run_case<S: MediaSubject>(subject: &S, name: &str) -> Outcome {
    if !CASES.contains(&name) {
        return Outcome::Failed(format!("{name}: unknown case"));
    }
    if name == "running_restart_records_failure" && !subject.capabilities().jobs_survive_restart {
        return Outcome::NotApplicable("jobs_survive_restart is false".into());
    }
    let routing = RecordingRoutingPeer::new();
    let vendor = RecordingVendor::new();
    let result = async {
        subject.configure_engines(engines()).await.map_err(err)?;
        subject
            .configure_peers(routing.clone(), vendor.clone())
            .await
            .map_err(err)?;
        subject
            .configure_principals(Principals::new(
                "reader",
                vec![Agent::new("agent", "alice").with_context_readers(vec!["bob".into()])],
            ))
            .await
            .map_err(err)?;
        if name == "reply_envelope" {
            let describe = call(subject, "alice", "role.describe", json!({})).await?;
            require(describe["engines"].is_array(), "describe omitted engines")
        } else if name == "wire_extension" {
            wire::run(subject).await
        } else if STREAM_REALTIME_CASES.contains(&name) {
            stream_realtime::run(subject, &routing, &vendor, name).await
        } else {
            run(subject, &routing, name).await
        }
    }
    .await;
    match result {
        Ok(()) => Outcome::Passed,
        Err(reason) => Outcome::Failed(format!("{name}: {reason}")),
    }
}

pub(crate) fn engines() -> Vec<Engine> {
    let mut engines = Vec::new();
    for (kind, model, local, formats) in [
        (
            "transcribe",
            "transcribe",
            false,
            vec!["audio/wav", "audio/L16;rate=16000;channels=1"],
        ),
        ("transcribe", "local", true, vec!["audio/wav"]),
        ("synthesize", "synthesize", false, vec!["audio/wav"]),
        ("image", "image", false, vec!["image/png"]),
        ("video", "video", false, vec!["video/mp4"]),
        (
            "stream_transcribe",
            "stream",
            false,
            vec!["audio/wav", "audio/L16;rate=16000;channels=1"],
        ),
    ] {
        engines.push(
            serde_json::from_value(json!({"kind":kind, "model":model, "local":local,
            "provider":if local { "local" } else { "fixture" }, "formats":formats,
            "reference_formats":["image/png"], "languages":["en"], "voices":["voice"],
            "sizes":["small"], "resolutions":["720p"], "qualities":["standard"],
            "max_input_bytes":if kind == "transcribe" {65_536} else {4_194_304}, "max_duration_ms":2_000, "latency_class":"batch"}))
            .unwrap(),
        );
    }
    engines
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn call<S: MediaSubject>(
    s: &S,
    principal: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let body = s
        .call_as(principal, Request::new(method, params))
        .await
        .map_err(|e| format!("{method}: {e}; {:?}", e.refusal))?;
    decode_reply(body).map_err(|e| format!("{method}: {e}"))
}
fn decode_reply(body: Value) -> Result<Value, String> {
    let bytes = serde_json::to_vec(&body).map_err(err)?;
    cortexkit_role_media::wire::read_reply(&bytes).map_err(|e| format!("{}: {e}", e.name()))
}
async fn refusal<S: MediaSubject>(
    s: &S,
    principal: &str,
    method: &str,
    params: Value,
    code: &str,
    field: Option<&str>,
) -> Result<Value, String> {
    match s.call_as(principal, Request::new(method, params)).await {
        Ok(v) => Err(format!("{method} accepted instead of {code}: {v}")),
        Err(e) => {
            let v = e.refusal.ok_or_else(|| e.message.clone())?;
            require(v["code"] == code, &format!("expected {code}, got {v}"))?;
            require(v["detail"]["class"].is_string(), "refusal omitted class")?;
            if let Some(field) = field {
                require(
                    v["detail"]["field"] == field,
                    &format!("expected field {field}: {v}"),
                )?;
            }
            Ok(v)
        }
    }
}
pub(crate) fn media(bytes: &[u8], mime: &str) -> Value {
    json!({"mime":mime, "bytes":bytes.len(), "sha256":sha256(bytes), "data":STANDARD.encode(bytes)})
}
pub(crate) fn transcript(text: &str) -> Value {
    json!({"text":text, "words":[], "no_speech":false})
}
pub(crate) fn request(id: &str, kind: &str) -> Value {
    let input = match kind {
        "synthesize" => json!({"text":"hello", "voice":"voice", "format":"audio/wav"}),
        "image" => json!({"prompt":"a shape", "size":"small", "format":"image/png"}),
        "video" => {
            json!({"prompt":"a shape", "duration_s":1, "resolution":"720p", "format":"video/mp4"})
        }
        _ => json!({"audio":media(&AudioFixture::Tone.wav(), "audio/wav")}),
    };
    json!({"client_job_id":id, "kind":kind, "input":input, "model":kind, "decision_id":"decision"})
}
async fn script<S: MediaSubject>(s: &S, model: &str, steps: Vec<ScriptStep>) -> Result<(), String> {
    s.script_engine(model, Script::new(steps))
        .await
        .map_err(err)
}
async fn submit<S: MediaSubject>(s: &S, params: Value) -> Result<String, String> {
    let v = call(s, "alice", "media.submit", params).await?;
    let id = v["job_id"].as_str().ok_or("missing job_id")?;
    require(!id.is_empty() && id.chars().count() <= 64, "job_id length")?;
    Ok(id.into())
}
async fn status<S: MediaSubject>(s: &S, id: &str) -> Result<Value, String> {
    let v = call(s, "alice", "media.status", json!({"job_id":id})).await?;
    let typed: StatusAnswer = serde_json::from_value(v.clone()).map_err(err)?;
    check_status(&typed).map_err(err)?;
    Ok(v)
}
async fn expect_status<S: MediaSubject>(s: &S, id: &str, expected: &str) -> Result<Value, String> {
    let started = Instant::now();
    let mut last = Value::Null;
    loop {
        let remaining = STATUS_POLL_TIMEOUT.saturating_sub(started.elapsed());
        let v = tokio::time::timeout(remaining, status(s, id))
            .await
            .map_err(|_| format!("expected {expected}; last status: {last}"))??;
        if v["status"] == expected {
            return Ok(v);
        }
        if !matches!(v["status"].as_str(), Some("queued" | "running"))
            || started.elapsed() >= STATUS_POLL_TIMEOUT
        {
            return Err(format!("expected {expected}; last status: {v}"));
        }
        last = v;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
async fn result<S: MediaSubject>(s: &S, id: &str) -> Result<Value, String> {
    expect_status(s, id, "done").await?;
    call(s, "alice", "media.result", json!({"job_id":id})).await
}
async fn cancel<S: MediaSubject>(s: &S, id: &str, expected: &str) -> Result<(), String> {
    let v = call(s, "alice", "media.cancel", json!({"job_id":id})).await?;
    require(
        v["status"] == expected,
        &format!("cancel expected {expected}: {v}"),
    )
}
async fn feed<S: MediaSubject>(s: &S) -> Result<Value, String> {
    let v = call(s, "reader", "usage.read", json!({"limit":1000})).await?;
    let answer: UsageReadAnswer = serde_json::from_value(v.clone()).map_err(err)?;
    check_usage_answer(&answer).map_err(err)?;
    Ok(v)
}
async fn records<S: MediaSubject>(s: &S) -> Result<Vec<Value>, String> {
    let f = feed(s).await?;
    let entries = f["entries"].as_array().ok_or("missing entries")?;
    let mut out = Vec::new();
    for e in entries {
        let r = e.get("record").ok_or("expected record entry")?;
        if r["model"] == "local" {
            continue;
        }
        let typed: UsageRecord = serde_json::from_value(r.clone()).map_err(err)?;
        check_usage_record(&typed).map_err(err)?;
        require(
            r["principal"] == "alice" && r["provider"] == "fixture",
            "usage attribution",
        )?;
        out.push(r.clone());
    }
    Ok(out)
}

async fn wait_records<S: MediaSubject>(s: &S, count: usize) -> Result<Vec<Value>, String> {
    let started = Instant::now();
    loop {
        let remaining = STATUS_POLL_TIMEOUT.saturating_sub(started.elapsed());
        let rs = tokio::time::timeout(remaining, records(s))
            .await
            .map_err(|_| "usage.read did not finish within the polling ceiling".to_string())??;
        if rs.len() >= count {
            return Ok(rs);
        }
        if started.elapsed() >= STATUS_POLL_TIMEOUT {
            return Err(format!(
                "expected {count} remote records; last count: {}",
                rs.len()
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
async fn completed<S: MediaSubject>(s: &S, id: &str) -> Result<String, String> {
    script(
        s,
        "transcribe",
        vec![ScriptStep::Output(transcript("scripted words"))],
    )
    .await?;
    let id = submit(s, request(id, "transcribe")).await?;
    expect_status(s, &id, "done").await?;
    wait_records(s, 1).await?;
    Ok(id)
}
fn failure(code: &str, class: &str) -> Value {
    json!({"code":code,"class":class,"message":"scripted failure"})
}
fn calls(r: &RecordingRoutingPeer, method: &str) -> Vec<Value> {
    r.calls()
        .into_iter()
        .filter(|(m, _)| m == method)
        .map(|(_, v)| v)
        .collect()
}
fn events(r: &RecordingRoutingPeer, event: &str) -> Vec<Value> {
    calls(r, "route.account_observe")
        .into_iter()
        .filter(|v| v["kind"] == event)
        .collect()
}

async fn run<S: MediaSubject>(s: &S, r: &RecordingRoutingPeer, name: &str) -> Result<(), String> {
    match name {
        "submit_idempotent_on_client_job_id" => {
            let id = completed(s, "replay").await?;
            require(
                submit(s, request("replay", "transcribe")).await? == id,
                "replay changed job",
            )?;
            require(
                s.engine_inputs("transcribe").await.map_err(err)?.len() == 1,
                "replay ran engine",
            )?;
            require(records(s).await?.len() == 1, "replay wrote usage")
        }
        "submit_refuses_client_job_id_reuse" => {
            let original = request("reuse", "transcribe");
            let id = completed(s, "reuse").await?;
            for field in ["kind", "input", "model", "decision_id", "deadline_ms"] {
                let mut changed = original.clone();
                changed[field] = match field {
                    "input" => json!({"audio":media(b"different", "audio/wav")}),
                    "deadline_ms" => json!(5),
                    _ => json!("different"),
                };
                refusal(
                    s,
                    "alice",
                    "media.submit",
                    changed,
                    "client_job_id_reuse",
                    Some(field),
                )
                .await?;
                require(
                    submit(s, original.clone()).await? == id,
                    "refusal changed remembered request",
                )?;
                require(
                    result(s, &id).await?["text"] == "scripted words",
                    "refusal changed result",
                )?;
            }
            require(
                s.engine_inputs("transcribe").await.map_err(err)?.len() == 1,
                "refused reuse started work",
            )
        }
        "refused_submit_not_remembered" => {
            r.refuse_next(json!({"code":"account_hold", "message":"fixture hold", "detail":{"until_ms":1000}}));
            let first = refusal(
                s,
                "alice",
                "media.submit",
                request("refused", "transcribe"),
                "routing_refused",
                None,
            )
            .await?;
            require(
                first["detail"]["routing_code"] == "account_hold"
                    && first["detail"]["class"] == "transient"
                    && first["detail"]["retry_after_ms"] == 1000,
                "routing refusal translation",
            )?;
            require(records(s).await?.is_empty(), "refused submit wrote usage")?;
            let mut retry = request("refused", "transcribe");
            retry["decision_id"] = json!("new-decision");
            script(
                s,
                "transcribe",
                vec![ScriptStep::Output(transcript("accepted"))],
            )
            .await?;
            let id = submit(s, retry).await?;
            expect_status(s, &id, "done").await.map(|_| ())
        }
        "submit_refuses_outside_declared_facts" => {
            for (kind, field, bad) in [
                ("transcribe", "kind", json!("stream_transcribe")),
                ("transcribe", "model", json!("absent")),
                ("transcribe", "input.audio.mime", json!("audio/unknown")),
                ("synthesize", "input.format", json!("audio/unknown")),
                ("transcribe", "input.language", json!("fr")),
                ("synthesize", "input.voice", json!("unknown")),
                ("image", "input.size", json!("huge")),
                ("video", "input.resolution", json!("8k")),
                ("image", "input.quality", json!("unknown")),
                ("image", "input.references[0]", media(b"ref", "image/jpeg")),
                ("transcribe", "input.bias", json!({"terms":[]})),
            ] {
                let mut req = request(field, kind);
                if field == "kind" {
                    req["model"] = json!("stream");
                }
                set_path(&mut req, field, bad);
                refusal(s, "alice", "media.submit", req, "unsupported", Some(field)).await?;
            }
            require(records(s).await?.is_empty(), "unsupported work wrote usage")?;
            require(
                calls(r, "route.account_for").is_empty(),
                "unsupported work was admitted",
            )
        }
        "remote_submit_requires_decision" | "client_job_id_bounds" => {
            let mut req = request("invalid", "transcribe");
            let field = if name == "remote_submit_requires_decision" {
                req.as_object_mut().unwrap().remove("decision_id");
                "decision_id"
            } else {
                req["client_job_id"] = json!("é".repeat(129));
                "client_job_id"
            };
            refusal(
                s,
                "alice",
                "media.submit",
                req,
                "invalid_params",
                Some(field),
            )
            .await?;
            if name == "client_job_id_bounds" {
                let mut req = request("", "transcribe");
                refusal(
                    s,
                    "alice",
                    "media.submit",
                    req.clone(),
                    "invalid_params",
                    Some(field),
                )
                .await?;
                req["client_job_id"] = json!("é".repeat(128));
                script(s, "transcribe", vec![ScriptStep::Output(transcript("ok"))]).await?;
                submit(s, req).await?;
            }
            Ok(())
        }
        "input_too_large" => {
            let mut req = request("stated", "transcribe");
            req["input"]["audio"]["duration_ms"] = json!(2001);
            let v = refusal(
                s,
                "alice",
                "media.submit",
                req,
                "input_too_large",
                Some("input.audio"),
            )
            .await?;
            require(
                v["detail"]["limit"] == "max_duration_ms",
                "wrong exceeded limit",
            )?;
            let mut bytes_req = request("bytes", "transcribe");
            bytes_req["input"]["audio"] = media(&vec![0; 65_537], "audio/wav");
            let v = refusal(
                s,
                "alice",
                "media.submit",
                bytes_req,
                "input_too_large",
                Some("input.audio"),
            )
            .await?;
            require(
                v["detail"]["limit"] == "max_input_bytes" && v["detail"]["class"] == "permanent",
                "decoded byte limit ignored",
            )?;
            s.script_engine(
                "transcribe",
                Script::new(vec![ScriptStep::Output(transcript("too long"))])
                    .with_decoded_duration_ms(2001),
            )
            .await
            .map_err(err)?;
            let id = submit(s, request("decoded", "transcribe")).await?;
            let v = expect_status(s, &id, "failed").await?;
            require(
                v["failure"]["code"] == "input_too_large"
                    && v["failure"]["class"] == "permanent"
                    && v["failure"]["detail"]["limit"] == "max_duration_ms",
                "decoded duration ignored",
            )
        }
        "describe_matches_engine_declaration" | "describe_checker_codes" => {
            let v = call(s, "alice", "role.describe", json!({})).await?;
            let d: Describe = serde_json::from_value(v).map_err(err)?;
            check_describe(&d).map_err(err)?;
            let mut actual = d.engines.clone();
            let mut manifest = s.engine_declaration().await.map_err(err)?;
            let key = |e: &Engine| format!("{}:{}", e.kind.as_str(), e.model);
            actual.sort_by_key(&key);
            manifest.sort_by_key(&key);
            let mut expected = engines();
            expected.sort_by_key(key);
            require(
                actual == manifest && actual == expected,
                "describe/declaration facts differ",
            )?;
            if name == "describe_checker_codes" {
                crate::describe_checks::checker_codes()?;
            }
            Ok(())
        }
        "deadline_counts_queue_time" => {
            s.hold_engine("transcribe").await.map_err(err)?;
            let mut req = request("deadline", "transcribe");
            req["deadline_ms"] = json!(500);
            let id = submit(s, req).await?;
            expect_status(s, &id, "queued").await?;
            s.advance_clock(501).await.map_err(err)?;
            let v = expect_status(s, &id, "failed").await?;
            require(
                v["failure"]["code"] == "deadline_exceeded" && v["failure"]["class"] == "transient",
                "deadline did not include queue",
            )?;
            require(records(s).await?.is_empty(), "queued expiry wrote usage")
        }
        "refusal_class_decides_retry" => {
            for class in ["permanent", "future-class", "transient"] {
                script(
                    s,
                    "transcribe",
                    vec![
                        ScriptStep::Failure(failure("engine_crashed", class)),
                        ScriptStep::Output(transcript("retried")),
                    ],
                )
                .await?;
                let id = submit(s, request(class, "transcribe")).await?;
                let expected = if class == "transient" && s.capabilities().retries_transient {
                    "done"
                } else {
                    "failed"
                };
                let v = expect_status(s, &id, expected).await?;
                if class == "transient" && s.capabilities().retries_transient {
                    require(v["status"] == "done", "transient not retried")?;
                } else {
                    require(
                        v["status"] == "failed" && v["failure"]["class"] == class,
                        "retry ignored class",
                    )?;
                }
            }
            Ok(())
        }
        "empty_transcript_is_a_failure" => {
            for (id, fixture, text) in [
                ("silence", AudioFixture::Silence, ""),
                ("tone", AudioFixture::Tone, "scripted tone"),
            ] {
                s.script_engine(
                    "transcribe",
                    Script::new(vec![ScriptStep::Output(transcript(text))]).with_speech_check(true),
                )
                .await
                .map_err(err)?;
                let mut req = request(id, "transcribe");
                req["input"]["audio"] = media(&fixture.wav(), "audio/wav");
                let id = submit(s, req).await?;
                if text.is_empty() {
                    let v = expect_status(s, &id, "failed").await?;
                    require(
                        v["failure"]["code"] == "engine_empty_result"
                            && v["failure"]["class"] == "transient",
                        "empty transcript accepted",
                    )?;
                } else {
                    expect_status(s, &id, "done").await?;
                    require(
                        result(s, &id).await?["text"] == text,
                        "scripted tone text changed",
                    )?;
                }
            }
            Ok(())
        }
        "empty_output_is_a_failure" => {
            for (i, kind, output) in [
                (0, "synthesize", json!({"audio":media(b"","audio/wav")})),
                (1, "image", json!({"images":[]})),
                (2, "image", json!({"images":[media(b"","image/png")]})),
                (3, "video", json!({"video":media(b"","video/mp4")})),
            ] {
                script(s, kind, vec![ScriptStep::Output(output)]).await?;
                let id = submit(s, request(&format!("empty{i}"), kind)).await?;
                let v = expect_status(s, &id, "failed").await?;
                require(
                    v["failure"]["code"] == "engine_empty_result"
                        && v["failure"]["class"] == "transient",
                    "empty output accepted",
                )?;
            }
            Ok(())
        }
        _ => run_more(s, r, name).await,
    }
}

fn set_path(v: &mut Value, path: &str, value: Value) {
    if path == "input.references[0]" {
        v["input"]["references"] = json!([value]);
        return;
    }
    let mut parts = path.split('.').peekable();
    let mut cursor = v;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            cursor[part] = value;
            return;
        }
        cursor = &mut cursor[part];
    }
}

async fn run_more<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    name: &str,
) -> Result<(), String> {
    match name {
        "usage_absent_is_not_zero"
        | "usage_no_rounding"
        | "usage_quantity_bounds"
        | "usage_quantity_coefficient_bounds"
        | "usage_record_bounds" => {
            let measurements = if name == "usage_absent_is_not_zero" {
                vec![]
            } else {
                vec![
                    json!({"role":"output", "unit":"token", "quantity":"9007199254740993.00", "measure":"billed", "source":"vendor_reply"}),
                ]
            };
            let vendor_id = "識".repeat(257);
            s.script_engine(
                "transcribe",
                Script::new(vec![ScriptStep::Output(transcript("usage"))])
                    .with_measurements(measurements.clone())
                    .with_vendor_request_id(vendor_id.clone()),
            )
            .await
            .map_err(err)?;
            let id = submit(s, request("usage", "transcribe")).await?;
            expect_status(s, &id, "done").await?;
            let rs = wait_records(s, 1).await?;
            require(
                rs.len() == 1 && rs[0]["job_id"] == id && rs[0]["decision_id"] == "decision",
                "one attributed record required",
            )?;
            require(
                rs[0]["vendor_request_id"] == format!("sha256:{}", sha256(vendor_id.as_bytes())),
                "vendor id not hashed",
            )?;
            if name == "usage_absent_is_not_zero" {
                let ms = rs[0]["measurements"]
                    .as_array()
                    .ok_or("missing measurements")?;
                require(
                    ms.iter().all(|m| m["measure"] != "billed")
                        && ms.iter().any(|m| {
                            m["role"] == "input_audio"
                                && m["unit"] == "second"
                                && m["quantity"] == "1"
                                && m["measure"] == "observed"
                                && m["source"] == "provider_measured"
                        }),
                    "absent vendor usage must report measured duration, not billed zero",
                )
            } else {
                require(
                    rs[0]["measurements"] == json!(measurements),
                    "vendor decimals changed",
                )
            }
        }
        "usage_feed_redelivers_until_ack" => {
            completed(s, "feed").await?;
            let first = feed(s).await?;
            require(
                first["entries"].as_array().is_some_and(|v| v.len() == 1),
                "missing feed entry",
            )?;
            s.restart().await.map_err(err)?;
            require(
                feed(s).await? == first,
                "feed changed across redelivery/restart",
            )?;
            let seq = first["last_seq"].as_u64().ok_or("last_seq missing")?;
            call(s, "reader", "usage.ack", json!({"through_seq":seq})).await?;
            require(
                feed(s).await?["entries"] == json!([]),
                "acked entry redelivered",
            )
        }
        "usage_feed_admits_only_reader" => {
            completed(s, "private-feed").await?;
            for who in ["alice", "bob", "stranger"] {
                refusal(
                    s,
                    who,
                    "usage.read",
                    json!({"limit":1}),
                    "feed_not_admitted",
                    None,
                )
                .await?;
                refusal(
                    s,
                    who,
                    "usage.ack",
                    json!({"through_seq":1}),
                    "feed_not_admitted",
                    None,
                )
                .await?;
            }
            require(
                records(s).await?.len() == 1,
                "unauthorized ack discarded usage",
            )
        }
        "usage_ack_beyond_written_refused" => {
            refusal(
                s,
                "reader",
                "usage.ack",
                json!({"through_seq":1}),
                "invalid_params",
                Some("through_seq"),
            )
            .await?;
            completed(s, "ack").await?;
            refusal(
                s,
                "reader",
                "usage.ack",
                json!({"through_seq":2}),
                "invalid_params",
                Some("through_seq"),
            )
            .await?;
            require(records(s).await?.len() == 1, "refused ack lost record")
        }
        "usage_ack_answer" => {
            let v = call(s, "reader", "usage.ack", json!({"through_seq":0})).await?;
            require(v == json!({"acked_through_seq":0}), "empty ack answer")?;
            completed(s, "ack").await?;
            for seq in [1, 1, 0] {
                require(
                    call(s, "reader", "usage.ack", json!({"through_seq":seq})).await?
                        == json!({"acked_through_seq":1}),
                    "advancing/no-op ack answer",
                )?;
            }
            Ok(())
        }
        "usage_feed_oversize_entry_acks" => {
            let bytes = vec![b'x'; 262_145];
            s.seed_usage_entry(bytes.clone()).await.map_err(err)?;
            s.seed_usage_entry(b"not-json".to_vec())
                .await
                .map_err(err)?;
            let first = feed(s).await?;
            let entries = first["entries"].as_array().ok_or("missing entries")?;
            require(entries.len() == 2, "raw entries missing")?;
            let raw = &entries[0]["raw"];
            require(
                raw["reason"] == "oversize"
                    && raw["bytes_b64"] == ""
                    && raw["bytes"] == bytes.len()
                    && raw["sha256"] == sha256(&bytes),
                "oversize raw not bounded/preserved",
            )?;
            require(
                entries[1]["raw"]["bytes_b64"] == STANDARD.encode(b"not-json"),
                "undecodable raw lost",
            )?;
            require(feed(s).await? == first, "raw redelivery changed")?;
            call(
                s,
                "reader",
                "usage.ack",
                json!({"through_seq":first["last_seq"]}),
            )
            .await?;
            require(
                feed(s).await?["entries"] == json!([]),
                "raw entries unacknowledgeable",
            )
        }
        "failure_identical_across_surfaces" => {
            let mut f = failure("engine_crashed", "permanent");
            f["message"] = json!("界".repeat(1100));
            script(s, "transcribe", vec![ScriptStep::Failure(f)]).await?;
            let id = submit(s, request("failure", "transcribe")).await?;
            let st = expect_status(s, &id, "failed").await?;
            let rs = wait_records(s, 1).await?;
            require(
                rs.len() == 1 && st["failure"] == rs[0]["failure"],
                "failure differs across status/usage",
            )?;
            require(
                st["failure"]["message"] == "界".repeat(1024),
                "failure not cut at scalar boundary",
            )
        }
        "usage_record_per_attempt" => {
            script(
                s,
                "transcribe",
                vec![
                    ScriptStep::Pause,
                    ScriptStep::Output(transcript("cancelled")),
                ],
            )
            .await?;
            let id = submit(s, request("cancel-record", "transcribe")).await?;
            expect_status(s, &id, "running").await?;
            cancel(s, &id, "cancelled").await?;
            s.release_engine("transcribe").await.map_err(err)?;
            let rs = wait_records(s, 1).await?;
            require(
                rs.len() == 1 && rs[0]["status"] == "cancelled",
                "cancelled attempt record",
            )?;
            if s.capabilities().retries_transient {
                script(
                    s,
                    "transcribe",
                    vec![
                        ScriptStep::Failure(failure("engine_crashed", "transient")),
                        ScriptStep::Output(transcript("retry")),
                    ],
                )
                .await?;
                let id = submit(s, request("retry-record", "transcribe")).await?;
                expect_status(s, &id, "done").await?;
                let rs = wait_records(s, 3).await?;
                require(
                    rs.len() == 3
                        && rs[1]["status"] == "failed"
                        && rs[2]["status"] == "done"
                        && rs[1]["record_id"] != rs[2]["record_id"],
                    "retry attempt records",
                )?;
                require(
                    rs[1]["decision_id"] == "decision" && rs[2].get("decision_id").is_none(),
                    "later attempt carried decision",
                )?;
            }
            Ok(())
        }
        "restart_keeps_terminal_jobs" => {
            let done = completed(s, "done").await?;
            let output = result(s, &done).await?;
            script(s, "transcribe", vec![ScriptStep::Pause]).await?;
            let cancelled = submit(s, request("cancelled", "transcribe")).await?;
            expect_status(s, &cancelled, "running").await?;
            cancel(s, &cancelled, "cancelled").await?;
            s.restart().await.map_err(err)?;
            expect_status(s, &done, "done").await?;
            expect_status(s, &cancelled, "cancelled").await?;
            require(result(s, &done).await? == output, "restart lost output")?;
            require(
                submit(s, request("done", "transcribe")).await? == done,
                "restart lost replay",
            )?;
            refusal(
                s,
                "alice",
                "media.result",
                json!({"job_id":cancelled}),
                "job_not_done",
                None,
            )
            .await?;
            Ok(())
        }
        "cancel_then_restart_records_cancelled" | "running_restart_records_failure" => {
            script(s, "transcribe", vec![ScriptStep::Pause]).await?;
            let id = submit(s, request("restart", "transcribe")).await?;
            expect_status(s, &id, "running").await?;
            let cancelled = name == "cancel_then_restart_records_cancelled";
            if cancelled {
                cancel(s, &id, "cancelled").await?;
            }
            s.restart().await.map_err(err)?;
            let rs = wait_records(s, 1).await?;
            require(
                rs.iter().filter(|v| v["job_id"] == id).count() >= 1,
                "restart lost attempt record",
            )?;
            require(
                rs[0]["status"] == if cancelled { "cancelled" } else { "failed" },
                "restart attempt status",
            )?;
            if cancelled {
                expect_status(s, &id, "cancelled").await?;
                require(rs.len() == 1, "cancel/restart double recorded")?;
            } else {
                require(
                    rs[0]["failure"]["code"] == "module_restarted",
                    "restart failure code",
                )?;
            }
            require(
                events(r, "run_finished")
                    .iter()
                    .filter(|v| v["admission_id"] == format!("{id}/1"))
                    .count()
                    == 1,
                "restart must release once",
            )
        }
        "cancel_after_done_reports_done" => {
            let id = completed(s, "done").await?;
            cancel(s, &id, "done").await
        }
        "cancel_never_changes_terminal_status" => {
            script(
                s,
                "transcribe",
                vec![ScriptStep::Pause, ScriptStep::Output(transcript("late"))],
            )
            .await?;
            let mut req = request("cancel", "transcribe");
            req["deadline_ms"] = json!(500);
            let id = submit(s, req).await?;
            expect_status(s, &id, "running").await?;
            cancel(s, &id, "cancelled").await?;
            s.release_engine("transcribe").await.map_err(err)?;
            s.advance_clock(501).await.map_err(err)?;
            s.restart().await.map_err(err)?;
            expect_status(s, &id, "cancelled").await?;
            cancel(s, &id, "cancelled").await?;
            refusal(
                s,
                "alice",
                "media.result",
                json!({"job_id":id}),
                "job_not_done",
                None,
            )
            .await?;
            Ok(())
        }
        "foreign_job_is_unknown" => {
            let id = completed(s, "private").await?;
            for method in ["media.status", "media.result", "media.cancel"] {
                let foreign =
                    refusal(s, "bob", method, json!({"job_id":id}), "unknown_job", None).await?;
                let unknown = refusal(
                    s,
                    "bob",
                    method,
                    json!({"job_id":"nonexistent"}),
                    "unknown_job",
                    None,
                )
                .await?;
                require(foreign == unknown, "foreign id leaks existence")?;
            }
            Ok(())
        }
        _ => run_objects_and_routing(s, r, name).await,
    }
}

async fn upload<S: MediaSubject>(s: &S, bytes: &[u8]) -> Result<String, String> {
    let v = call(
        s,
        "alice",
        "media.upload_begin",
        json!({"mime":"audio/wav","bytes":bytes.len(),"sha256":sha256(bytes)}),
    )
    .await?;
    Ok(v["upload_id"].as_str().ok_or("missing upload_id")?.into())
}
async fn append<S: MediaSubject>(
    s: &S,
    id: &str,
    offset: usize,
    bytes: &[u8],
) -> Result<Value, String> {
    call(
        s,
        "alice",
        "media.upload_append",
        json!({"upload_id":id,"offset":offset,"data":STANDARD.encode(bytes)}),
    )
    .await
}
async fn commit<S: MediaSubject>(s: &S, id: &str) -> Result<Value, String> {
    call(s, "alice", "media.upload_commit", json!({"upload_id":id})).await
}
async fn read_all<S: MediaSubject>(s: &S, object: &Value) -> Result<Vec<u8>, String> {
    let size = object["bytes"].as_u64().ok_or("object size missing")?;
    let mut bytes = Vec::new();
    while bytes.len() < size as usize {
        let v = call(
            s,
            "alice",
            "media.read",
            json!({"object_id":object["object_id"],"offset":bytes.len(),"length":65536}),
        )
        .await?;
        let chunk = STANDARD
            .decode(v["data"].as_str().ok_or("read missing data")?)
            .map_err(err)?;
        require(!chunk.is_empty(), "read stalled")?;
        bytes.extend(chunk);
    }
    require(
        bytes.len() == size as usize && object["sha256"] == sha256(&bytes),
        "whole-object checksum mismatch",
    )?;
    Ok(bytes)
}

async fn run_objects_and_routing<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    name: &str,
) -> Result<(), String> {
    match name {
        "upload_resumes_at_received" => {
            let bytes = AudioFixture::Noise.wav();
            let id = upload(s, &bytes).await?;
            require(
                append(s, &id, 0, &bytes[..100]).await?["received"] == 100,
                "first append",
            )?;
            let v = refusal(
                s,
                "alice",
                "media.upload_append",
                json!({"upload_id":id,"offset":0,"data":STANDARD.encode(&bytes[..10])}),
                "upload_offset_mismatch",
                None,
            )
            .await?;
            require(
                v["detail"]["received"] == 100,
                "offset mismatch omitted resume offset",
            )?;
            let st = call(s, "alice", "media.upload_status", json!({"upload_id":id})).await?;
            require(
                st["state"] == "open" && st["received"] == 100,
                "refusal changed upload",
            )?;
            append(s, &id, 100, &bytes[100..]).await?;
            require(
                commit(s, &id).await?["sha256"] == sha256(&bytes),
                "resumed checksum",
            )
        }
        "upload_gone_after_restart" => {
            let bytes = AudioFixture::Tone.wav();
            let id = upload(s, &bytes).await?;
            append(s, &id, 0, &bytes[..100]).await?;
            s.restart().await.map_err(err)?;
            require(
                call(s, "alice", "media.upload_status", json!({"upload_id":id})).await?["state"]
                    == "gone",
                "open upload survived restart",
            )?;
            refusal(
                s,
                "alice",
                "media.upload_commit",
                json!({"upload_id":id}),
                "upload_gone",
                None,
            )
            .await?;
            refusal(
                s,
                "alice",
                "media.upload_append",
                json!({"upload_id":id,"offset":100,"data":"AA=="}),
                "upload_gone",
                None,
            )
            .await?;
            Ok(())
        }
        "committed_upload_is_object" | "objects_retained_for_one_hour" => {
            let bytes = AudioFixture::Tone.wav();
            let id = upload(s, &bytes).await?;
            append(s, &id, 0, &bytes).await?;
            let c = commit(s, &id).await?;
            require(
                c["state"] == "committed"
                    && c["bytes"] == bytes.len()
                    && c["mime"] == "audio/wav"
                    && c["sha256"] == sha256(&bytes),
                "commit facts",
            )?;
            if name == "objects_retained_for_one_hour" {
                // Expiry may start at 3,600 seconds, never before that instant.
                s.advance_clock(3_599_999).await.map_err(err)?;
            }
            let object = json!({"mime":"audio/wav","bytes":bytes.len(),"sha256":sha256(&bytes),"object_id":id});
            require(read_all(s, &object).await? == bytes, "upload read bytes")?;
            script(
                s,
                "transcribe",
                vec![ScriptStep::Output(transcript("object input"))],
            )
            .await?;
            let mut req = request("object", "transcribe");
            req["input"]["audio"] = object.clone();
            let job = submit(s, req.clone()).await?;
            expect_status(s, &job, "done").await?;
            req["client_job_id"] = json!("mismatch");
            req["input"]["audio"]["mime"] = json!("audio/L16;rate=16000;channels=1");
            refusal(
                s,
                "alice",
                "media.submit",
                req,
                "object_mismatch",
                Some("input.audio"),
            )
            .await?;
            refusal(
                s,
                "bob",
                "media.read",
                json!({"object_id":id,"offset":0,"length":1}),
                "unknown_object",
                None,
            )
            .await?;
            Ok(())
        }
        "chunked_result_checksum" => {
            let bytes: Vec<u8> = (0..2_200_000).map(|i| (i % 251) as u8).collect();
            script(
                s,
                "synthesize",
                vec![ScriptStep::Output(
                    json!({"audio":media(&bytes,"audio/wav")}),
                )],
            )
            .await?;
            let id = submit(s, request("chunked", "synthesize")).await?;
            let output = result(s, &id).await?;
            require(
                output["audio"].get("object_id").is_some() && output["audio"].get("data").is_none(),
                "large audio not stored as object",
            )?;
            require(
                read_all(s, &output["audio"]).await? == bytes,
                "chunk bytes differ",
            )
        }
        "oversize_output_is_object" => {
            let output = transcript(&"x".repeat(3_145_729));
            script(s, "transcribe", vec![ScriptStep::Output(output.clone())]).await?;
            let id = submit(s, request("oversize", "transcribe")).await?;
            let answer = result(s, &id).await?;
            require(
                answer["output_object"]["mime"] == "application/json",
                "oversize output not JSON object",
            )?;
            let bytes = read_all(s, &answer["output_object"]).await?;
            require(
                serde_json::from_slice::<Value>(&bytes).map_err(err)? == output,
                "stored output changed",
            )
        }
        "routing_admission_per_attempt" => routing_admissions(s, r).await,
        "renewal_lapse_ends_work" => {
            r.refuse_renewals(true);
            for queued in [true, false] {
                if queued {
                    s.hold_engine("transcribe").await.map_err(err)?;
                } else {
                    s.release_engine("transcribe").await.map_err(err)?;
                    script(s, "transcribe", vec![ScriptStep::Pause]).await?;
                }
                let id = submit(
                    s,
                    request(if queued { "queued" } else { "running" }, "transcribe"),
                )
                .await?;
                s.advance_clock(1001).await.map_err(err)?;
                let v = expect_status(s, &id, "failed").await?;
                require(
                    v["failure"]["code"] == "routing_refused"
                        && v["failure"]["detail"]["routing_code"] == "account_hold",
                    "lapse did not end work",
                )?;
                let rs = wait_records(s, usize::from(!queued)).await?;
                require(rs.len() == usize::from(!queued), "lapsed work usage count")?;
                require(
                    events(r, "run_finished")
                        .iter()
                        .filter(|v| v["admission_id"] == format!("{id}/1"))
                        .count()
                        == 1,
                    "lapse released other than once",
                )?;
            }
            Ok(())
        }
        _ => Err("unimplemented registered case".into()),
    }
}

async fn routing_admissions<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
) -> Result<(), String> {
    script(s, "local", vec![ScriptStep::Output(transcript("local"))]).await?;
    let mut local = request("local", "transcribe");
    local["model"] = json!("local");
    submit(s, local).await?;
    require(r.calls().is_empty(), "local work contacted routing")?;
    s.hold_engine("transcribe").await.map_err(err)?;
    let id = submit(s, request("queued", "transcribe")).await?;
    let admits = calls(r, "route.account_for");
    require(
        admits.len() == 1
            && admits[0]["decision_id"] == "decision"
            && admits[0]["hold_wait_max_s"] == 0
            && admits[0]["conversation_key"] == id
            && admits[0]["admission_id"] == format!("{id}/1"),
        "acceptance admission",
    )?;
    require(
        events(r, "admission_started").is_empty(),
        "held engine started",
    )?;
    s.advance_clock(800).await.map_err(err)?;
    let renews = calls(r, "route.account_for");
    require(
        renews.len() >= 2
            && renews[1]["mode"] == "renew"
            && renews[1]["admission_id"] == format!("{id}/1/renew/1")
            && renews[1].get("decision_id").is_none()
            && renews[1]["expected_pin"].is_object(),
        "queued renewal",
    )?;
    script(s, "transcribe", vec![ScriptStep::Pause]).await?;
    s.release_engine("transcribe").await.map_err(err)?;
    expect_status(s, &id, "running").await?;
    s.advance_clock(800).await.map_err(err)?;
    let all = calls(r, "route.account_for");
    require(
        all.len() >= 3
            && all[2]["admission_id"] == format!("{id}/1/renew/2")
            && all[2]["expected_pin"] != all[1]["expected_pin"],
        "running renewal did not use latest pin",
    )?;
    cancel(s, &id, "cancelled").await?;
    s.release_engine("transcribe").await.map_err(err)?;
    let before = r.calls().len();
    s.advance_clock(1500).await.map_err(err)?;
    require(r.calls().len() == before, "renewed after release")?;
    for (event, suffix) in [
        ("admission_started", "started"),
        ("run_finished", "finished"),
    ] {
        let ev = events(r, event);
        require(
            ev.len() == 1
                && ev[0]["admission_id"] == format!("{id}/1")
                && ev[0]["event_id"] == format!("{id}/1/{suffix}"),
            "observe original admission exactly once",
        )?;
    }
    if s.capabilities().retries_transient {
        script(
            s,
            "transcribe",
            vec![
                ScriptStep::Failure(failure("engine_crashed", "transient")),
                ScriptStep::Output(transcript("retry")),
            ],
        )
        .await?;
        let job = submit(s, request("retry", "transcribe")).await?;
        let all = calls(r, "route.account_for");
        let later = all
            .iter()
            .find(|v| v["admission_id"] == format!("{job}/2"))
            .ok_or("retry not newly admitted")?;
        require(
            later.get("decision_id").is_none() && later["hold_wait_max_s"] == 0,
            "retry reused decision",
        )?;
    }
    for ending in ["cancel", "deadline", "restart"] {
        if ending == "restart" && s.capabilities().jobs_survive_restart {
            continue;
        }
        s.hold_engine("transcribe").await.map_err(err)?;
        let mut req = request(ending, "transcribe");
        req["deadline_ms"] = json!(100);
        let job = submit(s, req).await?;
        match ending {
            "cancel" => cancel(s, &job, "cancelled").await?,
            "deadline" => s.advance_clock(101).await.map_err(err)?,
            _ => s.restart().await.map_err(err)?,
        }
        let before = r.calls().len();
        s.advance_clock(1200).await.map_err(err)?;
        require(r.calls().len() == before, "queued terminal work renewed")?;
        require(
            events(r, "run_finished")
                .iter()
                .filter(|v| v["admission_id"] == format!("{job}/1"))
                .count()
                == 1,
            "queued terminal release count",
        )?;
        require(
            !records(s).await?.iter().any(|v| v["job_id"] == job),
            "unstarted work wrote usage",
        )?;
    }
    Ok(())
}
