//! Compaction portion of the durable-log fake. World stores provider and model
//! observations outside Module's persistent log, so Module's restart cannot
//! reconstruct or erase those observations.
use super::*;
use cortexkit_role_compaction_provider::errors::RefuseCode;
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
    /// A Setup refused `setup_identity_mismatch` is recorded under another
    /// code (`misconfigured`).
    IdentityMismatchUnderAnotherCode,
    StandaloneLateHandoff,
    StandaloneNoHandoff,
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
    /// The Setup checkpoint `(tag_checkpoint, cut)` the provider answered,
    /// if any.
    pub checkpoint: Option<(u64, String)>,
    /// The lineage the recorded Setup answer belongs to.
    pub setup_lineage: Option<String>,
    /// The view the runner renders now, once one applied on this lineage.
    pub applied: Option<history::Applied>,
    /// Every view version the runner fetched and disposed of.
    pub disposed: BTreeSet<u64>,
    /// Every `cache_ttl` entry the provider answered, oldest first.
    pub cache_ttl: Vec<cortexkit_role_compaction_provider::cache_ttl::CacheTtl>,
    /// How long the latest Setup answer lets a step wait for a rewrite
    /// ride's answer, when it says.
    pub ride_wait_ms: Option<u64>,
}

/// A rewrite ride a step waits on.
#[derive(Clone, Debug)]
pub(super) struct PendingRide {
    run_id: String,
    step: usize,
    /// The notice events computed for the step, which its notice carries
    /// once the ride resolves.
    events: Vec<String>,
    /// When the ride was sent, on the ride clock.
    started: u64,
    /// How long the runner waits for the answer.
    deadline_ms: u64,
    /// When that wait ends, in milliseconds since the Unix epoch, as
    /// `deadline_ms` reports it.
    ends_at: u64,
    script: cortexkit_role_llm_runner_conformance::ScriptedRide,
    /// Whether the wait was reported `compaction_wait`.
    reported: bool,
}

#[derive(Default)]
pub(super) struct Providers {
    pub seal_setup_crash: bool,
    pub(super) sessions: BTreeMap<String, Provider>,
}

impl Providers {
    pub(super) fn new(seal_setup_crash: bool) -> Self {
        Self {
            seal_setup_crash,
            ..Self::default()
        }
    }
}

