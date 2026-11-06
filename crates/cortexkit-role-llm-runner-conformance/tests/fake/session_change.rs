//! The fake's `session_change` group: `session.refresh`,
//! `session.refresh_policy` and `session.flush_prefix`, kept in the same
//! durable log as everything else. A change waits for the start of the next
//! run (`apply_due_change`), so no step of a running run sees it; the
//! defects below break that and the answers one at a time.

use super::*;
use cortexkit_role_llm_runner_conformance::{
    wire::{
        baseline::{EffectiveBaseline, PendingChange, Rungs},
        rungs,
        session_change::{
            outside_host_set, refresh_order, surfaces, AlreadyAppliedDetail, FlushPrefixReply,
            FlushPrefixRequest, GenerationStaleDetail, PendingSupersededDetail, RefreshOrder,
            RefreshPolicyReply, RefreshPolicyRequest, RefreshReply, RefreshRequest,
            RungUnsupportedDetail, ToolDefinition, ToolOutsideHostSetDetail,
        },
    },
    RefreshPlan, RefreshVariant,
};

/// A second tool the host registered at start, which a refreshed plan may
/// add.
pub const EXTRA_TOOL: &str = "conformance_extra";

/// A tool the host never registered.
const UNREGISTERED_TOOL: &str = "conformance_unregistered";

/// The rungs every fake session supports, on both surfaces.
const FAKE_RUNGS: &[&str] = &[rungs::ON_PREFIX_REBUILD, rungs::IMMEDIATELY];

/// The rungs a fake session states, on both surfaces: [`FAKE_RUNGS`], or
/// only `immediately` when `defects.states_only_immediately` is set.
pub fn fake_rungs(defects: &Defects) -> Rungs {
    let stated: &[&str] = if defects.states_only_immediately {
        &[rungs::IMMEDIATELY]
    } else {
        FAKE_RUNGS
    };
    Rungs {
        tools: stated.iter().map(|rung| (*rung).to_owned()).collect(),
        system_text: stated.iter().map(|rung| (*rung).to_owned()).collect(),
    }
}

/// The tool set the fake's host fixes when it starts, as OpenCode registers
/// every tool's name, description and input schema then.
fn host_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new(
            SCRIPTED_TOOL,
            "Echo the call's arguments.",
            json!({"type":"object"}),
        ),
        ToolDefinition::new(EXTRA_TOOL, "A second echo.", json!({"type":"object"})),
    ]
}

/// The definitions a plan offers: its `tools` by name, each with the host's
/// definition unless the plan's `definitions` member gives another, and an
/// empty one for a name the host does not know.
fn offered(plan: &Map<String, Value>) -> (Vec<String>, Vec<ToolDefinition>) {
    let names: Vec<String> = plan
        .get("tools")
        .and_then(|tools| serde_json::from_value(tools.clone()).ok())
        .unwrap_or_default();
    let host = host_tools();
    let definitions = names
        .iter()
        .map(|name| {
            if let Some(given) = plan.get("definitions").and_then(|d| d.get(name)) {
                return ToolDefinition::new(
                    name.as_str(),
                    given["description"].as_str().unwrap_or_default(),
                    given["input_schema"].clone(),
                );
            }
            host.iter()
                .find(|tool| &tool.name == name)
                .cloned()
                .unwrap_or_else(|| ToolDefinition::new(name.as_str(), "", Value::Null))
        })
        .collect();
    (names, definitions)
}

/// A change accepted and not yet applied.
#[derive(Clone, Debug)]
pub struct PendingRec {
    generation: u64,
    policy: String,
    digest: String,
    tools: Vec<String>,
    senders: Vec<super::order::PlanSender>,
    system_note: Option<String>,
}

