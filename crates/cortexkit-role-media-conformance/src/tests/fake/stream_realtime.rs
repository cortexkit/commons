use super::*;

#[derive(Default)]
pub(super) struct Extras {
    pub vendor: RecordingVendor,
    pub speech_check: bool,
    pub packs: BTreeMap<(String, String), Value>,
    pub front_unavailable: BTreeMap<String, String>,
    streams: BTreeMap<(String, String), Stream>,
    seen: BTreeSet<(String, String)>,
    refuse_stream: bool,
    tokens: BTreeMap<String, Token>,
    calls: BTreeMap<(String, String), Call>,
    ended: BTreeSet<(String, String)>,
    watches: BTreeMap<(String, String), Vec<Value>>,
}
struct Stream {
    lease: Lease,
    first: Value,
    script: Script,
    steps: VecDeque<ScriptStep>,
    seq: u64,
    pending: BTreeMap<u64, Value>,
    audio: Vec<Vec<u8>>,
    bytes: u64,
    active: bool,
    admission: String,
    events: Vec<Value>,
    segments: Vec<Value>,
    closed: bool,
    answer: Option<Value>,
}
#[derive(Clone)]
struct Token {
    lease: Lease,
    principal: String,
    request: Value,
    expiry: u64,
    admission: String,
}
#[derive(Clone)]
struct Lease {
    key: String,
    admission: String,
    pin: Value,
    renewal: u64,
    next: u64,
}
struct Call {
    token: Token,
    request: Value,
    answer: Value,
    usage: Value,
    ended: bool,
    silence: u64,
}