pub(super) struct Provider {
    handoffs: usize,
    setup: CompactionSetup,
    calls: Vec<CompactionCall>,
    models: Vec<ModelCall>,
    /// Every view published, by version.
    pub views: Vec<cortexkit_role_compaction_provider::view::CompactionView>,
    /// Every advance notice received, as its bus headers.
    pub notices: Vec<BTreeMap<String, String>>,
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
            state.setup_lineage = record["lineage"].as_str().map(str::to_owned);
            state.setup = Some(m.clone());
            state.fold = Some(m);
            if let Ok(entry) = serde_json::from_value(record["cache_ttl"].clone()) {
                state.cache_ttl.push(entry);
            }
            state.ride_wait_ms = record["ride_wait_ms"].as_u64();
            if let (Some(n), Some(cut)) = (
                record["checkpoint"]["tag_checkpoint"].as_u64(),
                record["checkpoint"]["cut"].as_str(),
            ) {
                state.checkpoint = Some((n, cut.to_owned()));
            }
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

pub(super) fn canonical(sess: &Sess) -> Vec<ModelEntry> {
    if let Some(applied) = &sess.compaction.applied {
        return history::render_view(sess, applied);
    }
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
    let from = from.max(sess.first_ordinal);
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
                lineage_id: Some(sess.lineage.clone()),
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
            } => {
                if code == RefuseCode::StandaloneSessionNotMigrated
                    && self.world.standalone_in_process
                    && (!sess.ever_served
                        || self.world.defects.compaction == CompactionDefect::StandaloneLateHandoff)
                    && self.world.defects.compaction != CompactionDefect::StandaloneNoHandoff
                {
                    self.world
                        .compaction
                        .lock()
                        .unwrap()
                        .sessions
                        .get_mut(session)
                        .unwrap()
                        .handoffs += 1;
                    return self.commit(session, json!({"kind":"standalone_handoff"}));
                }
                let code = if code == RefuseCode::SetupIdentityMismatch
                    && self.world.defects.compaction
                        == CompactionDefect::IdentityMismatchUnderAnotherCode
                {
                    RefuseCode::Misconfigured
                } else {
                    code
                };
                self.compaction_error(session, run_id, code.as_str(), &reason, provider_code)
            }
            CompactionSetup::Ready(m) => {
                self.commit(
                    session,
                    json!({"kind":"compaction_message","setup":true,"message":encode(&m),
                           "lineage":sess.lineage}),
                )?;
                self.commit(
                    session,
                    json!({"kind":"compaction_fold","message":encode(&m)}),
                )
            }
            CompactionSetup::ReadyWithCacheTtl {
                initial: m,
                cache_ttl,
            } => {
                self.commit(
                    session,
                    json!({"kind":"compaction_message","setup":true,"message":encode(&m),
                           "lineage":sess.lineage, "cache_ttl": cache_ttl}),
                )?;
                self.commit(
                    session,
                    json!({"kind":"compaction_fold","message":encode(&m)}),
                )
            }
            CompactionSetup::ReadyWithRideWait {
                initial: m,
                cache_ttl,
                ride_wait_ms,
            } => {
                // A wait above the cap the Setup request carried, or none
                // at all, fails Setup, naming the member.
                if (ride_wait_ms == 0 || ride_wait_ms > RIDE_WAIT_MAX_MS)
                    && !self.world.defects.ride_wait_cap_ignored
                {
                    return self.compaction_error(
                        session,
                        run_id,
                        "compaction_unavailable",
                        "ride_wait_ms is 0 or above the ride_wait_max_ms of the Setup request",
                        None,
                    );
                }
                self.commit(
                    session,
                    json!({"kind":"compaction_message","setup":true,"message":encode(&m),
                           "lineage":sess.lineage, "cache_ttl": cache_ttl,
                           "ride_wait_ms": ride_wait_ms}),
                )?;
                self.commit(
                    session,
                    json!({"kind":"compaction_fold","message":encode(&m)}),
                )
            }
            // A checkpoint on a Setup that descends from another lineage
            // (a cut's) fails that Setup: such a lineage continues the old
            // lineage's numbering.
            CompactionSetup::ReadyWithCheckpoint { .. }
                if sess.pending_descent.is_some()
                    && self.world.defects.lineage
                        != history::LineageDefect::CheckpointOnDescentApplied =>
            {
                self.compaction_error(
                    session,
                    run_id,
                    "compaction_unavailable",
                    cortexkit_role_compaction_provider::setup::CHECKPOINT_ON_DESCENT,
                    None,
                )
            }
            CompactionSetup::ReadyWithCheckpoint {
                initial: m,
                tag_checkpoint,
            } => {
                // The cut is the newest message when Setup is called: the
                // session's first, its prompt.
                let cut = sess
                    .messages
                    .last()
                    .map(|m| m.mid.clone())
                    .unwrap_or_default();
                self.commit(
                    session,
                    json!({"kind":"compaction_message","setup":true,"message":encode(&m),
                           "checkpoint":{"tag_checkpoint":tag_checkpoint,"cut":cut},
                           "lineage":sess.lineage}),
                )?;
                self.commit(
                    session,
                    json!({"kind":"compaction_fold","message":encode(&m)}),
                )
            }
            // An answer kind this fake does not know fails Setup, as a
            // runner treats an answer it cannot decode.
            _ => self.compaction_error(
                session,
                run_id,
                "compaction_unavailable",
                "Setup answer of an unknown kind",
                None,
            ),
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
        if self.sessions.lock().unwrap()[session].standalone_handed_off {
            return Ok(true);
        }
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
        {
            // A step whose model request was already captured is past its
            // boundary: nothing applies or is set up again for it.
            let providers = self.world.compaction.lock().unwrap();
            let p = &providers.sessions[session];
            if let Some(call) = p.models.iter().find(|m| m.turn == step && m.run == run_id) {
                return Ok(call.released);
            }
        }
        // The step boundary: a fetched view applies now, and a view with a
        // cut starts a new lineage.
        self.dispose_views(session)?;
        let state = self.sessions.lock().unwrap()[session].compaction.clone();
        if let Some(m) = state.fold {
            self.commit(
                session,
                json!({"kind":"compaction_fold","message":encode(&m)}),
            )?;
        }
        let lineage = self.sessions.lock().unwrap()[session].lineage.clone();
        let other_lineage = state.setup_lineage.as_ref().is_some_and(|l| *l != lineage)
            && self.world.defects.lineage != history::LineageDefect::SetupOncePerSession;
        if state.setup.is_none() || other_lineage {
            if history::setup_held(&self.world, session) {
                return Ok(false);
            }
            self.issue_setup(session, run_id)?;
        }
        if self.sessions.lock().unwrap()[session].standalone_handed_off {
            return Ok(true);
        }
        if self.sessions.lock().unwrap()[session]
            .run(run_id)
            .unwrap()
            .state
            != "active"
        {
            return Ok(false);
        }
        // A rewrite ride in flight holds the step until it resolves.
        if self.world.rides.lock().unwrap().contains_key(session) {
            return Ok(false);
        }
        // Capture at the model boundary, independently of session.read.
        let mut sess = self.sessions.lock().unwrap()[session].clone();
        let resolved = {
            let mut resolved = self.world.resolved_rides.lock().unwrap();
            match resolved.get(session) {
                Some((run, at, _)) if run == run_id && *at == step => {
                    resolved.remove(session).map(|(_, _, events)| events)
                }
                _ => None,
            }
        };
        let events = match resolved {
            Some(events) => events,
            None => {
                let events = self.cache_events(session, &sess.compaction.cache_ttl);
                let rewrite = events.iter().any(|e| {
                    matches!(
                        e.as_str(),
                        "ttl_expired" | "model_change" | "surface_change"
                    )
                });
                let script = rewrite
                    .then(|| self.world.ride_scripts.lock().unwrap().remove(session))
                    .flatten();
                if let Some(script) = script {
                    self.start_ride(
                        session,
                        run_id,
                        step,
                        events,
                        script,
                        sess.compaction.ride_wait_ms,
                    )?;
                    return Ok(false);
                }
                events
            }
        };
        sess.notice_events.extend(events);
        self.publish_notice(session, &sess)?;
        let mut providers = self.world.compaction.lock().unwrap();
        let p = providers.sessions.get_mut(session).unwrap();
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
        if let Some(applied) = &sess.compaction.applied {
            page = page.with_compaction(
                history::view_compaction_id(applied.version),
                applied.version,
            );
        } else if let Some(view) = &sess.compaction.view {
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
                    handoffs: 0,
                    setup,
                    calls: Vec::new(),
                    models: Vec::new(),
                    views: Vec::new(),
                    notices: Vec::new(),
                },
            );
        }
    }
    /// Whether the session's scripted provider has received a Setup call.
    pub(super) fn setup_received(&self, session: &str) -> bool {
        self.world()
            .compaction
            .lock()
            .unwrap()
            .sessions
            .get(session)
            .is_some_and(|provider| !provider.calls.is_empty())
    }
    pub(super) fn observe_compaction(&self, session: &str) -> CompactionObservation {
        let world = self.world();
        let providers = world.compaction.lock().unwrap();
        let p = &providers.sessions[session];
        CompactionObservation {
            standalone_handoffs: p.handoffs,
            calls: p.calls.clone(),
            model_inputs: p.models.iter().map(|m| m.entries.clone()).collect(),
            notices: p.notices.clone(),
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
    pub(super) fn drive_compaction_modules(&self, session: &str) -> Result<(), HarnessError> {
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

/// How the fake decides the prompt cache's lifetime: correctly, or by one
/// of the rules the `cache_ttl` cases must catch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CacheTtlDefect {
    #[default]
    None,
    /// The newest entry is used whatever model it names.
    IgnoreModel,
    /// A configured entry whose `ms` is `null` is read as no entry.
    NullAsAbsent,
    /// After a model switch with no entry for the new model, the lifetime
    /// judged for the previous model is kept.
    KeepPrevious,
}

/// One session's prompt-cache clock, as the adapter's hook moves it.
#[derive(Clone, Debug, Default)]
pub(super) struct CacheClock {
    pub now: u64,
    last_request: Option<u64>,
    last_model: Option<String>,
    last_lifetime: Option<u64>,
}

impl Module {
    /// The model the session's requests go to now.
    pub(super) fn session_model(&self, session: &str) -> String {
        self.world
            .models
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_else(|| FAKE_MODEL.to_owned())
    }

    /// Send the step's rewrite ride: the step waits on it until the
    /// scripted answer arrives or the ride's deadline passes
    /// ([`Module::tick_ride`]).
    fn start_ride(
        &self,
        session: &str,
        run_id: &str,
        step: usize,
        events: Vec<String>,
        script: cortexkit_role_llm_runner_conformance::ScriptedRide,
        ride_wait_ms: Option<u64>,
    ) -> Result<(), Killed> {
        let deadline_ms = match ride_wait_ms {
            Some(ms) if !self.world.defects.ride_wait_fixed_deadline => ms,
            _ => OWN_RIDE_DEADLINE_MS,
        };
        let mut ride = PendingRide {
            run_id: run_id.to_owned(),
            step,
            events,
            started: self.world.ride_clock.load(Ordering::SeqCst),
            deadline_ms,
            ends_at: self.world.now() + deadline_ms,
            script,
            reported: false,
        };
        if self.world.defects.ride_wait_reported_at_once {
            self.commit(
                session,
                json!({"kind":"compaction_wait","run_id":run_id,"deadline_ms":ride.ends_at}),
            )?;
            ride.reported = true;
        }
        self.world
            .rides
            .lock()
            .unwrap()
            .insert(session.to_owned(), ride);
        Ok(())
    }

    /// Move `session`'s pending rewrite ride on by the ride clock: report it
    /// `compaction_wait` once its answer has been pending for 1 s, and
    /// resolve it when the scripted answer arrives within the deadline (a
    /// view is published, and applies at the step) or the deadline passes
    /// (the step goes on without it). A resolved ride's step then goes on.
    pub(super) fn tick_ride(&self, session: &str) -> Result<(), Killed> {
        let Some(mut ride) = self.world.rides.lock().unwrap().get(session).cloned() else {
            return Ok(());
        };
        let elapsed = self.world.ride_clock.load(Ordering::SeqCst) - ride.started;
        let answer_after = ride.script.answer_after_ms;
        let answered = answer_after <= ride.deadline_ms && elapsed >= answer_after;
        if answered || elapsed >= ride.deadline_ms {
            self.world.rides.lock().unwrap().remove(session);
            if let Some(view) = ride.script.view.clone().filter(|_| answered) {
                history::publish(&self.world, session, view);
            }
            if ride.reported {
                self.commit(
                    session,
                    json!({"kind":"ride_resolved","run_id":ride.run_id}),
                )?;
            }
            self.world.resolved_rides.lock().unwrap().insert(
                session.to_owned(),
                (ride.run_id.clone(), ride.step, ride.events),
            );
            return self.drive(session, &ride.run_id);
        }
        if !ride.reported
            && !self.world.defects.ride_wait_never_reported
            && elapsed >= cortexkit_role_llm_runner::COMPACTION_WAIT_REPORT_AFTER_MS
        {
            self.commit(
                session,
                json!({"kind":"compaction_wait","run_id":ride.run_id,"deadline_ms":ride.ends_at}),
            )?;
            ride.reported = true;
            self.world
                .rides
                .lock()
                .unwrap()
                .insert(session.to_owned(), ride);
        }
        Ok(())
    }

    /// The notice events a model request about to go out declares for the
    /// prompt cache: `model_change` after a switch, and `ttl_expired` when
    /// the cache idled at least as long as the lifetime judged for the
    /// request's model.
    fn cache_events(
        &self,
        session: &str,
        entries: &[cortexkit_role_compaction_provider::cache_ttl::CacheTtl],
    ) -> Vec<String> {
        use cortexkit_role_compaction_provider::cache_ttl::lifetime;
        let model = self.session_model(session);
        let observed = self
            .world
            .observed_cache_lifetimes
            .lock()
            .unwrap()
            .get(session)
            .copied()
            .flatten();
        let mut clocks = self.world.cache_clocks.lock().unwrap();
        let clock = clocks.entry(session.to_owned()).or_default();
        let switched = clock.last_model.as_ref().is_some_and(|last| *last != model);
        let judged = match self.world.cache_ttl_defect {
            CacheTtlDefect::None => lifetime(&model, entries, observed),
            CacheTtlDefect::IgnoreModel => {
                let renamed: Vec<_> = entries
                    .iter()
                    .map(|e| {
                        cortexkit_role_compaction_provider::cache_ttl::CacheTtl::new(
                            &model,
                            e.ms,
                            e.configured,
                        )
                    })
                    .collect();
                lifetime(&model, &renamed, observed)
            }
            CacheTtlDefect::NullAsAbsent => {
                let kept: Vec<_> = entries
                    .iter()
                    .filter(|e| !(e.configured && e.ms.is_none()))
                    .cloned()
                    .collect();
                lifetime(&model, &kept, observed)
            }
            CacheTtlDefect::KeepPrevious
                if switched && !entries.iter().any(|e| e.model == model) =>
            {
                clock.last_lifetime
            }
            CacheTtlDefect::KeepPrevious => lifetime(&model, entries, observed),
        };
        let idle = clock.last_request.map(|last| clock.now - last);
        let mut events = Vec::new();
        if switched {
            events.push("model_change".to_owned());
        }
        if idle.zip(judged).is_some_and(|(idle, ms)| idle >= ms) {
            events.push("ttl_expired".to_owned());
        }
        clock.last_request = Some(clock.now);
        clock.last_model = Some(model);
        clock.last_lifetime = judged;
        events
    }
}