/// What a session's `session_change` records built.
#[derive(Clone, Debug, Default)]
pub struct ChangeState {
    accepted: u64,
    applied: u64,
    /// Every accepted refresh by generation: its plan identity, policy and
    /// answer, which an identical repeat replays.
    refreshes: BTreeMap<u64, (String, String, Value)>,
    pending: Option<PendingRec>,
    /// The latest applied change: generation, how, plan identity.
    effective: Option<(u64, String, String)>,
    /// A flush waits for the next run.
    flush: bool,
    /// Answered flushes and policy changes, by `send_id`.
    flushes: BTreeMap<String, Value>,
    policies: BTreeMap<String, (Value, Value)>,
}

/// Apply one of the group's durable records to the session it names.
pub fn apply(sess: &mut Sess, kind: &str, record: &Value) {
    let change = &mut sess.change;
    match kind {
        "refresh" => {
            let generation = record["generation"].as_u64().unwrap_or_default();
            let policy = record["policy"].as_str().unwrap_or_default().to_owned();
            let digest = record["digest"].as_str().unwrap_or_default().to_owned();
            change.accepted = change.accepted.max(generation);
            change.refreshes.insert(
                generation,
                (digest.clone(), policy.clone(), record["answer"].clone()),
            );
            change.pending = Some(PendingRec {
                generation,
                policy,
                digest,
                tools: serde_json::from_value(record["tools"].clone()).unwrap_or_default(),
                senders: serde_json::from_value(record["senders"].clone()).unwrap_or_default(),
                system_note: record["system_note"].as_str().map(str::to_owned),
            });
            if record["defer_senders"] != true {
                sess.senders =
                    serde_json::from_value(record["senders"].clone()).unwrap_or_default();
            }
            sess.readers = record
                .get("readers")
                .filter(|value| !value.is_null())
                .map(|value| serde_json::from_value(value.clone()).unwrap());
            if record["store_note"] == true {
                let mut message = sess.messages.last().unwrap().clone();
                message.ordinal += 1;
                message.mid.push_str("-plan-note");
                message.body =
                    json!({"role":"system","parts":[{"type":"text","text":record["system_note"]}]});
                message.host_body = message.body.clone();
                sess.messages.push(message);
            }
        }
        "refresh_policy" => {
            change.policies.insert(
                record["send_id"].as_str().unwrap_or_default().to_owned(),
                (record["request"].clone(), record["answer"].clone()),
            );
            if let Some(pending) = &mut change.pending {
                pending.policy = record["policy"].as_str().unwrap_or_default().to_owned();
            }
        }
        "flush" => {
            change.flushes.insert(
                record["send_id"].as_str().unwrap_or_default().to_owned(),
                record["answer"].clone(),
            );
            change.flush = true;
        }
        "change_applied" => {
            if let Some(pending) = change.pending.take() {
                change.applied = pending.generation;
                change.effective = Some((
                    pending.generation,
                    record["applied_by"].as_str().unwrap_or_default().to_owned(),
                    pending.digest.clone(),
                ));
                if record["keep_old_tools"] != true {
                    sess.tools = pending.tools;
                }
                sess.senders = pending.senders;
                sess.plan_digest = Some(pending.digest);
            }
            sess.change.flush = false;
            if record["rewrite_history"] == true {
                if let Some(message) = sess.messages.first_mut() {
                    message.body = json!({"text":"rewritten by host relaunch"});
                }
            }
        }
        _ => {}
    }
}

