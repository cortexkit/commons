//! A durable fork log separate from the fake head log. Model-ingress recordings
//! live outside both logs and are never used to answer management reads.
use super::*;
use cortexkit_role_llm_runner::{content_digest::sha256_hex, fork_answer::plan_callers};
use cortexkit_role_llm_runner_conformance::{RawModelRequestObservation, ScriptedTurn};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForkDefect {
    #[default]
    None,
    Base,
    BaseBytes,
    RelocatedMarker,
    Tools,
    Failure,
    Replay,
    Expiry,
    Head,
    Callers,
    Account,
    HeldMapping,
    BlockedMapping,
    Key,
    Restart,
}

#[derive(Default)]
pub(super) struct Ingress {
    pub raw: BTreeMap<String, Vec<RawModelRequestObservation>>,
    pub head_keys: BTreeMap<String, String>,
    pub head_tiers: BTreeMap<String, String>,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Store {
    next_id: u64,
    sessions: BTreeMap<String, Session>,
}

#[derive(Default, Serialize, Deserialize)]
struct Session {
    now: u64,
    head: Option<Value>,
    callers: Vec<String>,
    forks: BTreeMap<String, Fork>,
}

#[derive(Serialize, Deserialize)]
struct Fork {
    principal: String,
    base: Value,
    mids: Vec<String>,
    branch: Vec<Value>,
    deadline: u64,
    ended: bool,
    revoked: bool,
    answers: BTreeMap<String, (String, Value)>,
    attempts: BTreeMap<String, u64>,
    pending: Option<Pending>,
}

#[derive(Serialize, Deserialize)]
struct Pending {
    id: String,
    question: String,
    answer: Value,
    admission: Option<String>,
}

impl Store {
    pub fn load(root: &Path) -> Self {
        std::fs::read(root.join("fork-log.json"))
            .ok()
            .map(|b| serde_json::from_slice(&b).expect("valid fake fork log"))
            .unwrap_or_default()
    }
    fn save(&self, module: &Module) {
        std::fs::write(
            module.log.with_file_name("fork-log.json"),
            serde_json::to_vec(self).unwrap(),
        )
        .unwrap();
    }
}

fn broken(module: &Module, defect: ForkDefect) -> bool {
    module.world.defects.fork == defect
}

pub(super) fn capture_head(module: &Module, session: &str, mut body: Value) {
    if broken(module, ForkDefect::RelocatedMarker) {
        for value in body["messages"].as_array_mut().unwrap() {
            *value = message(value.clone(), true);
        }
    }
    module
        .world
        .fork_ingress
        .lock()
        .unwrap()
        .raw
        .entry(session.into())
        .or_default()
        .push(RawModelRequestObservation::new(
            None,
            serde_json::to_vec(&body).unwrap(),
        ));
    let mut store = module.forks.lock().unwrap();
    store.sessions.entry(session.into()).or_default().head = Some(body);
    store.save(module);
}

pub(super) fn advance(module: &Module, session: &str, ms: u64) {
    if broken(module, ForkDefect::Expiry) {
        return;
    }
    let mut store = module.forks.lock().unwrap();
    store.sessions.entry(session.into()).or_default().now += ms;
    store.save(module);
}

pub(super) fn validate_plan(method: &str, params: &Value) -> Option<Reply> {
    if !matches!(method, ops::SESSION_SEND | ops::SESSION_REFRESH) {
        return None;
    }
    let plan = params.get("plan")?.as_object()?;
    plan_callers(plan).err().map(invalid)
}

pub(super) fn apply_plan(route: &FakeRoute, method: &str, params: &Value) {
    if !matches!(method, ops::SESSION_SEND | ops::SESSION_REFRESH) {
        return;
    }
    let Some(plan) = params.get("plan").and_then(Value::as_object) else {
        return;
    };
    let callers = plan_callers(plan).unwrap();
    let mut store = route.module.forks.lock().unwrap();
    let session = store.sessions.entry(route.session.clone()).or_default();
    if !broken(&route.module, ForkDefect::Callers) {
        for fork in session.forks.values_mut() {
            if !callers.contains(&fork.principal) {
                fork.revoked = true;
            }
        }
        session.callers = callers;
    }
    store.save(&route.module);
}

fn usage(turn: &ScriptedTurn) -> Value {
    match &turn.usage {
        Some(u) => {
            json!({"steps":1,"unreported_steps":0,"input_tokens":u.input_tokens,"cached_input_tokens":u.cache_read_tokens,"output_tokens":u.output_tokens,"by_model":[{"provider":FAKE_PROVIDER,"model":FAKE_MODEL,"steps":1,"unreported_steps":0,"input_tokens":u.input_tokens,"cached_input_tokens":u.cache_read_tokens,"output_tokens":u.output_tokens}]})
        }
        None => json!({"steps":1,"unreported_steps":1,"by_model":[]}),
    }
}

fn release(module: &Module, session: &str, admission: &Option<String>) {
    if let (Some(peer), Some(id)) = (
        module.world.routing_peers.lock().unwrap().get(session),
        admission,
    ) {
        peer.request(
            "fake-runner",
            "route.account_observe",
            json!({"event_id":format!("{id}/finish"),"kind":"run_finished","admission_id":id}),
        )
        .unwrap();
    }
}

fn finish(
    module: &Module,
    session_name: &str,
    session: &mut Session,
    id: &str,
    recovery: bool,
) -> Reply {
    let fork = session.forks.get_mut(id).unwrap();
    let pending = fork.pending.take().unwrap();
    let mut answer = pending.answer;
    if recovery {
        answer["answer"] = json!("");
        answer["finish"] = json!("model_error");
    }
    if answer["finish"] != "model_error" || broken(module, ForkDefect::Failure) {
        fork.branch.push(json!(pending.question));
        fork.branch.push(answer["answer"].clone());
    }
    if answer["finish"] == "tool_call_refused" {
        fork.ended = true;
    }
    fork.deadline = session.now + 2000;
    fork.answers
        .insert(pending.id, (pending.question, answer.clone()));
    release(module, session_name, &pending.admission);
    if fork.revoked {
        return refuse(errors::UNKNOWN_FORK, None);
    }
    respond(answer)
}

pub(super) fn recover(module: &Module) {
    let mut store = module.forks.lock().unwrap();
    for (name, session) in &mut store.sessions {
        if broken(module, ForkDefect::Restart) {
            session.forks.clear();
            continue;
        }
        let pending: Vec<_> = session
            .forks
            .iter()
            .filter(|(_, f)| f.pending.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        if !pending.is_empty() {
            module.world.held_models.lock().unwrap().remove(name);
        }
        for id in pending {
            finish(module, name, session, &id, true);
        }
    }
    store.save(module);
}

fn hash(value: Value) -> String {
    // These test inputs contain only strings and integers. BTreeMap key order
    // gives the canonical ordering independently of the kit's RFC 8785
    // JSON Canonicalization Scheme serializer.
    format!(
        "fork:{}",
        sha256_hex(serde_json::to_string(&value).unwrap().as_bytes())
    )
}

fn admission(
    module: &Module,
    session: &str,
    fork: &str,
    id: &str,
    attempt: u64,
) -> Result<Option<String>, Reply> {
    let peers = module.world.routing_peers.lock().unwrap();
    let Some(peer) = peers.get(session) else {
        return Ok(None);
    };
    let calls = peer.calls();
    let head = calls
        .iter()
        .find(|c| c.method == "route.account_for")
        .unwrap();
    let mut body = head.body.clone();
    let mut conversation = hash(json!({"fork_id":fork,"head_key":head.body["conversation_key"]}));
    if broken(module, ForkDefect::Key) {
        conversation = format!("{}:{fork}", head.body["conversation_key"].as_str().unwrap());
    }
    let admission = hash(json!({"ask_id":id,"attempt":attempt,"fork_id":fork}));
    body["conversation_key"] = json!(conversation);
    body["admission_id"] = json!(admission);
    body["hold_wait_max_s"] = json!(0);
    body["requested_account"] = json!({"account_id":peer.fixture()["account_id"],"credential_id":peer.fixture()["credential_id"]});
    let answer = peer
        .request("fake-runner", "route.account_for", body)
        .unwrap();
    if let Some(error) = answer.get("error") {
        let code = error["code"].as_str().unwrap();
        let held = matches!(
            code,
            "account_hold" | "account_pool_exhausted" | "account_selection_unavailable"
        );
        let wrong = broken(
            module,
            if held {
                ForkDefect::HeldMapping
            } else {
                ForkDefect::BlockedMapping
            },
        );
        return Err(refuse(
            if held != wrong {
                errors::FORK_ACCOUNT_HELD
            } else {
                errors::FORK_ACCOUNT_BLOCKED
            },
            Some(json!({"routing_code":code})),
        ));
    }
    let account = &answer["result"]["account"];
    let mismatch = if account["account_id"] != peer.fixture()["account_id"] {
        Some("account")
    } else if account["credential_id"] != peer.fixture()["credential_id"] {
        Some("credential")
    } else if answer["result"]
        .get("service_tier_chosen")
        .unwrap_or(&json!("standard"))
        != head.body.get("service_tier").unwrap_or(&json!("standard"))
    {
        Some("service_tier")
    } else {
        None
    };
    if let Some(mismatch) = mismatch.filter(|_| !broken(module, ForkDefect::Account)) {
        peer.request("fake-runner","route.account_observe",json!({"event_id":format!("{admission}/finish"),"kind":"run_finished","admission_id":admission})).unwrap();
        return Err(refuse(
            errors::FORK_ACCOUNT_HELD,
            Some(json!({"mismatch":mismatch})),
        ));
    }
    Ok(Some(admission))
}

pub(super) async fn request(
    route: &FakeRoute,
    method: &str,
    params: Value,
) -> Result<Reply, RouteFailure> {
    let result = begin(route, method, &params);
    let Some(fork_id) = params["fork_id"].as_str() else {
        return Ok(result);
    };
    if result != respond(json!({"waiting":true})) {
        return Ok(result);
    }
    loop {
        if !route.module.alive.load(Ordering::SeqCst) {
            return Err(RouteFailure::new("runner stopped with fork in flight"));
        }
        if !route
            .module
            .world
            .held_models
            .lock()
            .unwrap()
            .contains_key(&route.session)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let mut store = route.module.forks.lock().unwrap();
    let session = store.sessions.get_mut(&route.session).unwrap();
    let reply = finish(&route.module, &route.session, session, fork_id, false);
    store.save(&route.module);
    Ok(reply)
}

fn begin(route: &FakeRoute, method: &str, params: &Value) -> Reply {
    let module = &route.module;
    let mut store = module.forks.lock().unwrap();
    if method == ops::SESSION_FORK_PREPARE {
        if let Some(field) = unknown_field(params, &[]) {
            return invalid(&field);
        }
        let session = store.sessions.entry(route.session.clone()).or_default();
        if !session.callers.contains(&route.stamp.principal) && !broken(module, ForkDefect::Callers)
        {
            return refuse(errors::SCOPE_OWNER_MISMATCH, None);
        }
        let Some(base) = session.head.clone() else {
            return refuse(errors::FORK_BASE_UNAVAILABLE, None);
        };
        let mids = module.sessions.lock().unwrap()[&route.session]
            .messages
            .iter()
            .map(|m| m.mid.clone())
            .collect();
        let deadline = session.now + 2000;
        store.next_id += 1;
        let id = format!("fork:{}", store.next_id);
        store
            .sessions
            .get_mut(&route.session)
            .unwrap()
            .forks
            .insert(
                id.clone(),
                Fork {
                    principal: route.stamp.principal.clone(),
                    base,
                    mids,
                    branch: Vec::new(),
                    deadline,
                    ended: false,
                    revoked: false,
                    answers: BTreeMap::new(),
                    attempts: BTreeMap::new(),
                    pending: None,
                },
            );
        store.save(module);
        return respond(json!({"fork_id":id}));
    }
    let fields = if method == ops::SESSION_FORK_ASK {
        &["fork_id", "ask_id", "question"][..]
    } else {
        &["fork_id"][..]
    };
    if let Some(field) = unknown_field(params, fields) {
        return invalid(&field);
    }
    let Some(id) = params["fork_id"].as_str() else {
        return invalid("fork_id");
    };
    let Some(session) = store.sessions.get_mut(&route.session) else {
        return refuse(errors::UNKNOWN_FORK, None);
    };
    let Some(fork) = session.forks.get_mut(id) else {
        return refuse(errors::UNKNOWN_FORK, None);
    };
    if (fork.principal != route.stamp.principal || fork.revoked)
        && !broken(module, ForkDefect::Callers)
    {
        return refuse(errors::UNKNOWN_FORK, None);
    }
    if method == ops::SESSION_FORK_END {
        fork.ended = true;
        store.save(module);
        return respond(json!({"fork_id":id,"ended":true}));
    }
    let Some(ask_id) = params["ask_id"]
        .as_str()
        .filter(|s| (1..=256).contains(&s.chars().count()))
    else {
        return invalid("ask_id");
    };
    let Some(question) = params["question"].as_str().filter(|s| !s.is_empty()) else {
        return invalid("question");
    };
    if let Some((q, answer)) = fork.answers.get(ask_id) {
        if q != question {
            return refuse(errors::ASK_ID_REUSE, None);
        }
        let mut answer = answer.clone();
        if broken(module, ForkDefect::Replay) {
            answer["answer"] = json!("changed replay");
        }
        return respond(answer);
    }
    if fork.pending.is_some() {
        return refuse(errors::FORK_BUSY, None);
    }
    if session.now >= fork.deadline {
        return refuse(errors::UNKNOWN_FORK, None);
    }
    if fork.ended {
        return refuse(errors::FORK_ENDED, None);
    }
    let gone = module
        .sessions
        .lock()
        .unwrap()
        .get(&route.session)
        .is_none_or(|s| {
            !fork
                .mids
                .iter()
                .all(|mid| s.messages.iter().any(|m| &m.mid == mid))
        });
    if gone && !broken(module, ForkDefect::Base) {
        return refuse(errors::FORK_BASE_UNAVAILABLE, None);
    }
    let attempt = fork.attempts.entry(ask_id.into()).or_default();
    *attempt += 1;
    let admission = match admission(module, &route.session, id, ask_id, *attempt) {
        Ok(a) => a,
        Err(reply) => {
            store.save(module);
            return reply;
        }
    };
    let mut body = if broken(module, ForkDefect::Base) {
        session.head.clone().unwrap()
    } else {
        fork.base.clone()
    };
    if broken(module, ForkDefect::BaseBytes) {
        let mut first = body["messages"][0].as_str().unwrap().to_owned();
        first.replace_range(0..1, "X");
        body["messages"][0] = json!(first);
    }
    let messages = body["messages"].as_array_mut().unwrap();
    if broken(module, ForkDefect::RelocatedMarker) {
        for value in messages.iter_mut() {
            value["content"][0]
                .as_object_mut()
                .unwrap()
                .remove("cache_control");
        }
        messages.extend(fork.branch.iter().cloned().map(|text| message(text, false)));
        messages.push(message(json!(question), true));
    } else {
        messages.extend(fork.branch.clone());
        messages.push(json!(question));
    }
    let mut ingress = module.world.fork_ingress.lock().unwrap();
    let raw = ingress.raw.entry(route.session.clone()).or_default();
    let turn = module.world.scripts.lock().unwrap()[&route.session]
        .turns
        .get(raw.len())
        .cloned()
        .unwrap_or_else(|| ScriptedTurn::new(vec![ScriptedPart::Text("fallback".into())]));
    raw.push(RawModelRequestObservation::new(
        Some(id.into()),
        serde_json::to_vec(&body).unwrap(),
    ));
    drop(ingress);
    let mut answer = String::new();
    let mut finish_reason = "end_turn";
    for part in &turn.parts {
        match part {
            ScriptedPart::Text(s) => answer.push_str(s),
            ScriptedPart::ToolCall(tool) => {
                if broken(module, ForkDefect::Tools) {
                    *module
                        .world
                        .invocations
                        .lock()
                        .unwrap()
                        .entry(tool.arguments.to_string())
                        .or_default() += 1;
                }
                finish_reason = "tool_call_refused";
                break;
            }
            _ => {}
        }
    }
    if turn.model_failure.is_some() {
        answer.clear();
        finish_reason = "model_error";
    }
    let mut usage = usage(&turn);
    if let Some(tier) = module
        .world
        .fork_ingress
        .lock()
        .unwrap()
        .head_tiers
        .get(&route.session)
    {
        usage["service_tier_applied"] = json!(tier);
    }
    let answer = json!({"answer":answer,"finish":finish_reason,"usage":usage});
    if broken(module, ForkDefect::Head) {
        assert!(module
            .commit(
                &route.session,
                json!({"kind":"unattributed","marker":question})
            )
            .is_ok());
    }
    fork.pending = Some(Pending {
        id: ask_id.into(),
        question: question.into(),
        answer,
        admission,
    });
    let reply = if module
        .world
        .pending_model_holds
        .lock()
        .unwrap()
        .remove(&route.session)
    {
        module
            .world
            .held_models
            .lock()
            .unwrap()
            .insert(route.session.clone(), id.into());
        respond(json!({"waiting":true}))
    } else {
        finish(module, &route.session, session, id, false)
    };
    store.save(module);
    reply
}

fn message(text: Value, cache: bool) -> Value {
    let mut value = json!({"role":"user","content":[{"type":"text","text":text}]});
    if cache {
        value["content"][0]["cache_control"] = json!({"type":"ephemeral"});
    }
    value
}
