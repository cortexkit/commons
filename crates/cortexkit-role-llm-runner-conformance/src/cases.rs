//! The cases that run on the shared runner, one session each.

use std::collections::BTreeSet;

use cortexkit_role_harness::DriverHandle;
use cortexkit_role_llm_runner::{
    baseline::{BaselineReply, BaselineRequest},
    capabilities::{self as groups, source},
    describe::{check_describe, steer_receipt, RoleDescribe, Stability},
    errors, ops,
    read::{HeadMeta, ReadMessage, ReadPage, ReadRequest, ToolCallAttribution},
    run::{join_text_parts, states as run_states},
    send::{Delivered, DeliveredAs, SendReply, DELIVERY_FIELD},
    subscribe::{kinds, SubscribeEvent, SubscribeFrom, SubscribeRequest},
    PROVIDES,
};
use futures_util::future::join;
use serde_json::{json, Map, Value};

use crate::{
    drive::{
        call, call_key_well_formed, check_unique, decode, expect_code, expect_error, expect_field,
        expect_ok, head, holding, read, read_all, read_refusal, run_result, send_params,
        wait_run_end, Mint, RunEnd, MAX_POLLS,
    },
    route::{RunnerRoute, SubscribeOutcome},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

/// How a case that did not fail ended.
pub enum Ending {
    Passed,
    /// The question cannot be asked of this runner.
    Inapplicable(String),
    /// Not run: without a test clock, the case would have to wait longer in real time than the suite allows (30 seconds).
    Skipped {
        missing: Vec<Capability>,
        reason: String,
    },
}

type Check = Result<(), String>;

/// A session the suite writes, with its owner's route.
pub struct Session<R> {
    pub name: String,
    pub route: R,
    /// The `send_id` and prompt of the session's first send.
    pub first: Option<(String, String)>,
}

/// The context of a case that runs on the shared runner.
pub struct Case<'a, S: LlmRunnerSubject> {
    pub subject: &'a S,
    pub handle: &'a DriverHandle<S>,
    pub declared: &'a BTreeSet<Capability>,
    pub mint: &'a Mint,
}

/// A prompt far larger than any byte cap the oversize case asks for.
const OVERSIZE_BYTES: usize = 16 * 1024;

/// The byte cap the oversize case reads with.
const OVERSIZE_CAP: u64 = 1024;

/// Why a reply half of a `send_id` case cannot run.
const NO_SEND_MODE: &str =
    "the runner declares neither queue nor steer, so it takes no send to retry or reuse";

/// Why the settled retry case cannot run when the send's reply names no
/// run: `run.result` is the only way it can see a run end without reading
/// the transcript, and it takes a `run_id`.
const NO_RUN_TO_SETTLE: &str = "the send's reply names no run_id, so the suite cannot wait \
     for its run to end through run.result and compare retries made after it";

/// Why a written-once half of a `send_id` case cannot run.
const NO_TRANSCRIPT: &str = "the runner does not declare transcript_reads, so the suite cannot \
     read what the send wrote; the reply half of this check still runs";

