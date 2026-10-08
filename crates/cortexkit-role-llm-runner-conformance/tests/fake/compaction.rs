//! Compaction portion of the durable-log fake. Provider/model observations
//! live in World, independently of the log replayed by Module.
use super::*;
use cortexkit_role_llm_runner::read::{EntrySource, ModelEntry, ModelPage};
use cortexkit_role_llm_runner_conformance::{
    CompactionAnswer, CompactionCall, CompactionCallKind, CompactionMessage, CompactionObservation,
    CompactionScript, CompactionSetup, CompactionStep,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompactionDefect {
    #[default]
    None,
    RerunRecordedSetup,
    FoldBeforeAnswer,
    InsertionAfterMessage,
    UnavailableAsRefusal,
    FinerCodeReplacesRoleCode,
    ExceedWaitCap,
    ApplyLate,
    ApplyStale,
    TimeoutEndsUnavailable,
    ApplyTwoViews,
    ForgetRecordedFold,
}

#[derive(Clone, Debug)]
struct Pending {
    index: usize,
    request_id: String,
    deadline: u64,
    setup: bool,
    step: usize,
    consumed: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub setup: Option<CompactionMessage>,
    view: Option<CompactionMessage>,
    fold: Option<CompactionMessage>,
    request: Option<Pending>,
    request_count: usize,
    checked_step: Option<usize>,
    wait: Option<(u64, u64)>,
}

#[derive(Default)]
pub(super) struct Providers {
    sessions: BTreeMap<String, Provider>,
}

struct Provider {
    script: CompactionScript,
    now: u64,
    calls: Vec<CompactionCall>,
    models: Vec<ModelCall>,
}
struct ModelCall {
    turn: usize,
    run: String,
    entries: Vec<ModelEntry>,
    released: bool,
}

fn encode(m: &CompactionMessage) -> Value {
    json!({"compaction_id":m.compaction_id,"version":m.version,"from":m.from,"to":m.to,"replacement":m.replacement})
}
fn decode_message(v: &Value) -> CompactionMessage {
    CompactionMessage {
        compaction_id: v["compaction_id"].as_str().unwrap().into(),
        version: v["version"].as_u64().unwrap(),
        from: v["from"].as_u64().unwrap(),
        to: v["to"].as_u64().unwrap(),
        replacement: v["replacement"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_owned())
            .collect(),
    }
}

pub(super) fn apply(state: &mut State, record: &Value, defect: CompactionDefect) {
    match record["kind"].as_str().unwrap_or_default() {
        "compaction_request" => {
            state.request_count += 1;
            state.request = Some(Pending {
                index: record["index"].as_u64().unwrap() as usize,
                request_id: record["request_id"].as_str().unwrap().into(),
                deadline: record["deadline"].as_u64().unwrap(),
                setup: record["setup"].as_bool().unwrap(),
                step: record["step"].as_u64().unwrap() as usize,
                consumed: false,
            });
        }
        "compaction_message" => {
            let m = decode_message(&record["message"]);
            if record["setup"] == true {
                state.setup = Some(m.clone());
            }
            state.fold = Some(m);
            if let Some(r) = state.request.as_mut() {
                r.consumed = true;
            }
            if record["setup"] != true {
                state.checked_step = Some(record["step"].as_u64().unwrap() as usize);
            }
        }
        "compaction_fold" => {
            state.view = Some(decode_message(&record["message"]));
            state.fold = None;
        }
        "compaction_answer" | "compaction_timeout" => {
            state.checked_step = Some(record["step"].as_u64().unwrap() as usize);
            if let Some(r) = state.request.as_mut() {
                r.consumed = true;
            }
        }
        "compaction_wait" => {
            state.wait = Some((
                record["start"].as_u64().unwrap(),
                record["bound"].as_u64().unwrap(),
            ));
        }
        "compaction_wait_exit" => {
            state.wait = None;
        }
        "terminal" if record.get("error").is_some() => {
            state.request = None;
            state.checked_step = None;
            state.wait = None;
        }
        "resume"
            if state.setup.is_some()
                && state.request.as_ref().is_some_and(|r| r.setup)
                && defect == CompactionDefect::RerunRecordedSetup =>
        {
            state.setup = None;
            state.fold = None;
            state.request = None;
        }
        "resume" if defect == CompactionDefect::ForgetRecordedFold && state.view.is_some() => {
            state.fold = None;
        }
        _ => {}
    }
}

fn canonical(sess: &Sess) -> Vec<ModelEntry> {
    let mut entries = Vec::new();
    let mut from = 0;
    if let Some(m) = &sess.compaction.view {
        from = m.to;
        for text in &m.replacement {
            entries.push(ModelEntry::new(
                EntrySource::Replacement {
                    compaction_id: m.compaction_id.clone(),
                    version: m.version,
                    from_ordinal: m.from,
                    to_ordinal: m.to,
                },
                json!({"role":"system","text":text}),
            ));
        }
    }
    entries.extend(sess.messages.iter().filter(|m| m.ordinal >= from).map(|m| {
        ModelEntry::new(
            EntrySource::Message {
                ordinal: m.ordinal,
                mid: m.mid.clone(),
            },
            m.body.clone(),
        )
    }));
    entries
}

impl Module {
    fn compaction_now(&self, session: &str) -> u64 {
        self.world.compaction.lock().unwrap().sessions[session].now
    }

    fn compaction_error(
        &self,
        session: &str,
        run_id: &str,
        code: &str,
        reason: &str,
        finer: Option<String>,
    ) -> Result<(), Killed> {
        let mut error = json!({"provider_code":code,"provider":"fake.compaction","reason":reason});
        if let Some(finer) = finer {
            if self.world.defects.compaction == CompactionDefect::FinerCodeReplacesRoleCode {
                error["provider_code"] = json!(finer);
            } else {
                error["provider_detail_code"] = json!(finer);
            }
        }
        self.commit(
            session,
            json!({"kind":"terminal","run_id":run_id,"state":"error","error":error}),
        )
    }

    fn issue_compaction(
        &self,
        session: &str,
        run_id: &str,
        setup: bool,
        step: usize,
    ) -> Result<(), Killed> {
        let sess = self.sessions.lock().unwrap()[session].clone();
        let (index, now, timeout, action) = {
            let providers = self.world.compaction.lock().unwrap();
            let p = &providers.sessions[session];
            let action = if setup {
                match &p.script.setup {
                    CompactionSetup::Ready(m) => {
                        CompactionStep::Answer(CompactionAnswer::Message(m.clone()))
                    }
                    CompactionSetup::Refuse {
                        code,
                        reason,
                        provider_code,
                    } => CompactionStep::Answer(CompactionAnswer::Refuse {
                        code: code.clone(),
                        reason: reason.clone(),
                        provider_code: provider_code.clone(),
                    }),
                    CompactionSetup::Unavailable => CompactionStep::Hold(CompactionAnswer::Noop),
                }
            } else {
                p.script
                    .steps
                    .get(step)
                    .cloned()
                    .unwrap_or(CompactionStep::Answer(CompactionAnswer::Noop))
            };
            (p.calls.len(), p.now, p.script.call_timeout_ms, action)
        };
        let id = format!("{}-request-{}", sess.lineage, sess.compaction.request_count);
        self.commit(session,json!({"kind":"compaction_request","index":index,"request_id":id,"setup":setup,"step":step,"deadline":now+timeout}))?;
        self.world
            .compaction
            .lock()
            .unwrap()
            .sessions
            .get_mut(session)
            .unwrap()
            .calls
            .push(CompactionCall {
                kind: if setup {
                    CompactionCallKind::Setup
                } else {
                    CompactionCallKind::Step
                },
                request_id: id.clone(),
                issued_at_ms: now,
                deadline_ms: now + timeout,
            });
        if setup
            && self.world.compaction.lock().unwrap().sessions[session]
                .script
                .setup
                == CompactionSetup::Unavailable
        {
            let code = if self.world.defects.compaction == CompactionDefect::UnavailableAsRefusal {
                cortexkit_role_compaction_provider::errors::RefuseCode::ProviderBusy.as_str()
            } else {
                "compaction_unavailable"
            };
            return self.compaction_error(session, run_id, code, "Setup call failed", None);
        }
        match action {
            CompactionStep::Answer(answer) => self.receive_compaction(session, index, None, answer),
            CompactionStep::Hold(CompactionAnswer::Message(m))
                if self.world.defects.compaction == CompactionDefect::FoldBeforeAnswer =>
            {
                // The provider has only prepared the content. Folding it here
                // deliberately bypasses answer delivery and the request fence.
                self.commit(
                    session,
                    json!({"kind":"compaction_fold","message":encode(&m)}),
                )
            }
            CompactionStep::Hold(_) => Ok(()),
        }
    }

    fn receive_compaction(
        &self,
        session: &str,
        index: usize,
        request: Option<String>,
        answer: CompactionAnswer,
    ) -> Result<(), Killed> {
        let (now, call) = {
            let providers = self.world.compaction.lock().unwrap();
            let p = &providers.sessions[session];
            (p.now, p.calls[index].clone())
        };
        let sess = self.sessions.lock().unwrap()[session].clone();
        let Some(pending) = sess.compaction.request.clone() else {
            return Ok(());
        };
        let id = request.unwrap_or(call.request_id);
        let defect = self.world.defects.compaction;
        let late = now >= call.deadline_ms;
        let stale = id != pending.request_id || index != pending.index;
        let consumed = pending.consumed;
        let allowed = (!late || defect == CompactionDefect::ApplyLate)
            && (!stale || defect == CompactionDefect::ApplyStale)
            && (!consumed
                || (late && defect == CompactionDefect::ApplyLate)
                || defect == CompactionDefect::ApplyTwoViews);
        if !allowed {
            return self.commit(session,json!({"kind":"compaction_discard","request_id":id,"late":late,"stale":stale,"consumed":consumed}));
        }
        let run_id = sess.runs.last().unwrap().run_id.clone();
        match answer {
            CompactionAnswer::Message(m) => {
                if sess
                    .compaction
                    .view
                    .as_ref()
                    .is_some_and(|old| old.version >= m.version)
                {
                    return self.commit(
                        session,
                        json!({"kind":"compaction_answer","step":pending.step}),
                    );
                }
                self.commit(session,json!({"kind":"compaction_message","setup":pending.setup,"step":pending.step,"message":encode(&m)}))?;
                self.commit(
                    session,
                    json!({"kind":"compaction_fold","message":encode(&m)}),
                )
            }
            CompactionAnswer::Noop => self.commit(
                session,
                json!({"kind":"compaction_answer","step":pending.step}),
            ),
            CompactionAnswer::Wait { bound_ms, .. } => self.commit(
                session,
                json!({"kind":"compaction_wait","start":now,"bound":bound_ms}),
            ),
            CompactionAnswer::Refuse {
                code,
                reason,
                provider_code,
            } => self.compaction_error(session, &run_id, code.as_str(), &reason, provider_code),
        }
    }

    /// Drive the actual fake timer/state machine, before any model request.
    pub(super) fn prepare_compaction(
        &self,
        session: &str,
        run_id: &str,
        step: usize,
    ) -> Result<bool, Killed> {
        if !self
            .world
            .compaction
            .lock()
            .unwrap()
            .sessions
            .contains_key(session)
        {
            return Ok(true);
        }
        let mut state = self.sessions.lock().unwrap()[session].compaction.clone();
        if let Some(m) = state.fold.take() {
            self.commit(
                session,
                json!({"kind":"compaction_fold","message":encode(&m)}),
            )?;
        }
        if state.setup.is_none() {
            self.issue_compaction(session, run_id, true, step)?;
            state = self.sessions.lock().unwrap()[session].compaction.clone();
        }
        if self.sessions.lock().unwrap()[session]
            .run(run_id)
            .unwrap()
            .state
            != "active"
        {
            return Ok(false);
        }
        if let Some((start, bound)) = state.wait {
            let (cap, fits) = {
                let providers = self.world.compaction.lock().unwrap();
                let s = &providers.sessions[session].script;
                (
                    s.wait_cap_ms.min(s.prompt_cache_lifetime_ms),
                    s.request_tokens + s.output_limit + s.safety_margin <= s.context_window,
                )
            };
            let elapsed = self.compaction_now(session) - start;
            let effective_cap =
                cap + u64::from(self.world.defects.compaction == CompactionDefect::ExceedWaitCap);
            if elapsed >= effective_cap {
                self.commit(session, json!({"kind":"compaction_wait_exit"}))?;
                if !fits {
                    self.compaction_error(
                        session,
                        run_id,
                        "compaction_wait_exceeded",
                        "request cannot be shown to fit",
                        None,
                    )?;
                    return Ok(false);
                }
                self.commit(session, json!({"kind":"compaction_answer","step":step}))?;
            } else if elapsed >= bound {
                self.commit(session, json!({"kind":"compaction_wait_exit"}))?;
                self.issue_compaction(session, run_id, false, step)?;
                return Ok(false);
            } else {
                return Ok(false);
            }
            state = self.sessions.lock().unwrap()[session].compaction.clone();
        }
        if state.checked_step != Some(step) {
            let outstanding = state
                .request
                .as_ref()
                .filter(|p| !p.setup && p.step == step && !p.consumed);
            if let Some(p) = outstanding {
                if self.compaction_now(session) < p.deadline {
                    return Ok(false);
                }
                if self.world.defects.compaction == CompactionDefect::TimeoutEndsUnavailable {
                    self.compaction_error(
                        session,
                        run_id,
                        "compaction_unavailable",
                        "step timed out",
                        None,
                    )?;
                    return Ok(false);
                }
                self.commit(session, json!({"kind":"compaction_timeout","step":step}))?;
            } else {
                self.issue_compaction(session, run_id, false, step)?;
                return self.prepare_compaction(session, run_id, step);
            }
        }
        // Capture at the model boundary, independently of session.read.
        let sess = self.sessions.lock().unwrap()[session].clone();
        let mut providers = self.world.compaction.lock().unwrap();
        let p = providers.sessions.get_mut(session).unwrap();
        if let Some(call) = p.models.iter().find(|m| m.turn == step && m.run == run_id) {
            return Ok(call.released);
        }
        p.models.push(ModelCall {
            turn: step,
            run: run_id.into(),
            entries: canonical(&sess),
            released: false,
        });
        Ok(false)
    }

    pub(super) fn model_read(
        &self,
        sess: &Sess,
        mode: ReadMode<'_>,
        request: &ReadRequest,
    ) -> Reply {
        if matches!(mode, ReadMode::After { .. }) {
            return invalid("view");
        }
        let entries = canonical(sess);
        let mut groups: Vec<Vec<ModelEntry>> = Vec::new();
        for entry in entries {
            if groups
                .last()
                .is_some_and(|g| g[0].source.from_ordinal() == entry.source.from_ordinal())
            {
                groups.last_mut().unwrap().push(entry);
            } else {
                groups.push(vec![entry]);
            }
        }
        if let ReadMode::Range { from_ordinal } = mode {
            groups.retain(|g| g[0].source.from_ordinal() >= from_ordinal);
        }
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAXIMUM_LIMIT) as usize;
        let cap = request
            .max_bytes
            .unwrap_or(DEFAULT_MAX_BYTES)
            .min(MAXIMUM_MAX_BYTES) as usize;
        let mut chosen = Vec::new();
        let mut count = 0;
        let mut bytes = 0;
        let tail = matches!(mode, ReadMode::Tail);
        if tail {
            groups.reverse();
        }
        for group in &groups {
            let size: usize = group.iter().map(|e| e.message.to_string().len()).sum();
            if !chosen.is_empty() && (count + group.len() > limit || bytes + size > cap) {
                break;
            }
            count += group.len();
            bytes += size;
            chosen.push(group.clone());
        }
        let next = if !tail && chosen.len() < groups.len() {
            Some(groups[chosen.len()][0].source.from_ordinal())
        } else {
            None
        };
        if tail {
            chosen.reverse();
        }
        let mut messages: Vec<_> = chosen.into_iter().flatten().collect();
        if request.limit == Some(1)
            && self.world.defects.compaction == CompactionDefect::InsertionAfterMessage
            && messages.first().is_some_and(|e| e.source.is_insertion())
            && messages.last().is_some_and(|e| !e.source.is_insertion())
        {
            let last = messages.pop().unwrap();
            messages.insert(0, last);
        }
        let mut page = ModelPage::new(&sess.lineage, messages);
        if let Some(view) = &sess.compaction.view {
            page = page.with_compaction(&view.compaction_id, view.version);
        }
        if let Some(next) = next {
            page = page.with_next_from_ordinal(next);
        }
        respond(page)
    }
}