impl Module {
    /// Serve one of the group's ops.
    pub(super) fn session_change(
        &self,
        stamp: &RouteStamp,
        session: &str,
        method: &str,
        params: Value,
    ) -> Result<Reply, Killed> {
        let allowed: &[&str] = match method {
            ops::SESSION_REFRESH
                if self.world.serves(groups::PLAN_NOTES)
                    || self.world.defects.plan_notes == Some("undeclared_policy") =>
            {
                &["plan", "policy", "generation", "system_note"]
            }
            ops::SESSION_REFRESH => &["plan", "policy", "generation"],
            ops::SESSION_REFRESH_POLICY => &["generation", "policy", "send_id"],
            _ => &["send_id"],
        };
        if let Some(field) = unknown_field(&params, allowed) {
            return Ok(invalid(&field));
        }
        let missing = |error: serde_json::Error| {
            let text = error.to_string();
            let field = allowed
                .iter()
                .find(|field| text.contains(*field))
                .unwrap_or(&allowed[0]);
            invalid(field)
        };
        let Some(sess) = self.sessions.lock().unwrap().get(session).cloned() else {
            return Ok(invalid(allowed[0]));
        };
        if sess.owner != stamp.principal {
            return Ok(refuse(
                errors::SCOPE_OWNER_MISMATCH,
                Some(json!({ "accepted": sess.owner })),
            ));
        }
        match method {
            ops::SESSION_REFRESH
                if self.world.serves(groups::PLAN_NOTES)
                    || self.world.defects.plan_notes == Some("undeclared_policy") =>
            {
                let request: cortexkit_role_llm_runner::plan_notes::PlanNotesRefreshRequest =
                    match serde_json::from_value(params) {
                        Ok(request) => request,
                        Err(error) => return Ok(missing(error)),
                    };
                let groups = if self.world.defects.plan_notes == Some("undeclared_policy") {
                    vec![groups::PLAN_NOTES.to_owned()]
                } else {
                    self.world.groups.clone()
                };
                if let Err(field) = request.check(&groups, request.plan.contains_key("system_text"))
                {
                    return Ok(invalid(field));
                }
                self.refresh(
                    session,
                    &sess,
                    RefreshRequest::new(request.plan, request.policy, request.generation),
                    request.system_note,
                )
            }
            ops::SESSION_REFRESH => match serde_json::from_value(params) {
                Ok(request) => self.refresh(session, &sess, request, None),
                Err(error) => Ok(missing(error)),
            },
            ops::SESSION_REFRESH_POLICY => match serde_json::from_value(params) {
                Ok(request) => self.refresh_policy(session, &sess, request),
                Err(error) => Ok(missing(error)),
            },
            _ => match serde_json::from_value(params) {
                Ok(request) => self.flush_prefix(session, &sess, request),
                Err(error) => Ok(missing(error)),
            },
        }
    }

