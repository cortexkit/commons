use super::*;
use crate::subject::StreamObservation;

pub const CASES: &[&str] = &[
    "stream_reply_envelope",
    "realtime_watch",
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

fn first(id: &str) -> Value {
    json!({"stream_id":id,"model":"stream","decision_id":"decision","format":"audio/L16;rate=16000;channels=1"})
}
fn segment(n: u64, text: &str) -> Value {
    json!({"segment":n,"text":text,"words":[]})
}
async fn open<S: MediaSubject>(s: &S, id: &str) -> Result<(), String> {
    s.open_stream("alice", first(id)).await.map_err(err)
}
async fn audio<S: MediaSubject>(s: &S, id: &str, seq: u64, bytes: usize) -> Result<(), String> {
    s.send_stream_frame(
        "alice",
        id,
        json!({"seq":seq,"data":STANDARD.encode(vec![0;bytes])}),
    )
    .await
    .map_err(err)
}
async fn observe<S: MediaSubject>(
    s: &S,
    id: &str,
    terminal: bool,
) -> Result<StreamObservation, String> {
    let started = Instant::now();
    let mut all = Vec::new();
    loop {
        let remaining = STATUS_POLL_TIMEOUT.saturating_sub(started.elapsed());
        let mut v = tokio::time::timeout(remaining, s.poll_stream("alice", id))
            .await
            .map_err(|_| "stream poll exceeded ceiling".to_string())?
            .map_err(err)?;
        all.append(&mut v.events);
        if !terminal || v.closed {
            v.events = all;
            return Ok(v);
        }
        require(
            started.elapsed() < STATUS_POLL_TIMEOUT,
            "stream never closed",
        )?;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
async fn finish<S: MediaSubject>(s: &S, id: &str, seq: u64) -> Result<StreamObservation, String> {
    s.send_stream_frame("alice", id, json!({"seq":seq,"end":true}))
        .await
        .map_err(err)?;
    observe(s, id, true).await
}
async fn open_refused<S: MediaSubject>(
    s: &S,
    params: Value,
    code: &str,
    field: Option<&str>,
) -> Result<Value, String> {
    let e = s
        .open_stream("alice", params)
        .await
        .err()
        .ok_or("first frame accepted")?;
    let v = e.refusal.ok_or("first frame was not ERROR")?;
    require(
        v["code"] == code
            && v["detail"]["class"]
                == if matches!(code, "routing_refused" | "realtime_lane_full") {
                    "transient"
                } else {
                    "permanent"
                },
        &format!("first-frame refusal: {v}"),
    )?;
    if let Some(f) = field {
        require(v["detail"]["field"] == f, "first-frame field")?;
    }
    Ok(v)
}
fn balanced(r: &RecordingRoutingPeer, started: usize, released: usize) -> Result<(), String> {
    let admissions = calls(r, "route.account_for")
        .into_iter()
        .filter(|v| v["mode"] != "renew")
        .collect::<Vec<_>>();
    let starts = events(r, "admission_started");
    let ends = events(r, "run_finished");
    require(
        starts.len() == started && ends.len() == released,
        &format!("observations: starts {starts:?}, finishes {ends:?}"),
    )?;
    let ids = ends
        .iter()
        .map(|v| v["admission_id"].clone())
        .collect::<Vec<_>>();
    let start_ids: Vec<_> = starts.iter().map(|v| v["admission_id"].clone()).collect();
    for (i, id) in start_ids.iter().enumerate() {
        require(
            !start_ids[..i].contains(id) && admissions.iter().any(|a| a["admission_id"] == *id),
            "started observation is duplicated or names another admission",
        )?;
        if released >= started {
            require(ids.contains(id), "started admission was not released")?;
        }
    }
    for (i, id) in ids.iter().enumerate() {
        require(
            !ids[..i].contains(id) && admissions.iter().any(|a| a["admission_id"] == *id),
            "release is duplicated or names another admission",
        )?;
    }
    Ok(())
}
async fn wait_started(r: &RecordingRoutingPeer, count: usize) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if events(r, "admission_started").len() >= count {
            return Ok(());
        }
        require(
            start.elapsed() < STATUS_POLL_TIMEOUT,
            "engine never sent admission_started",
        )?;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
async fn answer<S: MediaSubject>(s: &S, observation: &StreamObservation) -> Result<Value, String> {
    let v = decode_reply(observation.answer.clone().ok_or("stream missing answer")?)?;
    if let Some(object) = v.get("output_object") {
        require(object["mime"] == "application/json", "stream output MIME")?;
        let bytes = read_all(s, object).await?;
        serde_json::from_slice(&bytes).map_err(err)
    } else {
        Ok(v)
    }
}

pub(super) async fn run<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    vendor: &RecordingVendor,
    name: &str,
) -> Result<(), String> {
    let mut es = engines();
    for model in ["realtime-a", "realtime-b"] {
        es.push(serde_json::from_value(json!({"kind":"realtime","model":model,"provider":"fixture","local":false,"formats":["audio/wav"],"latency_class":"realtime"})).map_err(err)?);
    }
    let mut e = serde_json::to_value(&es[0]).map_err(err)?;
    e["bias_limit"] = json!({"unit":"phrase","max":3});
    es[0] = serde_json::from_value(e).map_err(err)?;
    s.configure_engines(es.clone()).await.map_err(err)?;
    if name == "stream_realtime_admission_lifecycle" {
        return admission_lifecycle(s, r, vendor).await;
    }
    if name == "stream_realtime_renewal_lapse" {
        return renewal_lapse(s, r, vendor).await;
    }
    if name.starts_with("stream_") {
        return streams(s, r, name, es).await;
    }
    if name == "bias_trim_phrase_limit" {
        let bias = json!({"terms":[{"text":"one","weight":1.0},{"text":"two","weight":0.8},{"text":"three","weight":0.6},{"text":"four","weight":0.4},{"text":"five","weight":0.2}],"prompt":"never forwarded"});
        script(
            s,
            "transcribe",
            vec![ScriptStep::Output(transcript("words"))],
        )
        .await?;
        let mut req = request("biased", "transcribe");
        req["input"]["bias"] = bias.clone();
        let id = submit(s, req).await?;
        let out = result(s, &id).await?;
        let inputs = s.engine_inputs("transcribe").await.map_err(err)?;
        require(
            inputs.len() == 1
                && inputs[0]["bias"] == json!({"terms":bias["terms"].as_array().unwrap()[..3]}),
            "engine_inputs did not forward exactly three ranked terms without prompt",
        )?;
        return require(
            out["bias_applied"] == json!({"terms_used":3,"terms_dropped":2,"prompt_used":false}),
            "bias_applied is not exact",
        );
    }
    if name.starts_with("context_pack_") {
        return packs(s, name).await;
    }
    realtime(s, r, vendor, name).await
}

async fn streams<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    name: &str,
    mut es: Vec<Engine>,
) -> Result<(), String> {
    match name {
        "stream_reply_envelope" => {
            script(s, "stream", vec![ScriptStep::Final(segment(0, "words"))]).await?;
            open(s, "envelope").await?;
            audio(s, "envelope", 0, 32).await?;
            let observation = finish(s, "envelope", 1).await?;
            require(
                answer(s, &observation).await?
                    == json!({"segments":[segment(0,"words")],"no_speech":false}),
                "stream answer changed",
            )
        }
        "stream_input_limits" => {
            for (i, limit) in ["max_input_bytes", "max_duration_ms"]
                .into_iter()
                .enumerate()
            {
                let e = es.iter_mut().find(|e| e.model == "stream").unwrap();
                e.max_input_bytes = Some(if i == 0 { 64 } else { 4096 });
                e.max_duration_ms = Some(if i == 0 { 2000 } else { 2 });
                s.configure_engines(es.clone()).await.map_err(err)?;
                script(
                    s,
                    "stream",
                    vec![
                        ScriptStep::Final(segment(0, "kept")),
                        ScriptStep::Pause,
                        ScriptStep::Pause,
                        ScriptStep::Final(segment(1, "must not run")),
                    ],
                )
                .await?;
                let id = format!("limit-{i}");
                open(s, &id).await?;
                audio(s, &id, 0, 32).await?;
                audio(s, &id, 1, 32).await?;
                let before = observe(s, &id, false).await?;
                require(
                    !before.closed && before.answer.is_none(),
                    "exact input limit was rejected",
                )?;
                audio(s, &id, 2, if i == 0 { 1 } else { 32 }).await?;
                let v = answer(s, &observe(s, &id, true).await?).await?;
                require(
                    v["failure"]["code"] == "input_too_large"
                        && v["failure"]["class"] == "permanent"
                        && v["failure"]["detail"] == json!({"field":"audio","limit":limit})
                        && v.get("no_speech").is_none(),
                    "limit failure shape",
                )?;
                require(
                    v["segments"] == json!([segment(0, "kept")]),
                    "limit transcribed crossing frame or discarded prior final",
                )?;
                let rs = wait_records(s, i + 1).await?;
                require(
                    rs.len() == i + 1
                        && rs[i]["stream_id"] == id
                        && rs[i]["status"] == "failed"
                        && rs[i]["failure"] == v["failure"],
                    "stream limit record",
                )?;
            }
            balanced(r, 2, 2)
        }
        "stream_id_reuse_refused" => {
            script(s, "stream", vec![ScriptStep::Final(segment(0, "words"))]).await?;
            open(s, "reused").await?;
            audio(s, "reused", 0, 32).await?;
            finish(s, "reused", 1).await?;
            open_refused(s, first("reused"), "stream_id_reuse", None).await?;
            s.restart().await.map_err(err)?;
            open_refused(s, first("reused"), "stream_id_reuse", None).await?;
            balanced(r, 1, 1)
        }
        "stream_first_frame_refused" => {
            let invalid = s
                .refuse_next_stream("engine_crashed")
                .await
                .err()
                .ok_or("refuse_next_stream accepted an engine failure")?;
            require(
                invalid.refusal.is_none() && invalid.message.contains("refuse_next_stream"),
                "invalid injected refusal did not name hook",
            )?;
            // The adapter injects a lane refusal; the case does not assume a lane capacity.
            for (i, code) in ["routing_refused", "realtime_lane_full"]
                .into_iter()
                .enumerate()
            {
                if i == 0 {
                    r.refuse_next(json!({"code":"account_hold","detail":{"until_ms":1000}}));
                } else {
                    s.refuse_next_stream(code).await.map_err(err)?;
                }
                let id = format!("refused-{i}");
                let refused = open_refused(s, first(&id), code, None).await?;
                if i == 0 {
                    require(
                        refused["detail"]["routing_code"] == "account_hold"
                            && refused["detail"]["retry_after_ms"] == 1000,
                        "routing refusal lost code or delay",
                    )?;
                }
                require(
                    records(s).await?.len() == i,
                    "refused first frame wrote usage",
                )?;
                balanced(r, i, i)?;
                script(s, "stream", vec![ScriptStep::Final(segment(0, "accepted"))]).await?;
                open(s, &id).await?;
                audio(s, &id, 0, 32).await?;
                let v = answer(s, &finish(s, &id, 1).await?).await?;
                require(
                    v["segments"] == json!([segment(0, "accepted")]),
                    "refused stream id was remembered",
                )?;
            }
            balanced(r, 2, 2)
        }
        "stream_remote_requires_decision" | "stream_id_bounds" => {
            let mut v = first("valid");
            let field = if name == "stream_id_bounds" {
                v["stream_id"] = json!("é".repeat(129));
                "stream_id"
            } else {
                v.as_object_mut().unwrap().remove("decision_id");
                "decision_id"
            };
            open_refused(s, v, "invalid_params", Some(field)).await?;
            require(
                r.calls().is_empty() && records(s).await?.is_empty(),
                "invalid first frame admitted/wrote",
            )
        }
        "stream_restart" => {
            for started in [true, false] {
                if !started {
                    s.hold_engine("stream").await.map_err(err)?;
                }
                script(s, "stream", vec![ScriptStep::Pause]).await?;
                let id = if started { "started" } else { "queued" };
                open(s, id).await?;
                audio(s, id, 0, 32).await?;
                if started {
                    wait_started(r, 1).await?;
                }
                balanced(r, 1, usize::from(!started))?;
                s.restart().await.map_err(err)?;
                let v = observe(s, id, true).await?;
                require(
                    v.closed && v.answer.is_none(),
                    "restarted stream returned an answer",
                )?;
                let rs = records(s).await?;
                require(
                    rs.len() == 1
                        && rs[0]["stream_id"] == "started"
                        && rs[0]["status"] == "failed"
                        && rs[0]["failure"]["code"] == "module_restarted",
                    "restart usage must record only started stream",
                )?;
                open_refused(s, first(id), "stream_id_reuse", None).await?;
            }
            s.restart().await.map_err(err)?;
            balanced(r, 1, 2)
        }
        "stream_partials_replace_finals_append" => {
            let steps = vec![
                ScriptStep::Partial(json!({"segment":0,"text":"old"})),
                ScriptStep::Pause,
                ScriptStep::Partial(json!({"segment":0,"text":"new"})),
                ScriptStep::Final(segment(0, "first")),
                ScriptStep::Pause,
                ScriptStep::Partial(json!({"segment":1,"text":"draft"})),
                ScriptStep::Partial(json!({"segment":1,"text":"replacement"})),
                ScriptStep::Final(segment(1, "second")),
            ];
            script(s, "stream", steps).await?;
            open(s, "partials").await?;
            let mut seen = Vec::new();
            for seq in 0..3 {
                audio(s, "partials", seq, 32).await?;
                seen.extend(observe(s, "partials", false).await?.events);
            }
            let o = finish(s, "partials", 3).await?;
            seen.extend(o.events.clone());
            let expected = json!([
                {"type":"partial","segment":0,"text":"old"},{"type":"partial","segment":0,"text":"new"},
                {"type":"final","segment":0,"text":"first","words":[]},
                {"type":"partial","segment":1,"text":"draft"},{"type":"partial","segment":1,"text":"replacement"},
                {"type":"final","segment":1,"text":"second","words":[]}]);
            require(
                json!(seen) == expected,
                "stream events lost/reordered partial replacements or finals",
            )?;
            require(
                answer(s, &o).await?
                    == json!({"segments":[segment(0,"first"),segment(1,"second")],"no_speech":false}),
                "stream answer contains partials or wrong finals",
            )
        }
        "stream_oversize_final_splits" => oversize(s).await,
        "stream_empty_result_needs_speech_check" => empty_streams(s).await,
        _ => Err("unimplemented stream case".into()),
    }
}

async fn oversize<S: MediaSubject>(s: &S) -> Result<(), String> {
    let words: Vec<Value> = (0..3200)
        .map(|i| json!({"text":format!("{i:04}{}", "é".repeat(512)),"start_ms":i,"end_ms":i+1}))
        .collect();
    let text = words
        .iter()
        .map(|w| w["text"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    script(
        s,
        "stream",
        vec![
            ScriptStep::Partial(json!({"segment":0,"text":text})),
            ScriptStep::Final(json!({"segment":0,"text":text,"words":words})),
        ],
    )
    .await?;
    open(s, "large").await?;
    audio(s, "large", 0, 32).await?;
    let o = finish(s, "large", 1).await?;
    require(
        o.events.len() > 1
            && o.events
                .iter()
                .all(|v| v["type"] == "final" && serde_json::to_vec(v).unwrap().len() <= 3_145_728),
        "oversize partial emitted or final not split to message budget",
    )?;
    let mut all_words = Vec::new();
    let mut texts = Vec::new();
    for (i, e) in o.events.iter().enumerate() {
        require(
            e["segment"] == i && e["words"].as_array().is_some_and(|a| !a.is_empty()),
            "split segments not consecutive/nonempty",
        )?;
        let ws = e["words"].as_array().unwrap();
        require(
            e["text"]
                == ws
                    .iter()
                    .map(|w| w["text"].as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(" "),
            "split crossed a word boundary",
        )?;
        all_words.extend(ws.clone());
        texts.push(e["text"].as_str().unwrap());
    }
    require(
        all_words == words && texts.join(" ") == text,
        "split lost/repeated text or word timing",
    )?;
    require(
        decode_reply(o.answer.clone().ok_or("stream missing answer")?)?
            .get("output_object")
            .is_some(),
        "oversize stream answer not stored as object",
    )?;
    let out = answer(s, &o).await?;
    let expected: Vec<_> = o.events.iter().map(segment_with_words).collect();
    require(
        out == json!({"segments":expected,"no_speech":false}),
        "stored stream output differs from emitted finals",
    )
}
fn segment_with_words(e: &Value) -> Value {
    json!({"segment":e["segment"],"text":e["text"],"words":e["words"]})
}
async fn empty_streams<S: MediaSubject>(s: &S) -> Result<(), String> {
    let variants = [
        vec![],
        vec![ScriptStep::Final(segment(0, ""))],
        vec![
            ScriptStep::Final(segment(0, " \t")),
            ScriptStep::Final(segment(1, "\n")),
        ],
    ];
    let mut count = 0;
    for (i, steps) in variants.into_iter().enumerate() {
        for verdict in [None, Some(true), Some(false)] {
            let mut sc = Script::new(steps.clone());
            if let Some(v) = verdict {
                sc = sc.with_speech_check(v);
            }
            s.script_engine("stream", sc).await.map_err(err)?;
            let id = format!("empty-{count}");
            open(s, &id).await?;
            // Tone, not digital silence: only the scripted speech verdict authorizes no_speech.
            s.send_stream_frame(
                "alice",
                &id,
                json!({"seq":0,"data":STANDARD.encode(&AudioFixture::Tone.wav()[44..])}),
            )
            .await
            .map_err(err)?;
            let o = finish(s, &id, 1).await?;
            let v = answer(s, &o).await?;
            let silent = s.capabilities().has_speech_check && verdict == Some(false);
            require(
                v["segments"] == json!([]),
                "empty finals leaked into answer",
            )?;
            if silent {
                require(
                    v == json!({"segments":[],"no_speech":true}),
                    "verified silence not reported",
                )?;
            } else {
                require(
                    v["failure"]["code"] == "engine_empty_result" && v.get("no_speech").is_none(),
                    &format!("empty variant {i} accepted without verified silence"),
                )?;
            }
            count += 1;
            let rs = wait_records(s, count).await?;
            require(
                rs.len() == count
                    && rs[count - 1]["status"] == if silent { "done" } else { "failed" },
                "empty stream record status",
            )?;
        }
    }
    // Output scripts can report contradictory engine-level terminal metadata.
    s.script_engine(
        "stream",
        Script::new(vec![
            ScriptStep::Pause,
            ScriptStep::Output(json!({"segments":[segment(0,"words")],"no_speech":true})),
        ])
        .with_speech_check(false),
    )
    .await
    .map_err(err)?;
    open(s, "contradiction").await?;
    audio(s, "contradiction", 0, 32).await?;
    let v = answer(s, &finish(s, "contradiction", 1).await?).await?;
    require(
        v["failure"]["code"] == "engine_empty_result" && v.get("no_speech").is_none(),
        "nonempty no_speech answer accepted",
    )
}
async fn packs<S: MediaSubject>(s: &S, name: &str) -> Result<(), String> {
    for purpose in ["bias", "front"] {
        let pack = if purpose == "bias" {
            json!({"version":"bias-v1","terms":[{"text":"name","weight":1.0}],"prompt":"context","built_at_ms":123,"privacy":{}})
        } else {
            json!({"version":"front-v1","persona":"helpful","memories":[],"board":{"card":"one"},"recent":["turn"],"built_at_ms":456,"privacy":{}})
        };
        s.configure_pack("agent", purpose, pack.clone())
            .await
            .map_err(err)?;
        for principal in ["alice", "bob"] {
            let req = json!({"agent_id":"agent","purpose":purpose});
            require(
                call(s, principal, "context.for_agent", req.clone()).await? == pack,
                "owner/reader pack differs from configured content",
            )?;
            if name == "context_pack_if_version" {
                let mut same = req.clone();
                same["if_version"] = pack["version"].clone();
                require(
                    call(s, principal, "context.for_agent", same).await?
                        == json!({"unchanged":true,"version":pack["version"]}),
                    "matching if_version not unchanged",
                )?;
                let mut different = req;
                different["if_version"] = json!(if purpose == "bias" {
                    "front-v1"
                } else {
                    "bias-v1"
                });
                require(
                    call(s, principal, "context.for_agent", different).await? == pack,
                    "version comparison crossed purposes",
                )?;
            }
        }
        if name == "context_pack_admission" {
            refusal(
                s,
                "mallory",
                "context.for_agent",
                json!({"agent_id":"agent","purpose":purpose}),
                "unknown_agent",
                None,
            )
            .await?;
            refusal(
                s,
                "alice",
                "context.for_agent",
                json!({"agent_id":"missing","purpose":purpose}),
                "unknown_agent",
                None,
            )
            .await?;
        }
    }
    Ok(())
}
fn token_request(id: &str, model: &str) -> Value {
    json!({"call_id":id,"agent_id":"agent","purpose":"dictation","model":model,"decision_id":"decision"})
}
async fn token<S: MediaSubject>(
    s: &S,
    principal: &str,
    id: &str,
    model: &str,
) -> Result<Value, String> {
    let v = call(s, principal, "realtime.token", token_request(id, model)).await?;
    require(
        v["token"].as_str().is_some_and(|v| !v.is_empty())
            && v["token_ref"].as_str().is_some_and(|v| !v.is_empty()),
        "missing token/reference",
    )?;
    Ok(v)
}
fn attach_request(id: &str, t: &Value, session: &str) -> Value {
    json!({"call_id":id,"token_ref":t["token_ref"],"vendor_session_id":session})
}
fn vendor_usage() -> Value {
    json!([{"role":"input_audio","unit":"second","quantity":"1.250000001","measure":"billed","source":"vendor_reply"}])
}
async fn attach<S: MediaSubject>(
    s: &S,
    v: &RecordingVendor,
    principal: &str,
    id: &str,
    t: &Value,
    session: &str,
) -> Result<Value, String> {
    v.open_session(session, t["token_ref"].as_str().unwrap(), vendor_usage());
    let a = call(
        s,
        principal,
        "realtime.call_started",
        attach_request(id, t, session),
    )
    .await?;
    require(
        a == json!({"call_id":id,"vendor_session_id":session}),
        "attach answer",
    )?;
    Ok(a)
}
async fn end_call<S: MediaSubject>(s: &S, p: &str, id: &str) -> Result<(), String> {
    require(
        call(s, p, "realtime.call_ended", json!({"call_id":id})).await?
            == json!({"call_id":id,"status":"ended"}),
        "end answer",
    )
}
async fn no_effect<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    before: usize,
    count: usize,
) -> Result<(), String> {
    require(
        r.calls().len() == before && feed(s).await?["entries"].as_array().unwrap().len() == count,
        "refusal observed, admitted, released or wrote usage",
    )
}
async fn realtime<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    vendor: &RecordingVendor,
    name: &str,
) -> Result<(), String> {
    match name {
        "realtime_token_uses_requested_model" => {
            for model in ["realtime-b", "realtime-a"] {
                let t = token(s, "alice", model, model).await?;
                let admissions = calls(r, "route.account_for");
                let a = admissions.last().unwrap();
                require(
                    t["model"] == model
                        && t["provider"] == "fixture"
                        && a["model"] == model
                        && a["decision_id"] == "decision"
                        && a["hold_wait_max_s"] == 0,
                    "requested model/decision not used",
                )?;
            }
            Ok(())
        }
        "realtime_token_requires_decision" | "call_id_bounds" => {
            let mut req = token_request("call", "realtime-a");
            let field = if name == "call_id_bounds" {
                req["call_id"] = json!("é".repeat(129));
                "call_id"
            } else {
                req.as_object_mut().unwrap().remove("decision_id");
                "decision_id"
            };
            refusal(
                s,
                "alice",
                "realtime.token",
                req,
                "invalid_params",
                Some(field),
            )
            .await?;
            no_effect(s, r, 0, 0).await
        }
        "realtime_token_refuses_unadmitted_principal" => {
            for recorded in [false, true] {
                if recorded {
                    let t = token(s, "alice", "private", "realtime-a").await?;
                    attach(s, vendor, "alice", "private", &t, "private-session").await?;
                }
                let before = r.calls().len();
                for (principal, agent) in [("mallory", "agent"), ("alice", "missing")] {
                    let mut req = token_request("private", "realtime-a");
                    req["agent_id"] = json!(agent);
                    refusal(s, principal, "realtime.token", req, "unknown_agent", None).await?;
                    no_effect(s, r, before, 0).await?;
                }
            }
            Ok(())
        }
        "realtime_token_bound_to_call"
        | "call_started_refuses_swapped_vendor_call"
        | "call_started_refuses_other_principal" => {
            let t = token(s, "alice", "bound", "realtime-a").await?;
            vendor.open_session("right", t["token_ref"].as_str().unwrap(), vendor_usage());
            let mut req = attach_request("bound", &t, "right");
            let mut principal = "alice";
            let code = match name {
                "realtime_token_bound_to_call" => {
                    req["call_id"] = json!("wrong");
                    "token_call_mismatch"
                }
                "call_started_refuses_other_principal" => {
                    principal = "bob";
                    "token_call_mismatch"
                }
                _ => {
                    let other = token(s, "alice", "other", "realtime-b").await?;
                    vendor.open_session(
                        "swapped",
                        other["token_ref"].as_str().unwrap(),
                        vendor_usage(),
                    );
                    req["vendor_session_id"] = json!("swapped");
                    "vendor_call_mismatch"
                }
            };
            let before = r.calls().len();
            refusal(s, principal, "realtime.call_started", req, code, None).await?;
            no_effect(s, r, before, 0).await?;
            attach(s, vendor, "alice", "bound", &t, "right").await?;
            balanced(r, 1, 0)
        }
        "realtime_token_within_max_ttl" => {
            let d = call(s, "alice", "role.describe", json!({})).await?;
            let ttl = d["token_max_ttl_ms"]
                .as_u64()
                .ok_or("missing token_max_ttl_ms")?;
            let now = r.now_ms();
            r.set_admission_ttl_ms(ttl.saturating_add(1000));
            let t = token(s, "alice", "ttl", "realtime-a").await?;
            let expiry = t["expires_at_ms"].as_u64().ok_or("missing expiry")?;
            require(
                expiry > now && expiry <= now.saturating_add(ttl),
                "token exceeds declared TTL",
            )?;
            let admission_ttl = (ttl / 2).max(1);
            r.set_admission_ttl_ms(admission_ttl);
            let bounded = token(s, "alice", "admission-bound", "realtime-a").await?;
            let bounded_expiry = bounded["expires_at_ms"]
                .as_u64()
                .ok_or("missing bounded expiry")?;
            require(
                bounded_expiry > now && bounded_expiry <= now.saturating_add(admission_ttl),
                "token exceeds admission validity",
            )?;
            s.advance_clock(expiry - now + 1).await.map_err(err)?;
            require(
                calls(r, "route.account_for").len() == 2,
                "unattached admission renewed",
            )?;
            vendor.open_session("expired", t["token_ref"].as_str().unwrap(), vendor_usage());
            let before = r.calls().len();
            refusal(
                s,
                "alice",
                "realtime.call_started",
                attach_request("ttl", &t, "expired"),
                "token_call_mismatch",
                None,
            )
            .await?;
            no_effect(s, r, before, 0).await
        }
        "realtime_token_refuses_recorded_call" => {
            let t = token(s, "alice", "recorded", "realtime-a").await?;
            attach(s, vendor, "alice", "recorded", &t, "recorded-session").await?;
            let before = r.calls().len();
            refusal(
                s,
                "alice",
                "realtime.token",
                token_request("recorded", "realtime-b"),
                "call_id_reuse",
                None,
            )
            .await?;
            no_effect(s, r, before, 0).await
        }
        "call_started_replay_after_expiry" => {
            let t = token(s, "alice", "replay", "realtime-a").await?;
            let a = attach(s, vendor, "alice", "replay", &t, "replay-session").await?;
            s.advance_clock(t["expires_at_ms"].as_u64().unwrap() - r.now_ms() + 1)
                .await
                .map_err(err)?;
            let after_clock = records(s).await?;
            require(
                after_clock.is_empty()
                    || (after_clock.len() == 1
                        && after_clock[0]["call_id"] == "replay"
                        && after_clock[0]["status"] == "ended"),
                "clock advance wrote unexpected usage",
            )?;
            let before = r.calls().len();
            require(
                call(
                    s,
                    "alice",
                    "realtime.call_started",
                    attach_request("replay", &t, "replay-session"),
                )
                .await?
                    == a,
                "expired recorded attach did not replay",
            )?;
            no_effect(s, r, before, after_clock.len()).await?;
            balanced(r, 1, after_clock.len())
        }
        "calls_scoped_by_principal" => {
            s.configure_principals(Principals::new(
                "reader",
                vec![Agent::new("agent", "alice")
                    .with_context_readers(vec!["bob".into(), "carol".into()])],
            ))
            .await
            .map_err(err)?;
            let t = token(s, "alice", "shared", "realtime-a").await?;
            attach(s, vendor, "alice", "shared", &t, "shared-session").await?;
            end_call(s, "bob", "shared").await?;
            balanced(r, 1, 0)?;
            require(
                feed(s).await?["entries"] == json!([]),
                "foreign call_ended ended live call",
            )?;
            require(
                call(
                    s,
                    "alice",
                    "realtime.call_started",
                    attach_request("shared", &t, "shared-session"),
                )
                .await?
                    == json!({"call_id":"shared","vendor_session_id":"shared-session"}),
                "foreign end changed A's live call",
            )?;
            end_call(s, "alice", "shared").await?;
            balanced(r, 1, 1)?;
            token(s, "carol", "shared", "realtime-a").await?;
            refusal(
                s,
                "bob",
                "realtime.token",
                token_request("shared", "realtime-a"),
                "call_id_reuse",
                None,
            )
            .await?;
            Ok(())
        }
        "call_ended_unrecorded" => {
            let t = token(s, "alice", "never-attached", "realtime-a").await?;
            let before = r.calls().len();
            end_call(s, "alice", "never-attached").await?;
            end_call(s, "alice", "never-attached").await?;
            no_effect(s, r, before, 0).await?;
            vendor.open_session("late", t["token_ref"].as_str().unwrap(), vendor_usage());
            refusal(
                s,
                "alice",
                "realtime.call_started",
                attach_request("never-attached", &t, "late"),
                "call_id_reuse",
                None,
            )
            .await?;
            no_effect(s, r, before, 0).await
        }
        "realtime_watch" => watch(s, vendor).await,
        "ended_call_never_reattaches" | "call_end_writes_one_record" => {
            call_ends(s, r, vendor).await
        }
        _ => Err("unimplemented realtime case".into()),
    }
}
async fn watch<S: MediaSubject>(s: &S, vendor: &RecordingVendor) -> Result<(), String> {
    let t = token(s, "alice", "watch", "realtime-a").await?;
    let secret = t["token"].as_str().ok_or("token missing")?;
    attach(s, vendor, "alice", "watch", &t, "watch-session").await?;
    for (principal, id) in [("bob", "watch"), ("mallory", "watch"), ("alice", "missing")] {
        match s.open_watch(principal, id).await {
            Ok(()) => return Err("watch admitted a foreign principal or unknown call".into()),
            Err(e) => {
                let body = e.refusal.ok_or("watch failed without an ERROR body")?;
                require(
                    !body.to_string().contains(secret),
                    "watch refusal quoted the token",
                )?;
                require(
                    body["code"] == "unknown_call",
                    "watch refusal was not unknown_call",
                )?;
                require(
                    body["detail"]["class"].is_string(),
                    "watch refusal omitted class",
                )?;
            }
        }
    }
    s.open_watch("alice", "watch").await.map_err(err)?;
    vendor.push_tool_call("watch-session").map_err(err)?;
    wait_watch(s, secret, "tool_call").await?;
    end_call(s, "alice", "watch").await?;
    wait_watch(s, secret, "ended").await
}

async fn wait_watch<S: MediaSubject>(s: &S, secret: &str, expected: &str) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let remaining = STATUS_POLL_TIMEOUT.saturating_sub(started.elapsed());
        let events = tokio::time::timeout(remaining, s.poll_watch("alice", "watch"))
            .await
            .map_err(|_| format!("watch did not push {expected}"))?
            .map_err(err)?;
        let mut found = false;
        for event in events {
            require(
                !event.to_string().contains(secret),
                "watch push quoted the token",
            )?;
            require(
                event["call_id"] == "watch",
                "watch push has the wrong call_id",
            )?;
            require(
                event["at_ms"].as_u64().is_some(),
                "watch push omitted epoch at_ms",
            )?;
            if expected == "tool_call" && !found && event["type"] == "started" {
                continue;
            }
            require(
                !found && event["type"] == expected,
                "watch push order or type differs",
            )?;
            found = true;
        }
        if found {
            return Ok(());
        }
        if started.elapsed() >= STATUS_POLL_TIMEOUT {
            return Err(format!("watch did not push {expected}"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn call_ends<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    vendor: &RecordingVendor,
) -> Result<(), String> {
    let describe = call(s, "alice", "role.describe", json!({})).await?;
    let silence = describe["call_silence_timeout_s"]
        .as_u64()
        .ok_or("missing silence timeout")?
        * 1000;
    for (i, mode) in ["explicit", "silence", "restart"].into_iter().enumerate() {
        let first = token(s, "alice", mode, "realtime-a").await?;
        let fresh = token(s, "alice", mode, "realtime-a").await?;
        require(
            first["token_ref"] != fresh["token_ref"] && first["token"] != fresh["token"],
            "token query was replay-cached",
        )?;
        attach(s, vendor, "alice", mode, &first, &format!("session-{mode}")).await?;
        match mode {
            "silence" => s.advance_clock(silence + 1).await.map_err(err)?,
            "restart" => s.restart().await.map_err(err)?,
            _ => end_call(s, "alice", mode).await?,
        }
        end_call(s, "alice", mode).await?;
        let rs = wait_records(s, i + 1).await?;
        require(
            rs.len() == i + 1
                && rs[i]["call_id"] == mode
                && rs[i]["status"] == "ended"
                && rs[i]["model"] == "realtime-a"
                && rs[i]["measurements"] == vendor_usage(),
            "end must write one vendor-stated usage record",
        )?;
        balanced(r, i + 1, i + 1)?;
        let before = r.calls().len();
        vendor.open_session(
            &format!("new-{mode}"),
            fresh["token_ref"].as_str().unwrap(),
            vendor_usage(),
        );
        refusal(
            s,
            "alice",
            "realtime.call_started",
            attach_request(mode, &fresh, &format!("new-{mode}")),
            "call_id_reuse",
            None,
        )
        .await?;
        refusal(
            s,
            "alice",
            "realtime.token",
            token_request(mode, "realtime-a"),
            "call_id_reuse",
            None,
        )
        .await?;
        no_effect(s, r, before, i + 1).await?;
    }
    Ok(())
}

fn check_admission_trace(r: &RecordingRoutingPeer) -> Result<(), String> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut pins: BTreeMap<String, (Value, Value, String, u64)> = BTreeMap::new();
    let mut finished = BTreeSet::new();
    for (i, (method, v)) in r.calls().iter().enumerate() {
        if method == "route.account_for" {
            require(v["hold_wait_max_s"] == 0, "admission hold_wait_max_s")?;
            let key = v["conversation_key"]
                .as_str()
                .ok_or("missing conversation_key")?
                .to_string();
            require(
                v["provider"] == "fixture"
                    && v["model"]
                        == if key.starts_with("stream:") {
                            "stream"
                        } else {
                            "realtime-a"
                        },
                "admission or renewal named the wrong engine",
            )?;
            let id = v["admission_id"].as_str().ok_or("missing admission_id")?;
            if v["mode"] == "renew" {
                let previous = pins.get(&key).ok_or("renewal before admission")?;
                require(
                    v["decision_id"].is_null()
                        && v["expected_pin"] == json!({"id":previous.0,"generation":previous.1})
                        && id == format!("{}/renew/{}", previous.2, previous.3 + 1)
                        && !finished.contains(&previous.2),
                    "renewal used wrong pin, id, decision or renewed released work",
                )?;
                pins.insert(
                    key.clone(),
                    (
                        json!(format!("pin:{key}")),
                        json!(i + 1),
                        previous.2.clone(),
                        previous.3 + 1,
                    ),
                );
            } else {
                require(
                    v["decision_id"] == "decision",
                    "first admission lost decision",
                )?;
                let (prefix, len) = if key.starts_with("stream:") {
                    ("/s/", 106)
                } else {
                    ("/t/", 104)
                };
                let suffix = id
                    .strip_prefix(&format!("{key}{prefix}"))
                    .ok_or("admission key prefix")?;
                require(
                    id.len() == len
                        && suffix.len() == 32
                        && suffix
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "admission id shape",
                )?;
                pins.insert(
                    key.clone(),
                    (json!(format!("pin:{key}")), json!(i + 1), id.into(), 0),
                );
            }
        } else if method == "route.account_observe" {
            let id = v["admission_id"]
                .as_str()
                .ok_or("observation admission id")?;
            let event = if v["kind"] == "run_finished" {
                require(finished.insert(id.to_string()), "admission released twice")?;
                "finished"
            } else {
                "started"
            };
            require(
                v["event_id"] == format!("{id}/{event}") && !id.contains("/renew/"),
                "observe names renewal instead of original admission",
            )?;
        }
    }
    Ok(())
}
async fn admission_lifecycle<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    v: &RecordingVendor,
) -> Result<(), String> {
    s.hold_engine("stream").await.map_err(err)?;
    script(
        s,
        "stream",
        vec![ScriptStep::Pause, ScriptStep::Final(segment(0, "words"))],
    )
    .await?;
    open(s, "shared").await?;
    s.open_stream("bob", first("shared")).await.map_err(err)?;
    audio(s, "shared", 0, 32).await?;
    s.advance_clock(1001).await.map_err(err)?;
    let a = calls(r, "route.account_for");
    for (principal, index) in [("alice", 0), ("bob", 1)] {
        let canonical = format!("{{\"principal\":\"{principal}\",\"stream_id\":\"shared\"}}");
        require(
            a[index]["conversation_key"] == format!("stream:{}", sha256(canonical.as_bytes())),
            "stream key is not scoped by principal",
        )?;
    }
    require(
        a.iter().filter(|a| a["mode"] == "renew").count() >= 2
            && events(r, "admission_started").is_empty(),
        "queued streams did not renew before starting",
    )?;
    s.release_engine("stream").await.map_err(err)?;
    wait_started(r, 1).await?;
    s.close_stream("bob", "shared").await.map_err(err)?;
    s.advance_clock(1001).await.map_err(err)?;
    require(
        calls(r, "route.account_for")
            .iter()
            .filter(|a| a["mode"] == "renew")
            .count()
            >= 3,
        "running stream did not renew",
    )?;
    finish(s, "shared", 1).await?;
    let old = token(s, "alice", "shared", "realtime-a").await?;
    token(s, "bob", "shared", "realtime-a").await?;
    let before = r.calls().len();
    s.advance_clock(1001).await.map_err(err)?;
    require(
        r.calls().len() == before,
        "unattached tokens renewed or released explicitly",
    )?;
    r.set_admission_ttl_ms(100);
    let fresh = token(s, "alice", "shared", "realtime-a").await?;
    let admissions = calls(r, "route.account_for");
    let token_admissions: Vec<_> = admissions
        .iter()
        .filter(|a| {
            a["conversation_key"]
                .as_str()
                .is_some_and(|k| k.starts_with("call:"))
                && a["mode"] != "renew"
        })
        .collect();
    require(
        token_admissions.len() == 3,
        "token request did not create a fresh admission",
    )?;
    for (p, index) in [("alice", 0), ("bob", 1)] {
        let canonical = format!("{{\"call_id\":\"shared\",\"principal\":\"{p}\"}}");
        require(
            token_admissions[index]["conversation_key"]
                == format!("call:{}", sha256(canonical.as_bytes())),
            "call key is not scoped by principal",
        )?;
    }
    require(
        token_admissions[0]["admission_id"] != token_admissions[2]["admission_id"],
        "token reused lapsed admission",
    )?;
    v.open_session("old", old["token_ref"].as_str().unwrap(), vendor_usage());
    refusal(
        s,
        "alice",
        "realtime.call_started",
        attach_request("shared", &old, "old"),
        "token_call_mismatch",
        None,
    )
    .await?;
    attach(s, v, "alice", "shared", &fresh, "fresh").await?;
    require(
        events(r, "admission_started").last().unwrap()["admission_id"]
            == token_admissions[2]["admission_id"],
        "attach started the old admission",
    )?;
    s.advance_clock(101).await.map_err(err)?;
    let all = calls(r, "route.account_for");
    require(
        all.iter()
            .any(|a| a["mode"] == "renew" && a["model"] == "realtime-a"),
        "attached call did not renew",
    )?;
    end_call(s, "alice", "shared").await?;
    balanced(r, 2, 3)?;
    let before = r.calls().len();
    s.advance_clock(2000).await.map_err(err)?;
    require(
        r.calls().len() == before,
        "released stream/call kept renewing",
    )?;
    check_admission_trace(r)
}
async fn renewal_lapse<S: MediaSubject>(
    s: &S,
    r: &RecordingRoutingPeer,
    v: &RecordingVendor,
) -> Result<(), String> {
    r.set_admission_ttl_ms(500);
    script(s, "stream", vec![ScriptStep::Pause]).await?;
    open(s, "running").await?;
    audio(s, "running", 0, 32).await?;
    wait_started(r, 1).await?;
    s.hold_engine("stream").await.map_err(err)?;
    open(s, "queued").await?;
    audio(s, "queued", 0, 32).await?;
    let t = token(s, "alice", "lapse", "realtime-a").await?;
    attach(s, v, "alice", "lapse", &t, "lapse-session").await?;
    r.refuse_renewals(true);
    s.advance_clock(501).await.map_err(err)?;
    for id in ["running", "queued"] {
        let out = answer(s, &observe(s, id, true).await?).await?;
        require(
            out["failure"]["code"] == "routing_refused"
                && out["failure"]["detail"]["routing_code"] == "account_hold",
            "lapsed stream did not report routing refusal",
        )?;
    }
    let rs = wait_records(s, 2).await?;
    require(
        rs.len() == 2
            && rs
                .iter()
                .any(|r| r["stream_id"] == "running" && r["status"] == "failed")
            && rs
                .iter()
                .any(|r| r["call_id"] == "lapse" && r["status"] == "ended")
            && rs.iter().all(|r| r["stream_id"] != "queued"),
        "lapse record attribution/count",
    )?;
    balanced(r, 2, 3)?;
    let before = r.calls().len();
    s.advance_clock(2000).await.map_err(err)?;
    no_effect(s, r, before, 2).await
}