impl FakeSubject {
    pub(super) fn has_compaction_script(&self, session: &str) -> bool {
        self.world()
            .compaction
            .lock()
            .unwrap()
            .sessions
            .contains_key(session)
    }
    pub(super) fn install_compaction(&self, session: &str, script: CompactionScript) {
        let world = self.world();
        let mut providers = world.compaction.lock().unwrap();
        if let Some(p) = providers.sessions.get_mut(session) {
            p.script = script;
        } else {
            providers.sessions.insert(
                session.into(),
                Provider {
                    script,
                    now: 0,
                    calls: Vec::new(),
                    models: Vec::new(),
                },
            );
        }
    }
    pub(super) fn observe_compaction(&self, session: &str) -> CompactionObservation {
        let world = self.world();
        let providers = world.compaction.lock().unwrap();
        let p = &providers.sessions[session];
        CompactionObservation {
            calls: p.calls.clone(),
            model_inputs: p.models.iter().map(|m| m.entries.clone()).collect(),
        }
    }
    fn compaction_modules(&self, session: &str) -> Vec<Arc<Module>> {
        self.modules
            .lock()
            .unwrap()
            .iter()
            .filter(|m| {
                m.alive.load(Ordering::SeqCst) && m.sessions.lock().unwrap().contains_key(session)
            })
            .cloned()
            .collect()
    }
    fn drive_compaction_modules(&self, session: &str) -> Result<(), HarnessError> {
        for m in self.compaction_modules(session) {
            let run = m.sessions.lock().unwrap()[session]
                .runs
                .last()
                .unwrap()
                .run_id
                .clone();
            m.drive(session, &run)
                .map_err(|Killed| HarnessError::new("killed during compaction"))?;
        }
        Ok(())
    }
    pub(super) fn advance_compaction(&self, session: &str, ms: u64) -> Result<(), HarnessError> {
        self.world()
            .compaction
            .lock()
            .unwrap()
            .sessions
            .get_mut(session)
            .unwrap()
            .now += ms;
        self.drive_compaction_modules(session)
    }
    pub(super) fn deliver_compaction(
        &self,
        session: &str,
        index: usize,
        request: Option<String>,
        answer: CompactionAnswer,
    ) -> Result<(), HarnessError> {
        for m in self.compaction_modules(session) {
            m.receive_compaction(session, index, request.clone(), answer.clone())
                .map_err(|Killed| HarnessError::new("killed after answer"))?;
        }
        self.drive_compaction_modules(session)
    }
    pub(super) fn release_model(&self, session: &str) -> Result<(), HarnessError> {
        {
            let world = self.world();
            let mut providers = world.compaction.lock().unwrap();
            let p = providers.sessions.get_mut(session).unwrap();
            let call = p
                .models
                .iter_mut()
                .find(|m| !m.released)
                .ok_or_else(|| HarnessError::new("no held model call"))?;
            call.released = true;
        }
        self.drive_compaction_modules(session)
    }
    pub(super) fn durable_setup(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<Option<CompactionMessage>, HarnessError> {
        let contents =
            std::fs::read_to_string(&handle.0.log).map_err(|e| HarnessError::new(e.to_string()))?;
        Ok(contents
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .rfind(|r| {
                r["session"] == session && r["kind"] == "compaction_message" && r["setup"] == true
            })
            .map(|r| decode_message(&r["message"])))
    }
}