    fn refresh(
        &self,
        session: &str,
        sess: &Sess,
        request: RefreshRequest,
        system_note: Option<String>,
    ) -> Result<Reply, Killed> {
        let selection = match self.selection(&request.plan) {
            Ok(model) => model,
            Err(reply) => return Ok(reply),
        };
        if let Err(field) = request.check_sender_succession(&self.world.groups) {
            return Ok(invalid(&field));
        }
        if self.world.defects.frozen_plan_senders_accepted != Some("refresh") {
            let senders =
                match cortexkit_role_llm_runner::delivery_order::plan_senders(&request.plan) {
                    Ok(senders) => senders,
                    Err(field) => return Ok(invalid(field)),
                };
            if let Err(field) = cortexkit_role_llm_runner::scope_admission::check_plan_senders(
                sess.frozen_opener.as_deref(),
                &senders,
            ) {
                if self.world.defects.frozen_plan_senders_accepted == Some("refresh_write") {
                    self.commit(
                        session,
                        json!({"kind":"start", "owner":sess.owner, "lineage":"refused-plan-write"}),
                    )?;
                }
                return Ok(invalid(field));
            }
        }
        let notes = request.policy == cortexkit_role_llm_runner::plan_notes::NOTES_UNTIL_REBUILD;
        if let Err(field) = if notes
            && (self.world.serves(groups::PLAN_NOTES)
                || self.world.defects.plan_notes == Some("undeclared_policy"))
        {
            Ok(())
        } else {
            request.check_policy()
        } {
            return Ok(invalid(field));
        }
        let defects = &self.world.defects;
        let change = &sess.change;
        let digest = plan_digest(&request.plan);
        let earlier = change
            .refreshes
            .get(&request.generation)
            .map(|(digest, policy, _)| (digest.as_str(), policy.as_str()));
        let order = match refresh_order(
            change.accepted,
            request.generation,
            &digest,
            &request.policy,
            earlier,
        ) {
            RefreshOrder::Repeat if defects.refresh_repeat_refused => RefreshOrder::Stale,
            RefreshOrder::Stale if defects.refresh_stale_accepted => RefreshOrder::New,
            order => order,
        };
        match order {
            RefreshOrder::Repeat => {
                return Ok(Reply::Response(
                    change.refreshes[&request.generation].2.clone(),
                ))
            }
            RefreshOrder::Stale => {
                return Ok(refuse(
                    errors::GENERATION_STALE,
                    Some(json!(GenerationStaleDetail::new(change.accepted))),
                ))
            }
            RefreshOrder::New => {}
        }
        let (names, definitions) = offered(&request.plan);
        let outside = if defects.host_set_by_name_only {
            let host: Vec<String> = host_tools().into_iter().map(|tool| tool.name).collect();
            names
                .iter()
                .filter(|name| !host.contains(name))
                .cloned()
                .collect()
        } else {
            outside_host_set(&host_tools(), &definitions)
        };
        if !outside.is_empty() && !self.world.serves(groups::REFRESH_RELAUNCH) {
            return Ok(refuse(
                errors::TOOL_OUTSIDE_HOST_SET,
                Some(json!(ToolOutsideHostSetDetail::new(outside))),
            ));
        }
        if !notes && !FAKE_RUNGS.contains(&request.policy.as_str()) {
            return Ok(refuse(
                errors::RUNG_UNSUPPORTED,
                Some(json!(RungUnsupportedDetail::new(
                    surfaces::TOOLS,
                    &request.policy
                ))),
            ));
        }
        let running = sess.runs.iter().any(|run| run.state == "active");
        let applies =
            request.policy == rungs::IMMEDIATELY || change.flush || defects.refresh_claims_next_run;
        let answer = RefreshReply::new(
            PendingChange {
                generation: request.generation,
                policy: request.policy.clone(),
            },
            running && !defects.refresh_hides_running_run,
            applies,
        );
        let answer = serde_json::to_value(answer).unwrap();
        self.commit(
            session,
            json!({ "kind": "refresh", "generation": request.generation,
                    "selection": selection,
                    "policy": request.policy, "digest": digest, "tools": names,
                    "senders": request.plan.get("senders").cloned().unwrap_or_else(|| json!([])),
                    "defer_senders": defects.refreshed_senders_wait_for_next_run,
                    "readers": request.plan.get("readers"),
                    "system_note": system_note,
                    "store_note": notes && defects.plan_notes == Some("stored_entry"),
                    "answer": answer }),
        )?;
        if running && request.policy == rungs::IMMEDIATELY && defects.refresh_applies_mid_run {
            self.apply_change(session, "append")?;
        }
        Ok(Reply::Response(answer))
    }

    fn refresh_policy(
        &self,
        session: &str,
        sess: &Sess,
        request: RefreshPolicyRequest,
    ) -> Result<Reply, Killed> {
        if let Err(field) = request.check_policy() {
            return Ok(invalid(field));
        }
        let change = &sess.change;
        if let Some((earlier, answer)) = change.policies.get(&request.send_id) {
            let earlier: RefreshPolicyRequest =
                serde_json::from_value(earlier.clone()).expect("stored requests decode");
            return Ok(match request.reused_field(&earlier) {
                Some(field) => refuse(errors::SEND_ID_REUSE, Some(json!({ "field": field }))),
                None => Reply::Response(answer.clone()),
            });
        }
        let pending = match &change.pending {
            Some(pending) if pending.generation == request.generation => pending,
            Some(pending) if request.generation < pending.generation => {
                return Ok(refuse(
                    errors::PENDING_SUPERSEDED,
                    Some(json!(PendingSupersededDetail::new(
                        request.generation,
                        pending.generation
                    ))),
                ))
            }
            _ if request.generation <= change.applied => {
                let applied_by = change
                    .effective
                    .as_ref()
                    .map(|(_, by, _)| by.clone())
                    .unwrap_or_default();
                return Ok(refuse(
                    errors::ALREADY_APPLIED,
                    Some(json!(AlreadyAppliedDetail::new(
                        request.generation,
                        applied_by
                    ))),
                ));
            }
            _ => return Ok(invalid("generation")),
        };
        if !FAKE_RUNGS.contains(&request.policy.as_str()) {
            return Ok(refuse(
                errors::RUNG_UNSUPPORTED,
                Some(json!(RungUnsupportedDetail::new(
                    surfaces::TOOLS,
                    &request.policy
                ))),
            ));
        }
        let answer = serde_json::to_value(RefreshPolicyReply::new(PendingChange {
            generation: pending.generation,
            policy: request.policy.clone(),
        }))
        .unwrap();
        self.commit(
            session,
            json!({ "kind": "refresh_policy", "send_id": request.send_id, "request": request,
                    "policy": request.policy, "answer": answer }),
        )?;
        Ok(Reply::Response(answer))
    }

