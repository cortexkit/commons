//! Compaction portion of the durable-log fake. World stores provider and model
//! observations outside Module's persistent log, so Module's restart cannot
//! reconstruct or erase those observations.
use super::*;
use cortexkit_role_llm_runner::read::{EntrySource, ModelEntry, ModelPage};
use cortexkit_role_llm_runner_conformance::{
    CompactionCall, CompactionMessage, CompactionObservation, CompactionRouteIdentity,
    CompactionSetup, CompactionSetupIdentity,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompactionDefect {
    #[default]
    None,
    RerunRecordedSetup,
    RerunSetupOnResumeSend,
    ChangeSetupResumeView,
    CancelSetupCrash,
    InsertionAfterMessage,
    UnavailableAsRefusal,
    FinerCodeReplacesRoleCode,
    BindWrongSession,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompactionRouteMode {
    #[default]
    SingleSession,
    SharedProject,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub setup: Option<CompactionMessage>,
    view: Option<CompactionMessage>,
    fold: Option<CompactionMessage>,
    /// The runner issued its Setup call, and no error ended its run since.
    setup_issued: bool,
    request_count: usize,
    sealed_setup_crash: bool,
}

#[derive(Default)]
pub(super) struct Providers {
    pub seal_setup_crash: bool,
    sessions: BTreeMap<String, Provider>,
}

impl Providers {
    pub(super) fn new(seal_setup_crash: bool) -> Self {
        Self {
            seal_setup_crash,
            ..Self::default()
        }
    }
}

struct Provider {
    setup: CompactionSetup,
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
            state.setup_issued = true;
        }
        "compaction_message" => {
            let m = decode_message(&record["message"]);
            state.setup = Some(m.clone());
            state.fold = Some(m);
        }
        "compaction_fold" => {
            state.view = Some(decode_message(&record["message"]));
            state.fold = None;
        }
        "terminal" if record.get("error").is_some() => {
            state.setup_issued = false;
        }
        "resume"
            if state.setup.is_some()
                && state.setup_issued
                && defect == CompactionDefect::RerunRecordedSetup =>
        {
            state.setup = None;
            state.fold = None;
            state.setup_issued = false;
        }
        "terminal" if record["state"] == "interrupted" && state.setup_issued => {
            state.sealed_setup_crash = true;
        }
        "send" if state.sealed_setup_crash => {
            state.sealed_setup_crash = false;
            match defect {
                CompactionDefect::RerunSetupOnResumeSend => {
                    state.setup = None;
                    state.setup_issued = false;
                }
                CompactionDefect::ChangeSetupResumeView => {
                    state.view.as_mut().unwrap().replacement[1] = "changed-kept-summary".into();
                }
                _ => {}
            }
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
    pub(super) fn seal_compaction_setup_crash(
        &self,
        session: &str,
        run_id: &str,
    ) -> Result<bool, Killed> {
        let state = self.sessions.lock().unwrap()[session].compaction.clone();
        if !self.world.compaction.lock().unwrap().seal_setup_crash
            || state.setup.is_none()
            || state.view.is_none()
            || !state.setup_issued
        {
            return Ok(false);
        }
        let state = if self.world.defects.compaction == CompactionDefect::CancelSetupCrash {
            "cancelled"
        } else {
            "interrupted"
        };
        self.commit(
            session,
            json!({"kind":"terminal", "run_id":run_id, "state":state}),
        )?;
        Ok(true)
    }

    fn compaction_error(
        &self,
        session: &str,
        run_id: &str,
        code: &str,
        reason: &str,
        finer: Option<String>,
    ) -> Result<(), Killed> {
        let mut error = json!({"class":"permanent","message":reason,"provider_code":code,"provider":"fake.compaction","reason":reason});
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

    /// Call Setup on the session's scripted provider and record its answer.
    fn issue_setup(&self, session: &str, run_id: &str) -> Result<(), Killed> {
        let sess = self.sessions.lock().unwrap()[session].clone();
        let setup = self.world.compaction.lock().unwrap().sessions[session]
            .setup
            .clone();
        let id = format!("{}-request-{}", sess.lineage, sess.compaction.request_count);
        self.commit(
            session,
            json!({"kind":"compaction_request","request_id":id}),
        )?;
        let route_session = match self.world.compaction_route_mode {
            CompactionRouteMode::SingleSession
                if self.world.defects.compaction == CompactionDefect::BindWrongSession =>
            {
                format!("wrong-{session}")
            }
            CompactionRouteMode::SingleSession => session.to_owned(),
            CompactionRouteMode::SharedProject => String::new(),
        };
        self.world
            .compaction
            .lock()
            .unwrap()
            .sessions
            .get_mut(session)
            .unwrap()
            .calls
            .push(CompactionCall {
                request_id: id.clone(),
                setup_identity: CompactionSetupIdentity {
                    harness: "broca".into(),
                    session: session.to_owned(),
                },
                route_identity: CompactionRouteIdentity {
                    harness: "runner".into(),
                    session: route_session,
                    project_root: "/fake/project".into(),
                },
            });
        match setup {
            CompactionSetup::Unavailable => {
                let code = if self.world.defects.compaction
                    == CompactionDefect::UnavailableAsRefusal
                {
                    cortexkit_role_compaction_provider::errors::RefuseCode::ProviderBusy.as_str()
                } else {
                    "compaction_unavailable"
                };
                self.compaction_error(session, run_id, code, "Setup call failed", None)
            }
            CompactionSetup::Refuse {
                code,
                reason,
                provider_code,
            } => self.compaction_error(session, run_id, code.as_str(), &reason, provider_code),
            CompactionSetup::Ready(m) => {
                self.commit(
                    session,
                    json!({"kind":"compaction_message","setup":true,"message":encode(&m)}),
                )?;
                self.commit(
                    session,
                    json!({"kind":"compaction_fold","message":encode(&m)}),
                )
            }
        }
    }

    /// Before any model request: fold a recorded Setup answer, call Setup if
    /// none is recorded, then capture the request at the scripted model and
    /// hold it until released.
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
        let state = self.sessions.lock().unwrap()[session].compaction.clone();
        if let Some(m) = state.fold {
            self.commit(
                session,
                json!({"kind":"compaction_fold","message":encode(&m)}),
            )?;
        }
        if state.setup.is_none() {
            self.issue_setup(session, run_id)?;
        }
        if self.sessions.lock().unwrap()[session]
            .run(run_id)
            .unwrap()
            .state
            != "active"
        {
            return Ok(false);
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
    pub(super) fn install_compaction(&self, session: &str, setup: CompactionSetup) {
        let world = self.world();
        let mut providers = world.compaction.lock().unwrap();
        if let Some(p) = providers.sessions.get_mut(session) {
            p.setup = setup;
        } else {
            providers.sessions.insert(
                session.into(),
                Provider {
                    setup,
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