impl<S> Case<'_, S>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    pub async fn run(&self, name: &str) -> Result<Ending, String> {
        let passed = |result: Check| result.map(|()| Ending::Passed);
        match name {
            "role_describe_shape" => passed(self.role_describe_shape().await),
            "role_describe_cacheable" => passed(self.role_describe_cacheable().await),
            "role_describe_groups_complete" => passed(self.role_describe_groups_complete().await),
            "extra_op_still_admitted" => passed(self.extra_op_still_admitted().await),
            "describe_states_max_bytes" => passed(self.describe_states_max_bytes().await),
            "baseline_owner_only" => passed(self.baseline_owner_only().await),
            "baseline_matches_admission_reply" => self.baseline_matches_admission_reply().await,
            "tail_read_is_newest_page" => passed(self.tail_read_is_newest_page().await),
            "range_read_stops_by_count" => passed(self.range_read_stops_by_count().await),
            "range_read_stops_by_bytes" => passed(self.range_read_stops_by_bytes().await),
            "oversize_message_alone_on_its_page" => passed(self.oversize_alone().await),
            "after_mid_reads_strictly_after" => passed(self.after_mid_strictly_after().await),
            "after_mid_unknown_mid_refused" => passed(self.after_mid_unknown_mid().await),
            "read_other_lineage_refused" => passed(self.read_other_lineage().await),
            "lineage_id_on_every_page" => passed(self.lineage_on_every_page().await),
            "unknown_read_field_refused" => passed(self.unknown_read_field().await),
            "never_written_session_empty_page" => passed(self.never_written_page().await),
            "never_written_session_empty_head" => passed(self.never_written_head().await),
            "head_has_no_bodies" => passed(self.head_has_no_bodies().await),
            "run_result_completed_final_text" => passed(self.run_result_final_text().await),
            "run_result_text_parts_joined_unchanged" => passed(self.run_result_joined().await),
            "run_result_completed_no_text_is_empty" => passed(self.run_result_no_text().await),
            "dispatched_to_per_call" => passed(self.dispatched_to_per_call().await),
            "indeterminate_until_closed" => passed(self.indeterminate_until_closed().await),
            "sibling_calls_distinct_call_keys" => passed(self.sibling_call_keys().await),
            "recurring_model_id_distinct_call_keys" => passed(self.recurring_call_keys().await),
            "subscribe_from_head_no_gap_no_duplicate" => passed(self.subscribe_from_head().await),
            "send_id_retry_same_answer" => self.send_id_retry().await,
            "send_id_retry_settled_same_answer" => self.send_id_retry_settled().await,
            "send_id_retry_written_once" => self.send_id_retry_written_once().await,
            "send_id_reuse_refused_naming_field" => self.send_id_reuse().await,
            "send_id_reuse_writes_nothing" => self.send_id_reuse_writes_nothing().await,
            "delivery_change_refused" => self.delivery_change().await,
            "delivery_change_writes_nothing" => self.delivery_change_writes_nothing().await,
            "unknown_delivery_refused" => passed(self.unknown_delivery().await),
            "undeclared_delivery_refused" => passed(self.undeclared_delivery().await),
            "guaranteed_steer_never_pending_or_unknown" => {
                self.guaranteed_steer_never_pending_or_unknown().await
            }
            "resend_steer_delivered_stable" => self.resend_steer_delivered_stable().await,
            name if name.starts_with("retention_") => self.retention_case(name).await,
            name if name.starts_with("compaction_") || name.starts_with("model_view_") => {
                crate::compaction::run(self, name)
                    .await
                    .map(|()| Ending::Passed)
            }
            other => Err(format!("the runner has no case named {other}")),
        }
    }

    fn declares(&self, capability: Capability) -> bool {
        self.declared.contains(&capability)
    }

    // ---- sessions -------------------------------------------------------

    /// A fresh session whose model is `script`, with the owner's route.
    pub(crate) async fn open(
        &self,
        label: &str,
        script: Script,
    ) -> Result<Session<S::Route>, String> {
        let name = self.mint.next(label);
        self.subject
            .install_script(&name, script)
            .await
            .map_err(|e| format!("installing the script of {name}: {e}"))?;
        let route = self
            .subject
            .session_route(self.handle.inner(), &name, &self.subject.owner_stamp())
            .await
            .map_err(|e| format!("opening the route of {name}: {e}"))?;
        Ok(Session {
            name,
            route,
            first: None,
        })
    }

    async fn stranger_route(&self, session: &str) -> Result<S::Route, String> {
        self.subject
            .session_route(self.handle.inner(), session, &self.subject.stranger_stamp())
            .await
            .map_err(|e| format!("opening {session} as a stranger: {e}"))
    }

    /// The params of a send into `session`.
    pub(crate) fn params(
        &self,
        session: &Session<S::Route>,
        prompt: &str,
        send_id: &str,
        delivery: Option<&str>,
    ) -> Value {
        let first = session
            .first
            .as_ref()
            .is_none_or(|(first_id, _)| first_id == send_id);
        send_params(
            self.subject,
            &session.name,
            first,
            prompt,
            send_id,
            delivery,
        )
    }

    async fn send_raw(
        &self,
        session: &Session<S::Route>,
        prompt: &str,
        send_id: &str,
        delivery: Option<&str>,
    ) -> Result<crate::route::Reply, String> {
        call(
            &session.route,
            ops::SESSION_SEND,
            self.params(session, prompt, send_id, delivery),
        )
        .await
    }

    /// Send `prompt` with a fresh `send_id` and wait for the run it starts
    /// to end. Returns the send's reply and how the run ended.
    async fn run_prompt(
        &self,
        session: &mut Session<S::Route>,
        prompt: &str,
    ) -> Result<(SendReply, RunEnd), String> {
        let send_id = self.mint.next("send");
        let before = self.last_run_id(&session.route).await?;
        let reply = expect_ok(
            "session.send",
            self.send_raw(session, prompt, &send_id, None).await?,
        )?;
        if session.first.is_none() {
            session.first = Some((send_id, prompt.to_owned()));
        }
        let reply: SendReply = decode("session.send", reply)?;
        let end = wait_run_end(
            self.subject,
            &session.route,
            self.declared,
            before.as_deref(),
            &reply,
        )
        .await?;
        Ok((reply, end))
    }

    async fn last_run_id(&self, route: &S::Route) -> Result<Option<String>, String> {
        if !self.declares(Capability::TranscriptReads) {
            return Ok(None);
        }
        Ok(head(route).await?.last_run_state.map(|last| last.run_id))
    }

    /// A session with one run per turn of `turns`, each started by a fresh
    /// prompt. Returns the session and the prompts.
    async fn session_with_runs(
        &self,
        label: &str,
        turns: Vec<ScriptedTurn>,
        prompts: Vec<String>,
    ) -> Result<Session<S::Route>, String> {
        let mut session = self.open(label, Script { turns }).await?;
        for prompt in &prompts {
            self.run_prompt(&mut session, prompt).await?;
        }
        Ok(session)
    }

    /// A session of three runs, each a prompt and a one-text-part answer:
    /// six messages at least, longer than a two-message page.
    async fn long_session(&self, label: &str) -> Result<(Session<S::Route>, Vec<String>), String> {
        let mut turns = Vec::new();
        let mut markers = Vec::new();
        let mut prompts = Vec::new();
        for _ in 0..3 {
            let (part, marker) = self.mint.text("answer");
            turns.push(ScriptedTurn { parts: vec![part] });
            markers.push(marker);
            let prompt = self.mint.next("prompt");
            markers.push(prompt.clone());
            prompts.push(prompt);
        }
        let session = self.session_with_runs(label, turns, prompts).await?;
        Ok((session, markers))
    }

    async fn all_messages(
        &self,
        route: &S::Route,
        least: usize,
    ) -> Result<Vec<ReadMessage>, String> {
        let (_, messages) = read_all(route).await?;
        check_unique("the transcript", &messages)?;
        if messages.len() < least {
            return Err(format!(
                "the transcript holds {} messages, expected at least {least}",
                messages.len()
            ));
        }
        Ok(messages)
    }

    // ---- describe -------------------------------------------------------

    async fn describe_on(&self, route: &S::Route) -> Result<Value, String> {
        expect_ok(
            "role.describe",
            call(route, ops::ROLE_DESCRIBE, json!({})).await?,
        )
    }

    async fn describe(&self) -> Result<Value, String> {
        let session = self.open("describe", Script::default()).await?;
        self.describe_on(&session.route).await
    }

    async fn checked_describe(&self) -> Result<RoleDescribe, String> {
        let raw = self.describe().await?;
        check_describe(&raw)
            .map_err(|problems| format!("check_describe refused {raw}: {problems:?}"))
    }

    async fn role_describe_shape(&self) -> Check {
        let describe = self.checked_describe().await?;
        let major = describe
            .major(PROVIDES)
            .ok_or("check_describe accepted an answer without the v1 major")?;
        if let Stability::Other(level) = major.stability() {
            return Err(format!(
                "the {PROVIDES} major's stability is {level:?}, not alpha, beta or stable"
            ));
        }
        match describe.session_capabilities_from.as_deref() {
            Some(source::ADMISSION | source::BASELINE) => Ok(()),
            other => Err(format!(
                "session_capabilities_from is {other:?}, not admission or baseline"
            )),
        }
    }

    async fn role_describe_cacheable(&self) -> Check {
        let one = self.open("describe", Script::default()).await?;
        let two = self.open("describe", Script::default()).await?;
        let first = self.describe_on(&one.route).await?;
        let again = self.describe_on(&one.route).await?;
        let other = self.describe_on(&two.route).await?;
        if first != again {
            return Err(format!(
                "two answers on one route differ: {first} then {again}"
            ));
        }
        if first != other {
            return Err(format!(
                "the answers on two sessions' routes differ: {first} and {other}"
            ));
        }
        Ok(())
    }

    async fn role_describe_groups_complete(&self) -> Check {
        let raw = self.describe().await?;
        let describe: RoleDescribe = decode("role.describe", raw)?;
        let major = describe
            .major(PROVIDES)
            .ok_or_else(|| format!("role.describe lists no {PROVIDES} major"))?;
        for (group, group_ops) in groups::GROUPS {
            if !describe.declares(group) {
                continue;
            }
            for op in *group_ops {
                if !major.serves(op) {
                    return Err(format!(
                        "role.describe declares {group} but does not list {op}"
                    ));
                }
            }
        }
        let on_wire: BTreeSet<Capability> = describe
            .capabilities
            .iter()
            .filter_map(|name| Capability::from_group(name))
            .collect();
        let by_subject: BTreeSet<Capability> = self
            .declared
            .iter()
            .copied()
            .filter(|capability| capability.group().is_some())
            .collect();
        if let Some(missing) = by_subject.difference(&on_wire).next() {
            return Err(format!(
                "the subject declares {}, but role.describe does not",
                missing.name()
            ));
        }
        if let Some(extra) = on_wire.difference(&by_subject).next() {
            return Err(format!(
                "role.describe declares {}, which the subject does not, so its cases would be \
                 skipped while the runner claims it",
                extra.name()
            ));
        }
        Ok(())
    }

    async fn extra_op_still_admitted(&self) -> Check {
        let raw = self.describe().await?;
        let base = check_describe(&raw)
            .map_err(|problems| format!("check_describe refused {raw}: {problems:?}"))?;
        let mut grown = raw.clone();
        let majors = grown["majors"]
            .as_array_mut()
            .ok_or("role.describe has no majors array")?;
        let v1 = majors
            .iter_mut()
            .find(|major| major["version"] == PROVIDES)
            .ok_or_else(|| format!("role.describe lists no {PROVIDES} major"))?;
        v1["ops"]
            .as_array_mut()
            .ok_or("the v1 major has no ops array")?
            .push(json!("conformance.extra_op"));
        majors.push(json!({ "version": "llm-runner/v99", "ops": ["conformance.op"], "stability": "experimental" }));
        match grown["capabilities"].as_array_mut() {
            Some(capabilities) => capabilities.push(json!("conformance_unknown_capability")),
            None => grown["capabilities"] = json!(["conformance_unknown_capability"]),
        }
        grown["conformance_extra_member"] = json!({ "nested": true });
        let admitted = check_describe(&grown).map_err(|problems| {
            format!("the answer with an extra op, major, capability and member was refused: {problems:?}")
        })?;
        for (group, _) in groups::GROUPS {
            if admitted.declares(group) != base.declares(group) {
                return Err(format!(
                    "adding unknown members changed whether {group} is declared"
                ));
            }
        }
        if admitted.session_capabilities_from != base.session_capabilities_from
            || admitted.max_bytes != base.max_bytes
        {
            return Err(
                "adding unknown members changed the session-capability source or the byte cap"
                    .into(),
            );
        }
        Ok(())
    }

    async fn describe_states_max_bytes(&self) -> Check {
        let describe = self.checked_describe().await?;
        let cap = describe
            .max_bytes
            .ok_or("role.describe declares transcript_reads without max_bytes")?;
        if cap.default == 0 || cap.default > cap.maximum {
            return Err(format!(
                "max_bytes default {} is zero or above its maximum {}",
                cap.default, cap.maximum
            ));
        }
        Ok(())
    }

    // ---- baseline -------------------------------------------------------

    async fn one_answer_session(&self, label: &str) -> Result<Session<S::Route>, String> {
        let (part, _) = self.mint.text("answer");
        self.open(
            label,
            Script {
                turns: vec![ScriptedTurn { parts: vec![part] }],
            },
        )
        .await
    }

    async fn baseline_of(&self, route: &S::Route) -> Result<crate::route::Reply, String> {
        let params = serde_json::to_value(BaselineRequest::default()).expect("serializes");
        call(route, ops::SESSION_BASELINE, params).await
    }

    async fn baseline_owner_only(&self) -> Check {
        let mut session = self.one_answer_session("baseline").await?;
        let prompt = self.mint.next("prompt");
        self.run_prompt(&mut session, &prompt).await?;
        let owner: BaselineReply = decode(
            "session.baseline",
            expect_ok(
                "the owner's session.baseline",
                self.baseline_of(&session.route).await?,
            )?,
        )?;
        if owner.state != cortexkit_role_llm_runner::baseline::states::READY
            || owner.baseline.is_none()
        {
            return Err(format!(
                "the owner's session.baseline after admission answered {owner:?}, not ready with a baseline"
            ));
        }
        let stranger = self.stranger_route(&session.name).await?;
        let error = expect_error(
            "a stranger's session.baseline",
            self.baseline_of(&stranger).await?,
        )?;
        expect_code(
            "a stranger's session.baseline",
            &error,
            errors::SCOPE_OWNER_MISMATCH,
        )
    }

    async fn baseline_matches_admission_reply(&self) -> Result<Ending, String> {
        let describe: RoleDescribe = decode("role.describe", self.describe().await?)?;
        if describe.session_capabilities_from.as_deref() == Some(source::BASELINE) {
            return Ok(Ending::Inapplicable(
                "the runner takes session capabilities from session.baseline and has no admission reply".into(),
            ));
        }
        let mut session = self.one_answer_session("admission").await?;
        let prompt = self.mint.next("prompt");
        let (reply, _) = self.run_prompt(&mut session, &prompt).await?;
        let hint = reply
            .baseline
            .ok_or("the reply to the send that admitted the session carries no baseline")?;
        let answered: BaselineReply = decode(
            "session.baseline",
            expect_ok("session.baseline", self.baseline_of(&session.route).await?)?,
        )?;
        match answered.baseline {
            Some(baseline) if baseline == hint => Ok(Ending::Passed),
            other => Err(format!(
                "the admission reply's baseline {hint:?} differs from session.baseline's {other:?}"
            )),
        }
    }

    // ---- transcript reads -----------------------------------------------

    async fn tail_read_is_newest_page(&self) -> Check {
        let (session, _) = self.long_session("tail").await?;
        let all = self.all_messages(&session.route, 3).await?;
        let tail = read(&session.route, &ReadRequest::tail().with_limit(2)).await?;
        let head = head(&session.route).await?;
        let newest = &all[all.len() - 2..];
        if tail.messages.len() != 2 {
            return Err(format!(
                "a tail read with limit 2 on a {}-message session answered {} messages",
                all.len(),
                tail.messages.len()
            ));
        }
        let last = tail.messages.last().map(|m| m.ordinal);
        if last != head.last_ordinal {
            return Err(format!(
                "the tail page ends at ordinal {last:?}, session.head's last_ordinal is {:?}",
                head.last_ordinal
            ));
        }
        if tail.messages != newest {
            return Err(format!(
                "the tail page holds ordinals {:?}; the newest are {:?}",
                ordinals(&tail.messages),
                ordinals(newest)
            ));
        }
        Ok(())
    }

    async fn range_read_stops_by_count(&self) -> Check {
        let (session, _) = self.long_session("count").await?;
        let all = self.all_messages(&session.route, 3).await?;
        let page = read(
            &session.route,
            &ReadRequest::range(all[0].ordinal).with_limit(2),
        )
        .await?;
        if page.messages != all[..2] {
            return Err(format!(
                "a range read from {} with limit 2 answered ordinals {:?}, not {:?}",
                all[0].ordinal,
                ordinals(&page.messages),
                ordinals(&all[..2])
            ));
        }
        let next = page.next_from_ordinal.ok_or(
            "a page that stopped at its limit before the end carries no next_from_ordinal",
        )?;
        if next <= all[1].ordinal {
            return Err(format!(
                "next_from_ordinal {next} is not past the page's last ordinal {}",
                all[1].ordinal
            ));
        }
        let following = read(&session.route, &ReadRequest::range(next).with_limit(1)).await?;
        if following.messages.first() != Some(&all[2]) {
            return Err(format!(
                "reading from next_from_ordinal {next} answered {:?}, not ordinal {}",
                ordinals(&following.messages),
                all[2].ordinal
            ));
        }
        Ok(())
    }

    async fn range_read_stops_by_bytes(&self) -> Check {
        let (session, _) = self.long_session("bytes").await?;
        let all = self.all_messages(&session.route, 3).await?;
        let request = ReadRequest::range(all[0].ordinal)
            .with_limit(all.len() as u64 + 10)
            .with_max_bytes(1);
        let page = read(&session.route, &request).await?;
        if page.messages.len() != 1 {
            return Err(format!(
                "a read with a one-byte cap and a limit above the transcript answered {} messages, not one",
                page.messages.len()
            ));
        }
        if page.messages[0] != all[0] {
            return Err(format!(
                "the message read under a one-byte cap differs from the same message read normally: {:?} vs {:?}",
                page.messages[0], all[0]
            ));
        }
        if page.next_from_ordinal.is_none() {
            return Err("a page that stopped at its byte cap carries no next_from_ordinal".into());
        }
        Ok(())
    }

    async fn oversize_alone(&self) -> Check {
        let (small, _) = self.mint.text("answer");
        let (after, _) = self.mint.text("answer");
        let marker = self.mint.next("oversize");
        let big = format!("{marker}{}", "x".repeat(OVERSIZE_BYTES));
        let mut session = self
            .open(
                "oversize",
                Script {
                    turns: vec![
                        ScriptedTurn { parts: vec![small] },
                        ScriptedTurn { parts: vec![after] },
                    ],
                },
            )
            .await?;
        self.run_prompt(&mut session, &big).await?;
        let prompt = self.mint.next("prompt");
        self.run_prompt(&mut session, &prompt).await?;
        let all = self.all_messages(&session.route, 4).await?;
        let index = all
            .iter()
            .position(|m| m.message.to_string().contains(&big))
            .ok_or("no message holds the oversize prompt whole")?;
        let request = ReadRequest::range(all[index].ordinal)
            .with_limit(100)
            .with_max_bytes(OVERSIZE_CAP);
        let page = read(&session.route, &request).await?;
        if page.messages.len() != 1 {
            return Err(format!(
                "the {OVERSIZE_BYTES}-byte message read under a {OVERSIZE_CAP}-byte cap came with {} messages on its page",
                page.messages.len()
            ));
        }
        if page.messages[0] != all[index] || !page.messages[0].message.to_string().contains(&big) {
            return Err("the oversize message came truncated or changed".into());
        }
        if page.next_from_ordinal.is_none() {
            return Err(
                "the oversize message's page carries no next_from_ordinal though more follow"
                    .into(),
            );
        }
        Ok(())
    }

    async fn lineage(&self, route: &S::Route) -> Result<String, String> {
        read(route, &ReadRequest::tail())
            .await?
            .lineage_id
            .ok_or_else(|| "a written session's page carries no lineage_id".to_owned())
    }

    async fn after_mid_strictly_after(&self) -> Check {
        let (session, _) = self.long_session("after").await?;
        let all = self.all_messages(&session.route, 3).await?;
        let lineage = self.lineage(&session.route).await?;
        let page = read(&session.route, &ReadRequest::after(&all[0].mid, &lineage)).await?;
        if page.messages.first() != Some(&all[1]) {
            return Err(format!(
                "a read after mid {} answered ordinals {:?}; the next message is {}",
                all[0].mid,
                ordinals(&page.messages),
                all[1].ordinal
            ));
        }
        if page.messages.iter().any(|m| m.ordinal <= all[0].ordinal) {
            return Err("a read after a message answered that message or an earlier one".into());
        }
        let last = &all[all.len() - 1];
        let past = read(&session.route, &ReadRequest::after(&last.mid, &lineage)).await?;
        if !past.messages.is_empty() {
            return Err(format!(
                "a read after the newest message answered ordinals {:?}",
                ordinals(&past.messages)
            ));
        }
        Ok(())
    }

    async fn after_mid_unknown_mid(&self) -> Check {
        let (session, _) = self.long_session("unknown-mid").await?;
        let lineage = self.lineage(&session.route).await?;
        let request = ReadRequest::after(self.mint.next("no-such-mid"), &lineage);
        let error = read_refusal(&session.route, "a read after an unknown mid", &request).await?;
        expect_code("a read after an unknown mid", &error, errors::UNKNOWN_MID)
    }

    async fn read_other_lineage(&self) -> Check {
        let (session, _) = self.long_session("other-lineage").await?;
        let all = self.all_messages(&session.route, 1).await?;
        let other = self.mint.next("other-lineage");
        for (mode, request) in [
            ("tail", ReadRequest::tail().with_lineage_id(&other)),
            (
                "range",
                ReadRequest::range(all[0].ordinal).with_lineage_id(&other),
            ),
            ("after", ReadRequest::after(&all[0].mid, &other)),
        ] {
            let what = format!("a {mode} read naming another lineage");
            let error = read_refusal(&session.route, &what, &request).await?;
            expect_code(&what, &error, errors::LINEAGE_CHANGED)?;
        }
        Ok(())
    }

    async fn lineage_on_every_page(&self) -> Check {
        let (session, _) = self.long_session("lineage").await?;
        let head = head(&session.route).await?;
        let lineage = head
            .lineage_id
            .ok_or("session.head of a written session carries no lineage_id")?;
        let (mut pages, all) = read_all(&session.route).await?;
        let first = all.first().ok_or("the written session has no message")?;
        pages.push(read(&session.route, &ReadRequest::tail().with_limit(1)).await?);
        pages.push(
            read(
                &session.route,
                &ReadRequest::range(first.ordinal).with_limit(1),
            )
            .await?,
        );
        pages.push(read(&session.route, &ReadRequest::after(&first.mid, &lineage)).await?);
        for page in &pages {
            if page.lineage_id.as_deref() != Some(lineage.as_str()) {
                return Err(format!(
                    "a page carries lineage_id {:?}; the session's is {lineage}",
                    page.lineage_id
                ));
            }
        }
        Ok(())
    }

    async fn unknown_read_field(&self) -> Check {
        let session = self.open("unknown-field", Script::default()).await?;
        let error = expect_error(
            "session.read with from_ordinl",
            call(
                &session.route,
                ops::SESSION_READ,
                json!({ "from_ordinl": 0 }),
            )
            .await?,
        )?;
        expect_field(
            "session.read with from_ordinl",
            &error,
            errors::INVALID_PARAMS,
            "from_ordinl",
        )?;
        let field = "conformance_unknown_field";
        let error = expect_error(
            "session.head with an unknown field",
            call(&session.route, ops::SESSION_HEAD, json!({ field: true })).await?,
        )?;
        expect_field(
            "session.head with an unknown field",
            &error,
            errors::INVALID_PARAMS,
            field,
        )
    }

    async fn never_written_page(&self) -> Check {
        let session = self.open("unwritten", Script::default()).await?;
        let page = read(&session.route, &ReadRequest::tail()).await?;
        if page != ReadPage::no_lineage() {
            return Err(format!(
                "a read of a session never written answered {page:?}, not an empty page without lineage_id"
            ));
        }
        Ok(())
    }

    async fn never_written_head(&self) -> Check {
        let session = self.open("unwritten", Script::default()).await?;
        let head = head(&session.route).await?;
        if head != HeadMeta::no_lineage() {
            return Err(format!(
                "session.head of a session never written answered {head:?}, not every member absent"
            ));
        }
        Ok(())
    }

    async fn head_has_no_bodies(&self) -> Check {
        let (session, markers) = self.long_session("head").await?;
        let all = self.all_messages(&session.route, 1).await?;
        let raw = expect_ok(
            "session.head",
            call(&session.route, ops::SESSION_HEAD, json!({})).await?,
        )?;
        let head = head(&session.route).await?;
        if head.lineage_id != self.lineage(&session.route).await.ok() {
            return Err("session.head's lineage_id differs from the page's".into());
        }
        if head.last_ordinal != all.last().map(|m| m.ordinal) {
            return Err(format!(
                "session.head's last_ordinal is {:?}; the newest message is {:?}",
                head.last_ordinal,
                all.last().map(|m| m.ordinal)
            ));
        }
        if head.last_run_state.is_none() {
            return Err("session.head after three runs carries no last_run_state".into());
        }
        let text = raw.to_string();
        if raw.get("messages").is_some() || markers.iter().any(|m| text.contains(m.as_str())) {
            return Err(format!("session.head carries message bodies: {raw}"));
        }
        Ok(())
    }

    // ---- run ops ----------------------------------------------------------

    /// Run one prompt through a session whose model is `turns`, and answer
    /// its `run.result`.
    async fn completed_result(
        &self,
        label: &str,
        turns: Vec<ScriptedTurn>,
    ) -> Result<cortexkit_role_llm_runner::run::RunResult, String> {
        let mut session = self.open(label, Script { turns }).await?;
        let prompt = self.mint.next("prompt");
        let (_, end) = self.run_prompt(&mut session, &prompt).await?;
        let result = run_result(&session.route, &end.run_id).await?;
        if result.state != run_states::COMPLETED {
            return Err(format!(
                "the scripted run ended {}, not completed",
                result.state
            ));
        }
        Ok(result)
    }

    async fn run_result_final_text(&self) -> Check {
        let (interim, interim_marker) = self.mint.text("interim");
        let (last, last_marker) = self.mint.text("final");
        let call = self.mint.call("call_final");
        let result = self
            .completed_result(
                "final-text",
                vec![
                    ScriptedTurn {
                        parts: vec![interim, ScriptedPart::ToolCall(call)],
                    },
                    ScriptedTurn { parts: vec![last] },
                ],
            )
            .await?;
        let text = final_text(&result)?;
        if text != last_marker {
            return Err(format!(
                "final_message.text is {text:?}; the final turn's text is {last_marker:?} (the earlier turn's is {interim_marker:?})"
            ));
        }
        Ok(())
    }

    async fn run_result_joined(&self) -> Check {
        let marker = self.mint.next("joined");
        let reasoning = self.mint.next("reasoning");
        let parts = [
            "{\"answer\":\"".to_owned(),
            marker.clone(),
            "\",\"n\":1}".to_owned(),
        ];
        let result = self
            .completed_result(
                "joined",
                vec![ScriptedTurn {
                    parts: vec![
                        ScriptedPart::Text(parts[0].clone()),
                        ScriptedPart::Reasoning(reasoning.clone()),
                        ScriptedPart::Text(parts[1].clone()),
                        ScriptedPart::Text(parts[2].clone()),
                    ],
                }],
            )
            .await?;
        let text = final_text(&result)?;
        let expected = join_text_parts(&parts);
        if text != expected {
            return Err(format!(
                "final_message.text is {text:?}; the text parts joined with nothing inserted are {expected:?}"
            ));
        }
        if serde_json::from_str::<Value>(&text).is_err() {
            return Err(format!(
                "the joined text {text:?} is not the JSON value the parts split"
            ));
        }
        Ok(())
    }

    async fn run_result_no_text(&self) -> Check {
        let reasoning = self.mint.next("reasoning");
        let result = self
            .completed_result(
                "no-text",
                vec![ScriptedTurn {
                    parts: vec![ScriptedPart::Reasoning(reasoning)],
                }],
            )
            .await?;
        let text = final_text(&result)?;
        if !text.is_empty() {
            return Err(format!(
                "a final message with only reasoning answered text {text:?}, not \"\""
            ));
        }
        Ok(())
    }

    // ---- dispatch attribution ---------------------------------------------

    /// Run one prompt through `turns` and answer the transcript.
    async fn dispatched(
        &self,
        label: &str,
        turns: Vec<ScriptedTurn>,
    ) -> Result<Vec<ReadMessage>, String> {
        let mut session = self.open(label, Script { turns }).await?;
        let prompt = self.mint.next("prompt");
        self.run_prompt(&mut session, &prompt).await?;
        self.all_messages(&session.route, 1).await
    }

    async fn dispatched_to_per_call(&self) -> Check {
        let calls = [
            self.mint.call("dispatch_a"),
            self.mint.call("dispatch_b"),
            self.mint.call("dispatch_c"),
        ];
        let (done, _) = self.mint.text("done");
        let all = self
            .dispatched(
                "dispatched",
                vec![
                    ScriptedTurn {
                        parts: vec![
                            ScriptedPart::ToolCall(calls[0].clone()),
                            ScriptedPart::ToolCall(calls[1].clone()),
                        ],
                    },
                    ScriptedTurn {
                        parts: vec![ScriptedPart::ToolCall(calls[2].clone())],
                    },
                    ScriptedTurn { parts: vec![done] },
                ],
            )
            .await?;
        let module = self.subject.tool_provider_module();
        for call in &calls {
            let entries = entries_for(&all, &call.tool_call_id);
            let [entry] = entries.as_slice() else {
                return Err(format!(
                    "tool call {} has {} tool_calls entries, not one",
                    call.tool_call_id,
                    entries.len()
                ));
            };
            if entry.dispatched_to.as_deref() != Some(module.as_str()) {
                return Err(format!(
                    "tool call {} names dispatched_to {:?}, not {module}",
                    call.tool_call_id, entry.dispatched_to
                ));
            }
            match &entry.call_key {
                Some(key) if call_key_well_formed(key) => {}
                other => {
                    return Err(format!(
                        "tool call {} carries call_key {other:?}, not a well-formed key",
                        call.tool_call_id
                    ))
                }
            }
            if entry.indeterminate {
                return Err(format!(
                    "tool call {} is indeterminate after its run ended",
                    call.tool_call_id
                ));
            }
            let invoked = self.subject.tool_invocations(&call.arguments).await;
            if invoked != 1 {
                return Err(format!(
                    "the scripted tool provider ran tool call {} {invoked} times",
                    call.tool_call_id
                ));
            }
        }
        Ok(())
    }

    async fn indeterminate_until_closed(&self) -> Check {
        let mut held = self.mint.call("held_call");
        held.hold = true;
        let (done, _) = self.mint.text("done");
        let session = self
            .open(
                "indeterminate",
                Script {
                    turns: vec![
                        ScriptedTurn {
                            parts: vec![ScriptedPart::ToolCall(held.clone())],
                        },
                        ScriptedTurn { parts: vec![done] },
                    ],
                },
            )
            .await?;
        let prompt = self.mint.next("prompt");
        let send_id = self.mint.next("send");
        let module = self.subject.tool_provider_module();
        let probe = async {
            let observed = self
                .observe_indeterminate(&session.route, &held, &module)
                .await;
            let released = self
                .subject
                .release_tool_call(&held.arguments)
                .await
                .map_err(|e| format!("releasing the held call: {e}"));
            observed.and(released)
        };
        let (sent, probed) = join(self.send_raw(&session, &prompt, &send_id, None), probe).await;
        probed?;
        let reply: SendReply = decode("session.send", expect_ok("session.send", sent?)?)?;
        wait_run_end(self.subject, &session.route, self.declared, None, &reply).await?;
        let all = self.all_messages(&session.route, 1).await?;
        let entries = entries_for(&all, &held.tool_call_id);
        match entries.as_slice() {
            [entry] if !entry.indeterminate => Ok(()),
            other => Err(format!(
                "after its result is in, the call's entries are {other:?}, not one entry that is no longer indeterminate"
            )),
        }
    }

    /// Wait until the tool provider holds `call`, then until a read shows
    /// it indeterminate with its key and module.
    async fn observe_indeterminate(
        &self,
        route: &S::Route,
        call: &crate::subject::ScriptedToolCall,
        module: &str,
    ) -> Check {
        self.subject
            .await_tool_call(&call.arguments)
            .await
            .map_err(|e| format!("the tool provider never held the call: {e}"))?;
        for _ in 0..MAX_POLLS {
            let (_, all) = read_all(route).await?;
            if let [entry] = entries_for(&all, &call.tool_call_id).as_slice() {
                if entry.indeterminate {
                    if entry.call_key.is_none() || entry.dispatched_to.as_deref() != Some(module) {
                        return Err(format!(
                            "the held call is indeterminate but its entry is {entry:?}, without its call_key or dispatched_to {module}"
                        ));
                    }
                    return Ok(());
                }
                return Err(format!(
                    "the call the tool provider holds is read as {entry:?}, not indeterminate"
                ));
            }
            self.subject.pause().await;
        }
        Err("the held call never appeared on a page".into())
    }

    async fn sibling_call_keys(&self) -> Check {
        let first = self.mint.call("sibling_a");
        let second = self.mint.call("sibling_b");
        let (done, _) = self.mint.text("done");
        let all = self
            .dispatched(
                "siblings",
                vec![
                    ScriptedTurn {
                        parts: vec![
                            ScriptedPart::ToolCall(first.clone()),
                            ScriptedPart::ToolCall(second.clone()),
                        ],
                    },
                    ScriptedTurn { parts: vec![done] },
                ],
            )
            .await?;
        let keys: Vec<Option<String>> = [&first, &second]
            .iter()
            .flat_map(|call| entries_for(&all, &call.tool_call_id))
            .map(|entry| entry.call_key.clone())
            .collect();
        distinct_keys("two sibling calls", &keys)
    }

    async fn recurring_call_keys(&self) -> Check {
        let first = self.mint.call("call_0");
        let second = self.mint.call("call_0");
        let (done, _) = self.mint.text("done");
        let all = self
            .dispatched(
                "recurring",
                vec![
                    ScriptedTurn {
                        parts: vec![ScriptedPart::ToolCall(first)],
                    },
                    ScriptedTurn {
                        parts: vec![ScriptedPart::ToolCall(second)],
                    },
                    ScriptedTurn { parts: vec![done] },
                ],
            )
            .await?;
        let keys: Vec<Option<String>> = entries_for(&all, "call_0")
            .into_iter()
            .map(|entry| entry.call_key.clone())
            .collect();
        distinct_keys("one model id used in two steps", &keys)
    }

    // ---- streaming ----------------------------------------------------------

    async fn control_cursors(
        &self,
        route: &S::Route,
        from: SubscribeFrom,
    ) -> Result<Vec<Map<String, Value>>, String> {
        let params = serde_json::to_value(SubscribeRequest::new(from)).expect("serializes");
        let events = match route
            .subscribe(params)
            .await
            .map_err(|e| format!("session.subscribe: the route failed: {}", e.message))?
        {
            SubscribeOutcome::Refused(error) => {
                return Err(format!(
                    "session.subscribe was refused {} {:?}",
                    error.code, error.detail
                ))
            }
            SubscribeOutcome::Events(events) => events,
        };
        let mut cursors = Vec::new();
        for event in events {
            let event: SubscribeEvent = decode("a session.subscribe event", event)?;
            if event.kind != kinds::CONTROL {
                continue;
            }
            cursors.push(
                event.cursor.ok_or_else(|| {
                    format!("a control event carries no cursor: {:?}", event.body)
                })?,
            );
        }
        Ok(cursors)
    }

    async fn subscribe_from_head(&self) -> Check {
        let (one, _) = self.mint.text("answer");
        let (two, _) = self.mint.text("answer");
        let mut session = self
            .open(
                "subscribe",
                Script {
                    turns: vec![
                        ScriptedTurn { parts: vec![one] },
                        ScriptedTurn { parts: vec![two] },
                    ],
                },
            )
            .await?;
        let prompt = self.mint.next("prompt");
        self.run_prompt(&mut session, &prompt).await?;
        let page = read(&session.route, &ReadRequest::tail()).await?;
        let page_head = page
            .head
            .ok_or("a page of a session with events carries no head")?;
        let covered = self
            .control_cursors(&session.route, SubscribeFrom::Start)
            .await?;
        let prompt = self.mint.next("prompt");
        self.run_prompt(&mut session, &prompt).await?;
        let everything = self
            .control_cursors(&session.route, SubscribeFrom::Start)
            .await?;
        let from_head = self
            .control_cursors(&session.route, SubscribeFrom::After(page_head.clone()))
            .await?;
        if everything.len() < covered.len() || everything[..covered.len()] != covered[..] {
            return Err("a second replay from start does not begin with the first one".into());
        }
        let after = &everything[covered.len()..];
        if after.is_empty() {
            return Err("the second run added no durable event to the stream".into());
        }
        if from_head != after {
            return Err(format!(
                "a subscription from the page's head {page_head:?} answered cursors {from_head:?}; the events after the page's snapshot are {after:?}"
            ));
        }
        Ok(())
    }

    // ---- sends ------------------------------------------------------------

    /// A session with one completed run. Returns it with the first send's
    /// id and prompt.
    async fn sent_session(
        &self,
        label: &str,
    ) -> Result<(Session<S::Route>, String, String), String> {
        let (part, _) = self.mint.text("answer");
        let (part2, _) = self.mint.text("answer");
        let mut session = self
            .open(
                label,
                Script {
                    turns: vec![
                        ScriptedTurn { parts: vec![part] },
                        ScriptedTurn { parts: vec![part2] },
                    ],
                },
            )
            .await?;
        let prompt = self.mint.next("prompt");
        self.run_prompt(&mut session, &prompt).await?;
        let (send_id, prompt) = session.first.clone().expect("the first send is recorded");
        Ok((session, send_id, prompt))
    }

    /// Send, and check the head did not move, however the send was
    /// answered. Returns the answer.
    async fn writes_nothing(
        &self,
        what: &str,
        session: &Session<S::Route>,
        prompt: &str,
        send_id: &str,
        delivery: Option<&str>,
    ) -> Result<crate::route::Reply, String> {
        let before = head(&session.route).await?;
        let reply = self.send_raw(session, prompt, send_id, delivery).await?;
        let after = head(&session.route).await?;
        if after != before {
            let answered = match &reply {
                crate::route::Reply::Error(error) => format!("refused {}", error.code),
                crate::route::Reply::Response(_) => "answered".to_owned(),
            };
            return Err(format!(
                "{what} was {answered} but the session changed: head {before:?} then {after:?}"
            ));
        }
        Ok(reply)
    }

    /// Send, expect a refusal, and check the head did not move.
    async fn refused_writes_nothing(
        &self,
        what: &str,
        session: &Session<S::Route>,
        prompt: &str,
        send_id: &str,
        delivery: Option<&str>,
    ) -> Result<subc_protocol::ErrorBody, String> {
        expect_error(
            what,
            self.writes_nothing(what, session, prompt, send_id, delivery)
                .await?,
        )
    }

    // The `send_id` cases come in two halves. The reply half checks only
    // what `session.send` answers, never reads the transcript, and so runs
    // on any runner that takes a send. The written-once half checks what
    // the send wrote, through `session.read` and `session.head`, and is
    // inapplicable on a runner without `transcript_reads`.

    /// The `delivery` of a reply half's first send: absent (`queue`) when
    /// the runner declares `queue`, otherwise `steer`, which with no run
    /// active is a plain send. `None` when the runner declares neither.
    fn reply_half_delivery(&self) -> Option<Option<&'static str>> {
        if self.declares(Capability::Queue) {
            Some(None)
        } else if self.declares(Capability::Steer) {
            Some(Some(groups::STEER))
        } else {
            None
        }
    }

    /// `Some` with the inapplicable ending of a written-once half, when the
    /// runner does not declare `transcript_reads`.
    fn without_transcript(&self) -> Option<Ending> {
        (!self.declares(Capability::TranscriptReads))
            .then(|| Ending::Inapplicable(NO_TRANSCRIPT.to_owned()))
    }

    /// A fresh session with one send of a fresh prompt, made with
    /// `delivery` and not waited on: a reply half never reads the
    /// transcript, so it cannot wait through it. Returns the session, the
    /// send's id and prompt, and its reply.
    async fn sent_unread(
        &self,
        label: &str,
        delivery: Option<&str>,
    ) -> Result<(Session<S::Route>, String, String, SendReply), String> {
        let (part, _) = self.mint.text("answer");
        let mut session = self
            .open(
                label,
                Script {
                    turns: vec![ScriptedTurn { parts: vec![part] }],
                },
            )
            .await?;
        let send_id = self.mint.next("send");
        let prompt = self.mint.next("prompt");
        let reply = expect_ok(
            "session.send",
            self.send_raw(&session, &prompt, &send_id, delivery).await?,
        )?;
        session.first = Some((send_id.clone(), prompt.clone()));
        let reply = decode("session.send", reply)?;
        Ok((session, send_id, prompt, reply))
    }

    /// Wait, without reading the transcript, until run `run_id` has a
    /// terminal state, through `run.result`. Only the case that requires
    /// `run_ops` calls this, so a runner without it is never asked. Bounded
    /// by [`MAX_POLLS`], so a run that never ends fails by name.
    async fn settle_unread(&self, route: &S::Route, run_id: &str) -> Check {
        for _ in 0..MAX_POLLS {
            if run_result(route, run_id).await?.run_state().is_terminal() {
                return Ok(());
            }
            self.subject.pause().await;
        }
        Err(format!("run {run_id} did not end within {MAX_POLLS} polls"))
    }

    /// Retry a send twice with the same `send_id` and payload, answering
    /// the two retries' replies.
    async fn retried_twice(
        &self,
        session: &Session<S::Route>,
        prompt: &str,
        send_id: &str,
        delivery: Option<&str>,
    ) -> Result<(SendReply, SendReply), String> {
        let first = self
            .retry_once("the first retry", session, prompt, send_id, delivery)
            .await?;
        let second = self
            .retry_once("the second retry", session, prompt, send_id, delivery)
            .await?;
        Ok((first, second))
    }

    async fn retry_once(
        &self,
        what: &str,
        session: &Session<S::Route>,
        prompt: &str,
        send_id: &str,
        delivery: Option<&str>,
    ) -> Result<SendReply, String> {
        let reply = expect_ok(
            what,
            self.send_raw(session, prompt, send_id, delivery).await?,
        )?;
        decode(what, reply)
    }

    /// Retries right after the send, never waiting: the send's state may
    /// still move between them, so only the ids and the forward-only
    /// `delivered` rule are checked.
    async fn send_id_retry(&self) -> Result<Ending, String> {
        let Some(delivery) = self.reply_half_delivery() else {
            return Ok(Ending::Inapplicable(NO_SEND_MODE.to_owned()));
        };
        let (session, send_id, prompt, original) = self.sent_unread("retry", delivery).await?;
        let (first, second) = self
            .retried_twice(&session, &prompt, &send_id, delivery)
            .await?;
        same_answer(&original, &first, &second, false)?;
        Ok(Ending::Passed)
    }

    /// Retries once the send's run has ended, seen through `run.result`
    /// (the case requires `run_ops`): nothing may move between them any
    /// more, so the two retries must be the same answer apart from
    /// `delivered`.
    async fn send_id_retry_settled(&self) -> Result<Ending, String> {
        let Some(delivery) = self.reply_half_delivery() else {
            return Ok(Ending::Inapplicable(NO_SEND_MODE.to_owned()));
        };
        let (session, send_id, prompt, original) =
            self.sent_unread("retry-settled", delivery).await?;
        let Some(run_id) = original.run_id.clone() else {
            return Ok(Ending::Inapplicable(NO_RUN_TO_SETTLE.to_owned()));
        };
        self.settle_unread(&session.route, &run_id).await?;
        let (first, second) = self
            .retried_twice(&session, &prompt, &send_id, delivery)
            .await?;
        same_answer(&original, &first, &second, true)?;
        Ok(Ending::Passed)
    }

    async fn send_id_retry_written_once(&self) -> Result<Ending, String> {
        if let Some(ending) = self.without_transcript() {
            return Ok(ending);
        }
        let (session, send_id, prompt) = self.sent_session("retry-written").await?;
        let last = self.last_run_id(&session.route).await?;
        for what in ["the first retry", "the second retry"] {
            let reply: SendReply = decode(
                what,
                expect_ok(
                    what,
                    self.send_raw(&session, &prompt, &send_id, None).await?,
                )?,
            )?;
            if reply.run_id.is_some() && reply.run_id != last {
                return Err(format!(
                    "{what} names run {:?}; the send started run {last:?}",
                    reply.run_id
                ));
            }
        }
        if self.last_run_id(&session.route).await? != last {
            return Err("a retried send started another run".into());
        }
        let (_, all) = read_all(&session.route).await?;
        let copies = holding(&all, &prompt);
        if copies != 1 {
            return Err(format!(
                "the retried prompt is written in {copies} messages, not one"
            ));
        }
        Ok(Ending::Passed)
    }

    async fn send_id_reuse(&self) -> Result<Ending, String> {
        let Some(delivery) = self.reply_half_delivery() else {
            return Ok(Ending::Inapplicable(NO_SEND_MODE.to_owned()));
        };
        let (session, send_id, _, _) = self.sent_unread("reuse", delivery).await?;
        let other = self.mint.next("other-prompt");
        let what = "a send_id reused with another prompt";
        let error = expect_error(
            what,
            self.send_raw(&session, &other, &send_id, delivery).await?,
        )?;
        expect_field(what, &error, errors::SEND_ID_REUSE, "prompt")?;
        Ok(Ending::Passed)
    }

    async fn send_id_reuse_writes_nothing(&self) -> Result<Ending, String> {
        if let Some(ending) = self.without_transcript() {
            return Ok(ending);
        }
        let (session, send_id, _) = self.sent_session("reuse-written").await?;
        let other = self.mint.next("other-prompt");
        self.writes_nothing(
            "a send_id reused with another prompt",
            &session,
            &other,
            &send_id,
            None,
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn delivery_change(&self) -> Result<Ending, String> {
        let Some(delivery) = self.reply_half_delivery() else {
            return Ok(Ending::Inapplicable(NO_SEND_MODE.to_owned()));
        };
        let first_mode = delivery.unwrap_or(groups::QUEUE);
        let Some(mode) = [Capability::Steer, Capability::Interrupt]
            .into_iter()
            .filter(|mode| self.declares(*mode))
            .filter_map(Capability::group)
            .find(|mode| *mode != first_mode)
        else {
            return Ok(Ending::Inapplicable(format!(
                "the runner declares no delivery mode other than {first_mode} to change to"
            )));
        };
        let (session, send_id, prompt, _) = self.sent_unread("delivery-change", delivery).await?;
        let what = format!("a send_id reused with delivery {mode}");
        let error = expect_error(
            &what,
            self.send_raw(&session, &prompt, &send_id, Some(mode))
                .await?,
        )?;
        expect_field(&what, &error, errors::SEND_ID_REUSE, DELIVERY_FIELD)?;
        Ok(Ending::Passed)
    }

    async fn delivery_change_writes_nothing(&self) -> Result<Ending, String> {
        if let Some(ending) = self.without_transcript() {
            return Ok(ending);
        }
        let mode = if self.declares(Capability::Steer) {
            groups::STEER
        } else {
            groups::INTERRUPT
        };
        let (session, send_id, prompt) = self.sent_session("delivery-change-written").await?;
        self.writes_nothing(
            &format!("a send_id reused with delivery {mode}"),
            &session,
            &prompt,
            &send_id,
            Some(mode),
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn unknown_delivery(&self) -> Check {
        let (session, _, _) = self.sent_session("unknown-delivery").await?;
        let what = "a send with an unknown delivery mode";
        let error = self
            .refused_writes_nothing(
                what,
                &session,
                &self.mint.next("prompt"),
                &self.mint.next("send"),
                Some("conformance_broadcast"),
            )
            .await?;
        expect_field(what, &error, errors::INVALID_PARAMS, DELIVERY_FIELD)
    }

    async fn undeclared_delivery(&self) -> Check {
        let mode = [Capability::Steer, Capability::Interrupt]
            .into_iter()
            .find(|mode| !self.declares(*mode))
            .and_then(Capability::group)
            .ok_or("the subject declares every delivery mode")?;
        let (session, _, _) = self.sent_session("undeclared-delivery").await?;
        let what = format!("a send with the undeclared delivery {mode}");
        let error = self
            .refused_writes_nothing(
                &what,
                &session,
                &self.mint.next("prompt"),
                &self.mint.next("send"),
                Some(mode),
            )
            .await?;
        expect_code(&what, &error, errors::DELIVERY_UNSUPPORTED)?;
        let named = error
            .detail
            .as_ref()
            .and_then(|detail| detail.get(DELIVERY_FIELD))
            .and_then(Value::as_str);
        if named != Some(mode) {
            return Err(format!(
                "delivery_unsupported names detail {:?}, not delivery {mode}",
                error.detail
            ));
        }
        Ok(())
    }

    async fn guaranteed_steer_never_pending_or_unknown(&self) -> Result<Ending, String> {
        let describe: RoleDescribe = decode("role.describe", self.describe().await?)?;
        if !describe.declares(groups::STEER) || !self.declares(Capability::Steer) {
            return Ok(Ending::Inapplicable(
                "the runner does not declare steer".into(),
            ));
        }
        if describe.steer_receipt() == steer_receipt::CONFIRM {
            return Ok(Ending::Inapplicable(
                "the runner declares steer_receipt: confirm".into(),
            ));
        }
        if !self.declares(Capability::HoldToolCalls) {
            return Ok(Ending::Inapplicable(
                "the suite cannot hold a running turn without hold_tool_calls".into(),
            ));
        }
        // Keep the turn running by holding its tool call, the same way the
        // check for sends with an unknown delivery outcome does. The send and
        // the probe run together, so a runner whose first send only replies
        // once its run ends is supported too.
        let mut held = self.mint.call("held_steer_call");
        held.hold = true;
        let (done, _) = self.mint.text("done");
        let mut session = self
            .open(
                "steer-guaranteed",
                Script {
                    turns: vec![
                        ScriptedTurn {
                            parts: vec![ScriptedPart::ToolCall(held.clone())],
                        },
                        ScriptedTurn { parts: vec![done] },
                    ],
                },
            )
            .await?;
        let send_id = self.mint.next("send");
        let prompt = self.mint.next("prompt");
        session.first = Some((send_id.clone(), prompt.clone()));
        let steer_id = self.mint.next("steer");
        let steer_prompt = self.mint.next("prompt");
        let probe = async {
            let observed = async {
                self.subject
                    .await_tool_call(&held.arguments)
                    .await
                    .map_err(|e| format!("the tool provider never held the call: {e}"))?;
                let running = head(&session.route)
                    .await?
                    .last_run_state
                    .ok_or("the held turn has no run state")?;
                if running.state != run_states::ACTIVE {
                    return Err(format!("the held turn is {}, not active", running.state));
                }
                let mut replies = Vec::new();
                for what in ["session.send steer", "session.send steer re-send"] {
                    let reply: SendReply = decode(
                        what,
                        expect_ok(
                            what,
                            self.send_raw(&session, &steer_prompt, &steer_id, Some(groups::STEER))
                                .await?,
                        )?,
                    )?;
                    let after = head(&session.route).await?.last_run_state;
                    if !matches!(&after, Some(run) if run.run_id == running.run_id && run.state == run_states::ACTIVE)
                    {
                        return Err(format!(
                            "{what} was answered without the held run {} still in progress: {after:?}",
                            running.run_id
                        ));
                    }
                    if reply.run_id.as_deref() != Some(running.run_id.as_str()) {
                        return Err(format!(
                            "{what} names run {:?}, not the held running turn {}",
                            reply.run_id, running.run_id
                        ));
                    }
                    if let Some(delivered) = &reply.delivered {
                        if !delivered.is_delivered() {
                            return Err(format!(
                                "a guaranteed runner answered delivered.as: {} on {what}",
                                delivered.r#as
                            ));
                        }
                    }
                    replies.push(reply);
                }
                // Nothing can render the steer while its tool call is held.
                // Either answer may omit the receipt, but receipts already
                // supplied are final, opaque reference included.
                if let (Some(first), Some(retry)) = (&replies[0].delivered, &replies[1].delivered) {
                    if first != retry {
                        return Err(format!(
                            "re-send delivered changed from {first:?} to {retry:?}"
                        ));
                    }
                }
                Ok((replies, running.run_id))
            }
            .await;
            // Always release the held tool call, even when a receipt check or
            // the check that the turn is still running failed, so the suite
            // never leaves a run stalled.
            let released = self
                .subject
                .release_tool_call(&held.arguments)
                .await
                .map_err(|e| format!("releasing the held call: {e}"));
            let observed = observed?;
            released?;
            Ok::<_, String>(observed)
        };
        let (sent, probed) = join(self.send_raw(&session, &prompt, &send_id, None), probe).await;
        let reply: SendReply = decode("session.send", expect_ok("session.send", sent?)?)?;
        wait_run_end(self.subject, &session.route, self.declared, None, &reply).await?;
        let (held_replies, held_run_id) = probed?;
        let what = "session.send steer final re-send";
        let final_reply: SendReply = decode(
            what,
            expect_ok(
                what,
                self.send_raw(&session, &steer_prompt, &steer_id, Some(groups::STEER))
                    .await?,
            )?,
        )?;
        let final_receipt = final_reply
            .delivered
            .as_ref()
            .ok_or("a guaranteed runner omitted delivered on final re-send after the run ended")?;
        if !final_receipt.is_delivered() {
            return Err(format!(
                "a guaranteed runner answered delivered.as: {} on {what}",
                final_receipt.r#as
            ));
        }
        for earlier in held_replies
            .iter()
            .filter_map(|reply| reply.delivered.as_ref())
        {
            if earlier != final_receipt {
                return Err(format!(
                    "final re-send delivered changed from {earlier:?} to {final_receipt:?}"
                ));
            }
        }
        // A turn receipt names a run the suite observed, not necessarily the
        // held run: the runner may render the steer in a later episode. Step
        // references are opaque stored row ids, not necessarily transcript mids.
        let latest = head(&session.route).await?.last_run_state;
        for receipt in held_replies
            .iter()
            .filter_map(|reply| reply.delivered.as_ref())
            .chain(std::iter::once(final_receipt))
        {
            if receipt.r#as() == DeliveredAs::Turn {
                if let Some(reference) = &receipt.r#ref {
                    if reference != &held_run_id
                        && !matches!(&latest, Some(run) if &run.run_id == reference)
                    {
                        return Err(format!(
                            "delivered turn ref {reference:?} names no observed run: held {held_run_id}, latest {latest:?}"
                        ));
                    }
                }
            }
        }
        Ok(Ending::Passed)
    }

    async fn resend_steer_delivered_stable(&self) -> Result<Ending, String> {
        let describe: RoleDescribe = decode("role.describe", self.describe().await?)?;
        if !describe.declares(groups::STEER) || !self.declares(Capability::Steer) {
            return Ok(Ending::Inapplicable(
                "the runner does not declare steer".into(),
            ));
        }
        let (session, _, _) = self.sent_session("steer-stable").await?;
        let steer_id = self.mint.next("steer");
        let prompt = self.mint.next("prompt");
        let first_reply_raw = expect_ok(
            "session.send steer",
            self.send_raw(&session, &prompt, &steer_id, Some(groups::STEER))
                .await?,
        )?;
        let first_reply: SendReply = decode("session.send steer", first_reply_raw)?;
        let retry1_raw = expect_ok(
            "session.send steer retry 1",
            self.send_raw(&session, &prompt, &steer_id, Some(groups::STEER))
                .await?,
        )?;
        let retry1: SendReply = decode("session.send steer retry 1", retry1_raw)?;
        let retry2_raw = expect_ok(
            "session.send steer retry 2",
            self.send_raw(&session, &prompt, &steer_id, Some(groups::STEER))
                .await?,
        )?;
        let retry2: SendReply = decode("session.send steer retry 2", retry2_raw)?;

        let receipts = [
            first_reply.delivered.as_ref(),
            retry1.delivered.as_ref(),
            retry2.delivered.as_ref(),
        ];
        for pair in receipts.windows(2) {
            check_delivered_move(pair[0], pair[1]).map_err(|e| format!("re-send {e}"))?;
        }
        Ok(Ending::Passed)
    }
}

/// Whether a re-send's `delivered` receipt may follow `before`, the receipt
/// an earlier answer to the same `send_id` carried. A receipt moves only
/// forward: from absent or `pending` to `step`, `turn` or `unknown`. It may
/// also go from absent to `pending`, because on a `confirm` runner an
/// absent receipt already means `pending` and a re-send states it. `step`,
/// `turn` and `unknown` are final: once one appears, every later answer
/// carries it unchanged, `ref` included. Any other change, to or from an
/// `as` the role does not define included, is refused.
///
/// The one rule both the retry case and the steer re-send case apply.
pub(crate) fn delivered_move_allowed(
    before: Option<&Delivered>,
    after: Option<&Delivered>,
) -> bool {
    if before == after {
        return true;
    }
    let Some(after) = after else {
        return false;
    };
    match before.map(Delivered::r#as) {
        None => is_final(after) || after.r#as() == DeliveredAs::Pending,
        Some(DeliveredAs::Pending) => is_final(after),
        Some(_) => false,
    }
}

/// `step`, `turn` and `unknown`: the receipts that never change once given.
fn is_final(receipt: &Delivered) -> bool {
    matches!(
        receipt.r#as(),
        DeliveredAs::Step | DeliveredAs::Turn | DeliveredAs::Unknown
    )
}

/// [`delivered_move_allowed`], failing with both receipts named.
fn check_delivered_move(before: Option<&Delivered>, after: Option<&Delivered>) -> Check {
    if delivered_move_allowed(before, after) {
        return Ok(());
    }
    let why = match before {
        Some(before) if is_final(before) => {
            format!("{} is final and never changes, ref included", before.r#as)
        }
        _ => "a receipt moves only forward, from absent or pending to step, turn or unknown"
            .to_owned(),
    };
    Err(format!(
        "delivered changed from {} to {}: {why}",
        show_receipt(before),
        show_receipt(after)
    ))
}

fn show_receipt(receipt: Option<&Delivered>) -> String {
    match receipt {
        None => "absent".to_owned(),
        Some(receipt) => match &receipt.r#ref {
            None => receipt.r#as.clone(),
            Some(id) => format!("{} (ref {id})", receipt.r#as),
        },
    }
}

/// Check the answers to a send and to its two retries with the same
/// `send_id` and payload. Each retry names the `run_id` and
/// `submission_id` the earlier answers named (a retry may name one an
/// earlier answer had not yet), and `delivered` moves only forward
/// ([`delivered_move_allowed`]). When `settled`, the send's run had ended
/// before the retries, so nothing may move between them: the two retries
/// are the same answer, apart from `delivered`. Only the case that saw the
/// run end through `run.result` passes `settled`.
fn same_answer(
    original: &SendReply,
    first: &SendReply,
    second: &SendReply,
    settled: bool,
) -> Check {
    let answers = [
        ("the send", original),
        ("the first retry", first),
        ("the second retry", second),
    ];
    for (i, (earlier_name, earlier)) in answers.iter().enumerate() {
        for (later_name, later) in &answers[i + 1..] {
            for (field, a, b) in [
                ("run_id", &earlier.run_id, &later.run_id),
                (
                    "submission_id",
                    &earlier.submission_id,
                    &later.submission_id,
                ),
            ] {
                if let (Some(a), Some(b)) = (a, b) {
                    if a != b {
                        return Err(format!(
                            "{later_name} names {field} {b}; {earlier_name} named {a}"
                        ));
                    }
                }
            }
        }
    }
    if original.run_id.is_some() && (first.run_id.is_none() || second.run_id.is_none()) {
        return Err(format!(
            "the send named run {:?}, but a retry names none",
            original.run_id
        ));
    }
    check_delivered_move(original.delivered.as_ref(), first.delivered.as_ref())
        .map_err(|e| format!("the first retry's {e}"))?;
    check_delivered_move(first.delivered.as_ref(), second.delivered.as_ref())
        .map_err(|e| format!("the second retry's {e}"))?;
    if settled {
        let without_receipt = |reply: &SendReply| {
            let mut reply = reply.clone();
            reply.delivered = None;
            reply
        };
        if without_receipt(first) != without_receipt(second) {
            return Err(format!(
                "two retries of one send, after its run ended, answered {first:?} and {second:?}"
            ));
        }
    }
    Ok(())
}

fn ordinals(messages: &[ReadMessage]) -> Vec<u64> {
    messages.iter().map(|m| m.ordinal).collect()
}

fn final_text(result: &cortexkit_role_llm_runner::run::RunResult) -> Result<String, String> {
    result
        .final_message
        .as_ref()
        .map(|message| message.text.clone())
        .ok_or_else(|| "a completed run answers no final_message".to_owned())
}

/// Every `tool_calls` entry with the model id `tool_call_id`, in transcript
/// order.
pub fn entries_for<'a>(
    messages: &'a [ReadMessage],
    tool_call_id: &str,
) -> Vec<&'a ToolCallAttribution> {
    messages
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .filter(|entry| entry.tool_call_id == tool_call_id)
        .collect()
}