    fn flush_prefix(
        &self,
        session: &str,
        sess: &Sess,
        request: FlushPrefixRequest,
    ) -> Result<Reply, Killed> {
        if let Some(answer) = sess.change.flushes.get(&request.send_id) {
            return Ok(Reply::Response(answer.clone()));
        }
        let answer = serde_json::to_value(FlushPrefixReply::new(
            sess.change
                .pending
                .as_ref()
                .map(|pending| pending.generation),
        ))
        .unwrap();
        self.commit(
            session,
            json!({ "kind": "flush", "send_id": request.send_id, "answer": answer }),
        )?;
        let running = sess.runs.iter().any(|run| run.state == "active");
        if running && self.world.defects.flush_applies_mid_run {
            self.commit(
                session,
                json!({ "kind": "change_applied", "applied_by": "fold" }),
            )?;
        }
        Ok(Reply::Response(answer))
    }

    /// At the start of a run, apply the pending change if it is due: its
    /// rung is `immediately`, or a flush is waiting. Never called inside a
    /// running run, so no step sees a change its run did not start with.
    pub(super) fn apply_due_change(&self, session: &str) -> Result<(), Killed> {
        if !self.world.serves(groups::SESSION_CHANGE) {
            return Ok(());
        }
        let Some(change) = self
            .sessions
            .lock()
            .unwrap()
            .get(session)
            .map(|sess| sess.change.clone())
        else {
            return Ok(());
        };
        let defects = &self.world.defects;
        let flush = change.flush && !defects.flush_ignored;
        let immediate = change.pending.as_ref().is_some_and(|pending| {
            pending.policy == rungs::IMMEDIATELY && !defects.immediately_waits_for_rebuild
                || pending.policy == cortexkit_role_llm_runner::plan_notes::NOTES_UNTIL_REBUILD
                    && defects.plan_notes == Some("early_apply")
        });
        if flush || immediate {
            let applied_by = if flush { "fold" } else { "append" };
            self.apply_change(session, applied_by)?;
        }
        Ok(())
    }

    fn apply_change(&self, session: &str, applied_by: &str) -> Result<(), Killed> {
        let relaunch = self.world.serves(groups::REFRESH_RELAUNCH);
        self.commit(
            session,
            json!({
                "kind": "change_applied", "applied_by": applied_by,
                "keep_old_tools": relaunch && self.world.defects.relaunch_keeps_old_tools,
                "rewrite_history": relaunch && self.world.defects.relaunch_rewrites_history
            }),
        )
    }

    /// `base` with the session's change state: the rungs it supports, the
    /// accepted and applied generations, the pending change and the
    /// effective one. Unchanged on a runner without the group.
    pub(super) fn change_baseline(&self, session: &str, base: Baseline) -> Baseline {
        if !self.world.serves(groups::SESSION_CHANGE) {
            return base;
        }
        let mut baseline = base.with_rungs(fake_rungs(&self.world.defects));
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            return baseline;
        };
        let change = &sess.change;
        baseline = baseline.with_generations(change.accepted, change.applied);
        if let Some(pending) = &change.pending {
            baseline = baseline.with_pending(PendingChange {
                generation: pending.generation,
                policy: pending.policy.clone(),
            });
        }
        if let Some((generation, applied_by, digest)) = &change.effective {
            baseline.tool_names = sess.tools.clone();
            baseline = baseline.with_effective(EffectiveBaseline {
                generation: *generation,
                applied_by: applied_by.clone(),
                manifest_digest: digest.clone(),
            });
        }
        baseline
    }
}

