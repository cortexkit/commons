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
    send::{delivered_as, SendReply, DELIVERY_FIELD},
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
            "send_id_retry_same_answer" => passed(self.send_id_retry().await),
            "send_id_reuse_refused_naming_field" => passed(self.send_id_reuse().await),
            "delivery_change_refused" => passed(self.delivery_change().await),
            "unknown_delivery_refused" => passed(self.unknown_delivery().await),
            "undeclared_delivery_refused" => passed(self.undeclared_delivery().await),
            "guaranteed_steer_never_pending_or_unknown" => {
                self.guaranteed_steer_never_pending_or_unknown().await
            }
            "resend_steer_delivered_stable" => self.resend_steer_delivered_stable().await,
            other => Err(format!("the runner has no case named {other}")),
        }
    }

    fn declares(&self, capability: Capability) -> bool {
        self.declared.contains(&capability)
    }

    // ---- sessions -------------------------------------------------------

    /// A fresh session whose model is `script`, with the owner's route.
    async fn open(&self, label: &str, script: Script) -> Result<Session<S::Route>, String> {
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
    fn params(
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

    /// Send, expect a refusal, and check the head did not move.
    async fn refused_writes_nothing(
        &self,
        what: &str,
        session: &Session<S::Route>,
        prompt: &str,
        send_id: &str,
        delivery: Option<&str>,
    ) -> Result<subc_protocol::ErrorBody, String> {
        let before = head(&session.route).await?;
        let error = expect_error(
            what,
            self.send_raw(session, prompt, send_id, delivery).await?,
        )?;
        let after = head(&session.route).await?;
        if after != before {
            return Err(format!(
                "{what} was refused {} but the session changed: head {before:?} then {after:?}",
                error.code
            ));
        }
        Ok(error)
    }

    async fn send_id_retry(&self) -> Check {
        let (session, send_id, prompt) = self.sent_session("retry").await?;
        let last = self.last_run_id(&session.route).await?;
        let (session_ref, prompt_ref, send_id_ref) = (&session, &prompt, &send_id);
        let retry = |what: &'static str| async move {
            decode::<Value>(
                what,
                expect_ok(
                    what,
                    self.send_raw(session_ref, prompt_ref, send_id_ref, None)
                        .await?,
                )?,
            )
        };
        let first = retry("the first retry").await?;
        let second = retry("the second retry").await?;
        if first != second {
            return Err(format!(
                "two retries of one send answered {first} and {second}"
            ));
        }
        let reply: SendReply = decode("the retry", first)?;
        if reply.run_id.is_some() && reply.run_id != last {
            return Err(format!(
                "the retry names run {:?}; the send started run {last:?}",
                reply.run_id
            ));
        }
        if self.last_run_id(&session.route).await? != last {
            return Err("a retried send started another run".into());
        }
        let (_, all) = read_all(&session.route).await?;
        let (_, prompt) = session.first.clone().expect("recorded");
        let copies = holding(&all, &prompt);
        if copies != 1 {
            return Err(format!(
                "the retried prompt is written in {copies} messages, not one"
            ));
        }
        Ok(())
    }

    async fn send_id_reuse(&self) -> Check {
        let (session, send_id, _) = self.sent_session("reuse").await?;
        let other = self.mint.next("other-prompt");
        let what = "a send_id reused with another prompt";
        let error = self
            .refused_writes_nothing(what, &session, &other, &send_id, None)
            .await?;
        expect_field(what, &error, errors::SEND_ID_REUSE, "prompt")
    }

    async fn delivery_change(&self) -> Check {
        let mode = if self.declares(Capability::Steer) {
            groups::STEER
        } else {
            groups::INTERRUPT
        };
        let (session, send_id, prompt) = self.sent_session("delivery-change").await?;
        let what = format!("a send_id reused with delivery {mode}");
        let error = self
            .refused_writes_nothing(&what, &session, &prompt, &send_id, Some(mode))
            .await?;
        expect_field(&what, &error, errors::SEND_ID_REUSE, DELIVERY_FIELD)
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
        let (session, _, _) = self.sent_session("steer-guaranteed").await?;
        let steer_id = self.mint.next("steer");
        let prompt = self.mint.next("prompt");
        let reply_raw = expect_ok(
            "session.send steer",
            self.send_raw(&session, &prompt, &steer_id, Some(groups::STEER))
                .await?,
        )?;
        let reply: SendReply = decode("session.send steer", reply_raw)?;
        if let Some(delivered) = &reply.delivered {
            if delivered.r#as == delivered_as::PENDING || delivered.r#as == delivered_as::UNKNOWN {
                return Err(format!(
                    "a guaranteed runner answered delivered.as: {}",
                    delivered.r#as
                ));
            }
        }
        let retry_raw = expect_ok(
            "session.send steer retry",
            self.send_raw(&session, &prompt, &steer_id, Some(groups::STEER))
                .await?,
        )?;
        let retry: SendReply = decode("session.send steer retry", retry_raw)?;
        if let Some(delivered) = &retry.delivered {
            if delivered.r#as == delivered_as::PENDING || delivered.r#as == delivered_as::UNKNOWN {
                return Err(format!(
                    "a guaranteed runner answered delivered.as: {} on re-send",
                    delivered.r#as
                ));
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

        let receipt1 = match &retry1.delivered {
            Some(d) if matches!(d.r#as.as_str(), delivered_as::STEP | delivered_as::TURN) => d,
            _ => return Ok(Ending::Passed),
        };
        let receipt2 = retry2
            .delivered
            .as_ref()
            .ok_or_else(|| "the second re-send carries no delivered receipt".to_owned())?;
        if receipt1 != receipt2 {
            return Err(format!(
                "re-send delivered changed from {receipt1:?} to {receipt2:?}"
            ));
        }
        if let Some(first_receipt) = &first_reply.delivered {
            if matches!(
                first_receipt.r#as.as_str(),
                delivered_as::STEP | delivered_as::TURN
            ) && first_receipt != receipt1
            {
                return Err(format!(
                    "re-send delivered {receipt1:?} differs from initial reply {first_receipt:?}"
                ));
            }
        }
        Ok(Ending::Passed)
    }
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