impl State {
    pub(super) fn extra_times(&self) -> Vec<u64> {
        self.extra
            .streams
            .values()
            .filter(|s| !s.closed)
            .map(|s| s.lease.next)
            .chain(
                self.extra
                    .calls
                    .values()
                    .filter(|c| !c.ended)
                    .flat_map(|c| [c.token.lease.next, c.silence]),
            )
            .collect()
    }
    fn renew_extra(&mut self, lease: &mut Lease, req: &Value) -> SubjectResult<bool> {
        if lease.pin["lease_valid_until_ms"].as_u64().unwrap() <= self.now() {
            return Ok(false);
        }
        lease.renewal += 1;
        let mut params = json!({"provider":"fixture","model":req["model"],"conversation_key":lease.key,"admission_id":format!("{}/renew/{}",lease.admission,lease.renewal),"mode":"renew","expected_pin":{"id":lease.pin["id"],"generation":lease.pin["generation"]},"hold_wait_max_s":0});
        if self.fault("stream_realtime_admission_lifecycle") {
            params["hold_wait_max_s"] = json!(1);
        }
        match self.routing.call("route.account_for", params) {
            Ok(v) => {
                lease.pin = v["pin"].clone();
                lease.next = self.now()
                    + ((lease.pin["lease_valid_until_ms"].as_u64().unwrap() - self.now()) * 3 / 4)
                        .max(1);
            }
            Err(_) => {
                lease.next =
                    (self.now() + 100).min(lease.pin["lease_valid_until_ms"].as_u64().unwrap());
            }
        }
        Ok(true)
    }
    pub(super) fn extra_close(&mut self, p: &str, id: &str) -> SubjectResult<()> {
        let key = (p.into(), id.into());
        if let Some(mut st) = self.extra.streams.remove(&key) {
            if !st.closed {
                if st.active {
                    self.extra_record(p, ("stream_id", id), &st.first, "cancelled", None, None);
                }
                self.extra_observe(&st.admission, "run_finished")?;
                st.closed = true;
                st.active = false;
            }
            self.extra.streams.insert(key, st);
        }
        Ok(())
    }
    pub(super) fn extra_release(&mut self, model: &str) -> SubjectResult<()> {
        if model != "stream" {
            return Ok(());
        }
        let streams = std::mem::take(&mut self.extra.streams);
        for (key, mut st) in streams {
            if !st.closed && !st.active && st.bytes > 0 {
                st.active = true;
                self.extra_observe(&st.admission, "admission_started")?;
                // The held stream now starts its scripted attempt, without consuming another frame.
                if matches!(st.steps.front(), Some(ScriptStep::Pause)) {
                    st.steps.pop_front();
                }
            }
            self.extra.streams.insert(key, st);
        }
        Ok(())
    }
    pub(super) fn extra_call(
        &mut self,
        p: &str,
        method: &str,
        params: Value,
    ) -> SubjectResult<Value> {
        match method {
            "context.for_agent" => {
                if !self.allowed(p, &params["agent_id"]) && !self.fault("context_pack_admission") {
                    return Err(refuse("unknown_agent", None));
                }
                let pack = self
                    .extra
                    .packs
                    .get(&(
                        params["agent_id"].as_str().unwrap_or("").into(),
                        params["purpose"].as_str().unwrap_or("").into(),
                    ))
                    .cloned()
                    .unwrap_or(Value::Null);
                if params["if_version"] == pack["version"]
                    && params.get("if_version").is_some()
                    && !self.fault("context_pack_if_version")
                {
                    Ok(json!({"unchanged":true,"version":pack["version"]}))
                } else {
                    Ok(pack)
                }
            }
            "realtime.token" => self.mint(p, params),
            "realtime.call_started" => self.attach_call(p, params),
            "realtime.call_ended" => {
                let id = params["call_id"].as_str().unwrap();
                let key = (p.into(), id.into());
                self.end_call_key(&key)?;
                if self.fault("calls_scoped_by_principal") {
                    let keys: Vec<_> = self
                        .extra
                        .calls
                        .keys()
                        .filter(|(_, i)| i == id)
                        .cloned()
                        .collect();
                    for key in keys {
                        self.end_call_key(&key)?;
                    }
                }
                if self.fault("call_ended_unrecorded") {
                    self.extra_observe("unrecorded", "run_finished")?;
                }
                self.extra.ended.insert(key);
                Ok(json!({"call_id":id,"status":"ended"}))
            }
            _ => Err(SubjectError::new("unknown extra method")),
        }
    }
    fn allowed(&self, p: &str, agent: &Value) -> bool {
        self.principals.as_ref().is_some_and(|ps| {
            ps.agents.iter().any(|a| {
                agent.as_str() == Some(a.agent_id.as_str())
                    && (a.owner == p || a.context_readers.iter().any(|r| r == p))
            })
        })
    }
    fn extra_admit(
        &mut self,
        p: &str,
        id: &str,
        req: &Value,
        stream: bool,
    ) -> SubjectResult<Lease> {
        let key = if stream {
            format!(
                "stream:{}",
                sha256(
                    serde_json::to_string(&json!({"principal":p,"stream_id":id}))
                        .unwrap()
                        .as_bytes()
                )
            )
        } else {
            format!(
                "call:{}",
                sha256(
                    serde_json::to_string(&json!({"call_id":id,"principal":p}))
                        .unwrap()
                        .as_bytes()
                )
            )
        };
        let unique = self.id("").parse::<u64>().unwrap();
        let admission = format!("{key}/{}/{unique:032x}", if stream { "s" } else { "t" });
        let pin = self.routing.call("route.account_for",json!({"provider":"fixture","model":req["model"],"decision_id":req["decision_id"],"conversation_key":key,"admission_id":admission,"hold_wait_max_s":0})).map_err(|e| self.routing_refusal(e))?["pin"].clone();
        let next = self.now()
            + ((pin["admission_valid_until_ms"].as_u64().unwrap() - self.now()) * 3 / 4).max(1);
        Ok(Lease {
            key,
            admission,
            pin,
            renewal: 0,
            next,
        })
    }
    fn extra_observe(&self, admission: &str, kind: &str) -> SubjectResult<()> {
        self.routing.call("route.account_observe",json!({"kind":kind,"admission_id":admission,"event_id":format!("{admission}/{}",if kind == "run_finished" {"finished"} else {"started"})}))?;
        Ok(())
    }
    fn extra_record(
        &mut self,
        p: &str,
        (work, id): (&str, &str),
        req: &Value,
        status: &str,
        failure: Option<Value>,
        usage: Option<Value>,
    ) {
        let record_id = self.id("record");
        let mut v = json!({"record_id":record_id,"principal":p,"provider":"fixture","model":req["model"],"decision_id":req["decision_id"],"charge_basis":"vendor","credential_ref":"fixture-credential","auth_method":"api_key","started_at_ms":self.now(),"finished_at_ms":self.now(),"status":status,"selectors":{},"measurements":usage.unwrap_or(json!([]))});
        v[work] = json!(id);
        if let Some(f) = failure {
            v["failure"] = f;
        }
        self.usage
            .push(json!({"seq":self.usage.len()+1,"record":v}));
    }
    fn mint(&mut self, p: &str, mut req: Value) -> SubjectResult<Value> {
        if self.fault("realtime_front_check_order") {
            self.front_ready(&req)?;
        }
        if !self.allowed(p, &req["agent_id"])
            && !self.fault("realtime_token_refuses_unadmitted_principal")
        {
            return Err(refuse("unknown_agent", None));
        }
        let id = req["call_id"].as_str().unwrap_or("").to_string();
        let key = (p.into(), id.clone());
        if (self.extra.calls.contains_key(&key) || self.extra.ended.contains(&key))
            && !self.fault("realtime_token_refuses_recorded_call")
        {
            return Err(refuse("call_id_reuse", None));
        }
        if !self.fault("call_id_bounds") && !(1..=128).contains(&id.chars().count()) {
            return Err(refuse("invalid_params", Some("call_id")));
        }
        if req.get("decision_id").is_none() && !self.fault("realtime_token_requires_decision") {
            return Err(refuse("invalid_params", Some("decision_id")));
        }
        if self.fault("realtime_token_uses_requested_model") {
            req["model"] = json!("realtime-a");
        }
        if !self.fault("realtime_dictation_requires_realtime_engine")
            && !self
                .engines
                .iter()
                .any(|e| e.kind.as_str() == "realtime" && req["model"] == e.model)
        {
            return Err(refuse("unsupported", Some("model")));
        }
        if !self.fault("realtime_front_unavailable") {
            self.front_ready(&req)?;
        }
        let lease = self.extra_admit(p, &id, &req, false)?;
        let admission = lease.admission.clone();
        let mut expiry = self.now()
            + if self.fault("realtime_token_within_max_ttl") {
                501
            } else {
                500
            };
        if !self.fault("realtime_token_within_max_ttl") {
            expiry = expiry.min(lease.pin["admission_valid_until_ms"].as_u64().unwrap());
        }
        if self.fault("realtime_token_admission_bound") {
            expiry = self.now() + 500;
        }
        let reference = self.id("synthetic-ref-");
        let token = self.id("synthetic-token-");
        self.extra.tokens.insert(
            reference.clone(),
            Token {
                lease,
                principal: p.into(),
                request: req.clone(),
                expiry,
                admission,
            },
        );
        Ok(
            json!({"token":token,"token_ref":reference,"expires_at_ms":expiry,"model":req["model"],"provider":"fixture"}),
        )
    }
    fn front_ready(&self, req: &Value) -> SubjectResult<()> {
        if req["purpose"] == "voice_front" || self.fault("realtime_front_dictation") {
            if let Some(reason) = req["agent_id"]
                .as_str()
                .and_then(|id| self.extra.front_unavailable.get(id))
            {
                let mut error = refuse("front_unavailable", None);
                error.refusal.as_mut().unwrap()["detail"]["reason"] = json!(reason);
                return Err(error);
            }
        }
        Ok(())
    }
    pub(super) fn watch_open(&mut self, p: &str, id: &str) -> SubjectResult<()> {
        let key = (p.into(), id.into());
        if !self.extra.calls.contains_key(&key) {
            if self.fault("realtime_watch_scope") {
                return Ok(());
            }
            return Err(refuse("unknown_call", None));
        }
        self.extra.watches.entry(key).or_default();
        Ok(())
    }
    pub(super) fn watch_poll(&mut self, p: &str, id: &str) -> SubjectResult<Vec<Value>> {
        let key = (p.into(), id.into());
        let call = self
            .extra
            .calls
            .get(&key)
            .ok_or_else(|| refuse("unknown_call", None))?;
        let session = call.request["vendor_session_id"].as_str().unwrap();
        let pending = self.extra.vendor.take_tool_calls(session);
        let now = self.now();
        let wrong_id = self.fault("realtime_watch");
        let invalid_tool = self.fault("realtime_watch_tool_not_string");
        let events = self
            .extra
            .watches
            .get_mut(&key)
            .ok_or_else(|| refuse("unknown_call", None))?;
        for _ in 0..pending {
            let mut event =
                json!({"type":"tool_call","call_id":if wrong_id {"wrong"} else {id},"at_ms":now});
            if invalid_tool {
                event["tool"] = json!(true);
            }
            events.push(event);
        }
        Ok(std::mem::take(events))
    }
    fn attach_call(&mut self, p: &str, req: Value) -> SubjectResult<Value> {
        let id = req["call_id"].as_str().unwrap_or("").to_string();
        let key = (p.into(), id.clone());
        if let Some(c) = self.extra.calls.get(&key) {
            if c.request == req && !self.fault("call_started_replay_after_expiry") {
                return Ok(c.answer.clone());
            }
            if !self.fault("ended_call_never_reattaches") {
                return Err(refuse("call_id_reuse", None));
            }
        }
        if self.extra.ended.contains(&key) && !self.fault("ended_call_never_reattaches") {
            return Err(refuse("call_id_reuse", None));
        }
        let t = self
            .extra
            .tokens
            .get(req["token_ref"].as_str().unwrap_or(""))
            .cloned()
            .ok_or_else(|| refuse("token_call_mismatch", None))?;
        if (t.principal != p && !self.fault("call_started_refuses_other_principal"))
            || (t.request["call_id"] != id && !self.fault("realtime_token_bound_to_call"))
            || t.expiry < self.now()
        {
            return Err(refuse("token_call_mismatch", None));
        }
        let session = req["vendor_session_id"].as_str().unwrap_or("");
        let (reference, usage) = self
            .extra
            .vendor
            .session(session)
            .ok_or_else(|| refuse("vendor_call_mismatch", None))?;
        if reference != req["token_ref"] && !self.fault("call_started_refuses_swapped_vendor_call")
        {
            return Err(refuse("vendor_call_mismatch", None));
        }
        self.extra_observe(&t.admission, "admission_started")?;
        let answer = json!({"call_id":id,"vendor_session_id":session});
        self.extra.calls.insert(
            key,
            Call {
                token: t,
                request: req,
                answer: answer.clone(),
                usage,
                ended: false,
                silence: self.now() + 3000,
            },
        );
        Ok(answer)
    }
    fn end_call_key(&mut self, key: &(String, String)) -> SubjectResult<()> {
        if let Some(mut c) = self.extra.calls.remove(key) {
            if !c.ended {
                let mut usage = c.usage.clone();
                if self.fault("call_end_writes_one_record") {
                    usage[0]["quantity"] = json!("2");
                }
                self.extra_record(
                    &key.0,
                    ("call_id", &key.1),
                    &c.token.request,
                    "ended",
                    None,
                    Some(usage),
                );
                self.extra_observe(&c.token.admission, "run_finished")?;
                c.ended = true;
                let now = self.now();
                let misplaced_tool = self.fault("realtime_watch_tool_outside_call");
                if let Some(events) = self.extra.watches.get_mut(key) {
                    let mut event = json!({"type":"ended","call_id":key.1,"at_ms":now});
                    if misplaced_tool {
                        event["tool"] = json!("ask");
                    }
                    events.push(event);
                }
                self.extra.ended.insert(key.clone());
            }
            self.extra.calls.insert(key.clone(), c);
        }
        Ok(())
    }
    pub(super) fn extra_advance(&mut self) -> SubjectResult<()> {
        let streams = std::mem::take(&mut self.extra.streams);
        for (key, mut st) in streams {
            if !st.closed
                && st.lease.next <= self.now()
                && !self.renew_extra(&mut st.lease, &st.first)?
            {
                let mut f = failed("routing_refused", "transient");
                f["detail"] = json!({"routing_code":"account_hold"});
                if self.fault("stream_realtime_renewal_lapse") {
                    f["code"] = json!("wrong");
                }
                self.finish_stream(&key.0, &key.1, &mut st, Some(f))?;
            }
            self.extra.streams.insert(key, st);
        }
        let calls = std::mem::take(&mut self.extra.calls);
        let mut ended = Vec::new();
        for (key, mut c) in calls {
            if !c.ended
                && (c.silence <= self.now()
                    || (c.token.lease.next <= self.now()
                        && !self.renew_extra(&mut c.token.lease, &c.token.request)?))
            {
                ended.push(key.clone());
            }
            self.extra.calls.insert(key, c);
        }
        for key in ended {
            self.end_call_key(&key)?;
        }
        Ok(())
    }
    pub(super) fn extra_restart(&mut self) -> SubjectResult<()> {
        let keys: Vec<_> = self.extra.calls.keys().cloned().collect();
        for k in keys {
            self.end_call_key(&k)?;
        }
        let streams = std::mem::take(&mut self.extra.streams);
        for (key, mut st) in streams {
            if !st.closed {
                if st.active && !self.fault("stream_restart") {
                    self.extra_record(
                        &key.0,
                        ("stream_id", &key.1),
                        &st.first,
                        "failed",
                        Some(failed("module_restarted", "transient")),
                        None,
                    );
                }
                self.extra_observe(&st.admission, "run_finished")?;
                st.closed = true;
                st.active = false;
                st.answer = None;
            }
            self.extra.streams.insert(key, st);
        }
        Ok(())
    }
    pub(super) fn open_stream(&mut self, p: &str, req: Value) -> SubjectResult<()> {
        let id = req["stream_id"].as_str().unwrap_or("").to_string();
        let key = (p.into(), id.clone());
        if self.extra.seen.contains(&key) && !self.fault("stream_id_reuse_refused") {
            return Err(refuse("stream_id_reuse", None));
        }
        if !self.fault("stream_id_bounds") && !(1..=128).contains(&id.chars().count()) {
            return Err(refuse("invalid_params", Some("stream_id")));
        }
        if req.get("decision_id").is_none() && !self.fault("stream_remote_requires_decision") {
            return Err(refuse("invalid_params", Some("decision_id")));
        }
        if self.extra.refuse_stream {
            self.extra.refuse_stream = false;
            if self.fault("stream_first_frame_refused") {
                self.extra.seen.insert(key);
            }
            let mut e = refuse("realtime_lane_full", None);
            e.refusal.as_mut().unwrap()["detail"]["class"] = json!("transient");
            return Err(e);
        }
        let lease = self.extra_admit(p, &id, &req, true)?;
        let admission = lease.admission.clone();
        let sc = self.scripts.get("stream").cloned().unwrap_or_default();
        self.extra.seen.insert(key.clone());
        self.extra.streams.insert(
            key,
            Stream {
                lease,
                first: req,
                steps: sc.steps.clone().into(),
                script: sc,
                seq: 0,
                pending: BTreeMap::new(),
                audio: vec![],
                bytes: 0,
                active: false,
                admission,
                events: vec![],
                segments: vec![],
                closed: false,
                answer: None,
            },
        );
        Ok(())
    }
    pub(super) fn refuse_stream(&mut self, code: &str) -> SubjectResult<()> {
        if code != "realtime_lane_full" {
            return Err(SubjectError::new(
                "refuse_next_stream accepts only realtime_lane_full",
            ));
        }
        self.extra.refuse_stream = true;
        Ok(())
    }
    pub(super) fn stream_poll(&mut self, p: &str, id: &str) -> SubjectResult<StreamObservation> {
        let bare = self.fault("stream_reply_envelope");
        let extra_keys = self.fault("stream_reply_envelope_extra_keys");
        let st = self
            .extra
            .streams
            .get_mut(&(p.into(), id.into()))
            .ok_or_else(|| SubjectError::new("unknown stream"))?;
        let mut out = StreamObservation::new()
            .with_events(std::mem::take(&mut st.events))
            .with_closed(st.closed);
        out.answer = st.answer.clone().map(|answer| {
            if bare {
                answer
            } else if extra_keys {
                json!({"result":answer,"extra":true})
            } else {
                json!({"result":answer})
            }
        });
        Ok(out)
    }
    pub(super) fn stream_frame(&mut self, p: &str, id: &str, frame: Value) -> SubjectResult<()> {
        let mut key = (p.into(), id.into());
        if self.fault("stream_audio_principal_scope") {
            if let Some(found) = self.extra.streams.keys().find(|(_, sid)| sid == id) {
                key = found.clone();
            }
        }
        let mut st = self
            .extra
            .streams
            .remove(&key)
            .ok_or_else(|| refuse("stream_closed", None))?;
        let result = self.accept_stream_frame(p, id, &mut st, frame);
        self.extra.streams.insert(key, st);
        result
    }
    pub(super) fn stream_audio_inputs(&self, p: &str, id: &str) -> SubjectResult<Vec<Vec<u8>>> {
        self.extra
            .streams
            .get(&(p.into(), id.into()))
            .map(|st| st.audio.clone())
            .ok_or_else(|| refuse("stream_closed", None))
    }
    fn accept_stream_frame(
        &mut self,
        p: &str,
        id: &str,
        st: &mut Stream,
        frame: Value,
    ) -> SubjectResult<()> {
        if st.closed {
            return Err(refuse("stream_closed", None));
        }
        let seq = frame["seq"].as_u64().unwrap();
        let arrival_order = self.fault("stream_audio_reorders_window");
        if !arrival_order
            && (seq < st.seq
                || st.pending.contains_key(&seq)
                || (seq.saturating_sub(st.seq) >= 8 && !self.fault("stream_audio_outside_window")))
        {
            return Err(refuse("stream_seq_mismatch", None));
        }
        let bytes = frame["data"]
            .as_str()
            .map(|d| STANDARD.decode(d).unwrap().len() as u64)
            .unwrap_or(0);
        st.bytes += bytes;
        let e = self.engines.iter().find(|e| e.model == "stream").unwrap();
        let limit = if st.bytes > e.max_input_bytes.unwrap_or(u64::MAX) {
            Some("max_input_bytes")
        } else if st.bytes > e.max_duration_ms.unwrap_or(u64::MAX).saturating_mul(32) {
            Some("max_duration_ms")
        } else {
            None
        };
        if let Some(limit) = limit {
            let mut f = failed("input_too_large", "permanent");
            f["detail"] = json!({"field":"audio","limit":limit});
            if self.fault("stream_input_limits") {
                f["code"] = json!("wrong");
            }
            self.finish_stream(p, id, st, Some(f))?;
            if self.fault("stream_audio_crossing_frame") {
                return Ok(());
            }
            let mut e = refuse("input_too_large", Some("audio"));
            e.refusal.as_mut().unwrap()["detail"]["limit"] = json!(limit);
            return Err(e);
        }
        if arrival_order {
            return self.process_stream(p, id, st, frame);
        }
        st.pending.insert(seq, frame);
        while let Some(frame) = st.pending.remove(&st.seq) {
            st.seq += 1;
            self.process_stream(p, id, st, frame)?;
            if st.closed {
                break;
            }
        }
        Ok(())
    }
    fn process_stream(
        &mut self,
        p: &str,
        id: &str,
        st: &mut Stream,
        frame: Value,
    ) -> SubjectResult<()> {
        let audio = frame["data"].as_str().map(|d| STANDARD.decode(d).unwrap());
        let bytes = audio.as_ref().map_or(0, |v| v.len());
        if !st.active && !self.held.contains("stream") && bytes > 0 {
            st.active = true;
            self.extra_observe(&st.admission, "admission_started")?;
        }
        if st.active {
            if let Some(audio) = audio {
                st.audio.push(audio);
            }
            while let Some(step) = st.steps.pop_front() {
                match step {
                    ScriptStep::Pause => break,
                    ScriptStep::Partial(mut v) => {
                        v["type"] = json!("partial");
                        if serde_json::to_vec(&v).unwrap().len() <= 3_145_728
                            && !self.fault("stream_partials_replace_finals_append")
                        {
                            st.events.push(v);
                        }
                    }
                    ScriptStep::Final(v) => self.stream_final(st, v),
                    ScriptStep::Output(v)
                        if v["no_speech"] == true
                            && v["segments"].as_array().is_some_and(|a| !a.is_empty()) =>
                    {
                        return self.finish_stream(
                            p,
                            id,
                            st,
                            Some(failed("engine_empty_result", "transient")),
                        )
                    }
                    _ => {}
                }
            }
        }
        if frame["end"] == true {
            let silence = self.extra.speech_check && st.script.speech_check == Some(false);
            let f = if st.segments.is_empty()
                && !silence
                && !self.fault("stream_empty_result_needs_speech_check")
            {
                Some(failed("engine_empty_result", "transient"))
            } else {
                None
            };
            self.finish_stream(p, id, st, f)?;
        }
        Ok(())
    }
    fn stream_final(&self, st: &mut Stream, v: Value) {
        let oversized = serde_json::to_vec(&v).unwrap().len() > 3_145_600;
        if oversized && !self.fault("stream_oversize_final_splits") {
            let words = v["words"].as_array().unwrap();
            for chunk in words.chunks(1000) {
                let text = chunk
                    .iter()
                    .map(|w| w["text"].as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(" ");
                self.push_final(
                    st,
                    json!({"segment":st.segments.len(),"text":text,"words":chunk}),
                );
            }
        } else {
            self.push_final(st, v);
        }
    }
    fn push_final(&self, st: &mut Stream, mut v: Value) {
        if !v["text"].as_str().unwrap_or("").trim().is_empty() {
            st.segments.push(v.clone());
        }
        v["type"] = json!("final");
        st.events.push(v);
    }
    fn finish_stream(
        &mut self,
        p: &str,
        id: &str,
        st: &mut Stream,
        f: Option<Value>,
    ) -> SubjectResult<()> {
        let status = if f.is_some() { "failed" } else { "done" };
        let mut out = json!({"segments":st.segments});
        if let Some(f) = f.clone() {
            out["failure"] = f;
        } else {
            out["no_speech"] = json!(st.segments.is_empty());
        }
        let encoded = serde_json::to_vec(&out).unwrap();
        if encoded.len() > 3_145_728 {
            out = json!({"output_object":self.store(p,&encoded,"application/json")});
        }
        st.answer = Some(out);
        st.closed = true;
        if st.active {
            self.extra_record(p, ("stream_id", id), &st.first, status, f, None);
        }
        self.extra_observe(&st.admission, "run_finished")?;
        Ok(())
    }
}