pub fn notes_fixture() -> cortexkit_role_llm_runner_conformance::PlanNotesFixture {
    let mut plan = refresh_plan(RefreshVariant::ChangedTools).unwrap().plan;
    plan.insert("tools".into(), json!([EXTRA_TOOL]));
    plan.insert(
        "system_text".into(),
        json!("Replacement system text with detailed instructions, not a change note."),
    );
    cortexkit_role_llm_runner_conformance::PlanNotesFixture::new(
        plan,
        "Use the updated response style.",
        "Replacement system text with detailed instructions, not a change note.",
        SCRIPTED_TOOL,
    )
}

pub fn observe_notes(
    observation: &mut cortexkit_role_llm_runner_conformance::ModelRequestObservation,
    sess: &Sess,
    defects: &Defects,
) {
    let Some(pending) = sess
        .change
        .pending
        .as_ref()
        .filter(|p| p.policy == cortexkit_role_llm_runner::plan_notes::NOTES_UNTIL_REBUILD)
    else {
        return;
    };
    if let Some(note) = &pending.system_note {
        let text = if defects.plan_notes == Some("full_system_text") {
            notes_fixture().new_system_text
        } else {
            note.clone()
        };
        observation.rendered.push(text.clone());
        observation
            .system_text
            .get_or_insert_with(Vec::new)
            .push(text);
    }
}

pub fn removed_tool_message(sess: &Sess, tool_call_id: &str, defects: &Defects) -> Option<String> {
    let pending = sess
        .change
        .pending
        .as_ref()
        .filter(|p| p.policy == cortexkit_role_llm_runner::plan_notes::NOTES_UNTIL_REBUILD)?;
    let tool = sess
        .messages
        .iter()
        .flat_map(|message| message.body["parts"].as_array().into_iter().flatten())
        .find(|part| part["id"] == tool_call_id)?["tool"]
        .as_str()?;
    if pending.tools.iter().any(|name| name == tool)
        || defects.plan_notes == Some("removed_dispatched")
    {
        return None;
    }
    Some(if defects.plan_notes == Some("removal_message") {
        "tool not selected".into()
    } else {
        cortexkit_role_llm_runner::plan_notes::tool_removal(tool)
    })
}

/// The adapter's refresh plans: one adding [`EXTRA_TOOL`], one naming a
/// tool the host never registered, and one changing the scripted tool's
/// description and input schema under its kept name.
pub fn refresh_plan(variant: RefreshVariant) -> Option<RefreshPlan> {
    let mut plan = Map::new();
    let tools = match variant {
        RefreshVariant::ChangedTools => {
            plan.insert("tools".into(), json!([SCRIPTED_TOOL, EXTRA_TOOL]));
            vec![SCRIPTED_TOOL.to_owned(), EXTRA_TOOL.to_owned()]
        }
        RefreshVariant::OutsideHostSet => {
            plan.insert("tools".into(), json!([SCRIPTED_TOOL, UNREGISTERED_TOOL]));
            vec![UNREGISTERED_TOOL.to_owned()]
        }
        RefreshVariant::ChangedDefinition => {
            plan.insert("tools".into(), json!([SCRIPTED_TOOL]));
            plan.insert(
                "definitions".into(),
                json!({ (SCRIPTED_TOOL): {"description": "Echo, described anew.",
                                        "input_schema": {"type": "object", "required": ["marker"]}} }),
            );
            vec![SCRIPTED_TOOL.to_owned()]
        }
        _ => return None,
    };
    Some(RefreshPlan::new(plan, tools))
}