fn distinct_keys(what: &str, keys: &[Option<String>]) -> Check {
    if keys.len() != 2 {
        return Err(format!(
            "{what}: expected two tool_calls entries, found {}",
            keys.len()
        ));
    }
    match (&keys[0], &keys[1]) {
        (Some(a), Some(b)) if a != b => Ok(()),
        (Some(a), Some(_)) => Err(format!("{what} share call_key {a}")),
        _ => Err(format!("{what}: a call carries no call_key ({keys:?})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_role_llm_runner::send::delivered_as;

    fn receipt(r#as: &str) -> Option<Delivered> {
        Some(Delivered::new(r#as))
    }

    fn allowed(before: &Option<Delivered>, after: &Option<Delivered>) -> bool {
        delivered_move_allowed(before.as_ref(), after.as_ref())
    }

    #[test]
    fn a_receipt_may_move_forward_or_stay() {
        let open = [None, receipt(delivered_as::PENDING)];
        let finals = [
            receipt(delivered_as::STEP),
            Some(Delivered::new(delivered_as::TURN).with_ref("run-1")),
            receipt(delivered_as::UNKNOWN),
        ];
        for before in &open {
            for after in &finals {
                assert!(allowed(before, after), "{before:?} -> {after:?}");
            }
            assert!(allowed(before, before), "{before:?} unchanged");
        }
        for receipt in &finals {
            assert!(allowed(receipt, receipt), "{receipt:?} unchanged");
        }
        assert!(allowed(&None, &receipt(delivered_as::PENDING)));
        assert!(allowed(
            &receipt("future_delivery"),
            &receipt("future_delivery")
        ));
    }

    #[test]
    fn a_backward_receipt_move_is_refused() {
        let backward = [
            (receipt(delivered_as::STEP), None),
            (receipt(delivered_as::STEP), receipt(delivered_as::PENDING)),
            (receipt(delivered_as::TURN), receipt(delivered_as::PENDING)),
            (
                receipt(delivered_as::UNKNOWN),
                receipt(delivered_as::PENDING),
            ),
            (receipt(delivered_as::UNKNOWN), None),
            (receipt(delivered_as::PENDING), None),
        ];
        for (before, after) in &backward {
            assert!(!allowed(before, after), "{before:?} -> {after:?}");
        }
        let error = check_delivered_move(
            receipt(delivered_as::STEP).as_ref(),
            receipt(delivered_as::PENDING).as_ref(),
        )
        .unwrap_err();
        assert!(error.contains("from step to pending"), "{error}");
    }

    #[test]
    fn a_final_receipt_never_changes() {
        let changes = [
            (receipt(delivered_as::UNKNOWN), receipt(delivered_as::STEP)),
            (receipt(delivered_as::STEP), receipt(delivered_as::TURN)),
            (receipt(delivered_as::TURN), receipt(delivered_as::UNKNOWN)),
            (
                Some(Delivered::new(delivered_as::TURN).with_ref("run-1")),
                Some(Delivered::new(delivered_as::TURN).with_ref("run-2")),
            ),
            (
                Some(Delivered::new(delivered_as::STEP).with_ref("row-1")),
                receipt(delivered_as::STEP),
            ),
            (
                receipt(delivered_as::PENDING),
                Some(Delivered::new(delivered_as::PENDING).with_ref("row-1")),
            ),
            (receipt(delivered_as::PENDING), receipt("future_delivery")),
        ];
        for (before, after) in &changes {
            assert!(!allowed(before, after), "{before:?} -> {after:?}");
        }
        let error = check_delivered_move(
            Some(&Delivered::new(delivered_as::TURN).with_ref("run-1")),
            Some(&Delivered::new(delivered_as::TURN).with_ref("run-2")),
        )
        .unwrap_err();
        assert!(
            error.contains("from turn (ref run-1) to turn (ref run-2)"),
            "{error}"
        );
        assert!(error.contains("final"), "{error}");
    }

    #[test]
    fn retries_must_name_the_send_and_agree_once_settled() {
        let send = SendReply::new("active")
            .with_run_id("run-1")
            .with_submission_id("sub-1");
        let finished = SendReply::new("finished")
            .with_run_id("run-1")
            .with_reason("completed");
        assert_eq!(same_answer(&send, &finished, &finished, true), Ok(()));

        let other_submission = finished.clone().with_submission_id("sub-2");
        let error = same_answer(&send, &other_submission, &other_submission, true).unwrap_err();
        assert!(error.contains("submission_id sub-2"), "{error}");

        let other_run = finished.clone().with_run_id("run-2");
        let error = same_answer(&send, &finished, &other_run, true).unwrap_err();
        assert!(error.contains("run_id run-2"), "{error}");

        let error = same_answer(&send, &SendReply::new("finished"), &finished, false).unwrap_err();
        assert!(error.contains("names none"), "{error}");

        // Before the run is seen to end, its state may move between the
        // retries; after, it may not.
        let active = SendReply::new("active").with_run_id("run-1");
        assert_eq!(same_answer(&send, &active, &finished, false), Ok(()));
        let error = same_answer(&send, &active, &finished, true).unwrap_err();
        assert!(error.contains("after its run ended"), "{error}");

        let pending = active
            .clone()
            .with_delivered(Delivered::new(delivered_as::PENDING));
        let unknown = active
            .clone()
            .with_delivered(Delivered::new(delivered_as::UNKNOWN));
        assert_eq!(same_answer(&pending, &unknown, &unknown, true), Ok(()));
        let error = same_answer(&unknown, &pending, &pending, true).unwrap_err();
        assert!(
            error.contains("the first retry's delivered changed from unknown to pending"),
            "{error}"
        );
    }
}
