//! Opt-in moved-session and native-marker checks over the runner's route.

use cortexkit_role_llm_runner::{
    capabilities, ops,
    read::{HeadCut, ReadRequest},
    send::SendReply,
    tags::SetupCheckpoint,
};
use serde_json::{Map, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, read_all},
    lineage,
    subject::{Capability, LlmRunnerSubject, Script},
    RunnerRoute,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    Adoption,
    CutRemoved,
    MissingMessageCounter,
    MissingTagCounter,
}

/// Offline fixture facts, not values reconstructed from runner replies.
#[derive(Clone, Debug)]
pub struct Fixture {
    pub checkpoint: SetupCheckpoint,
    pub plan_fields: Map<String, Value>,
}

#[derive(Clone, Copy)]
enum SetupQuery {
    Head,
    AdoptionRead,
}

pub(crate) async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if !case
        .checked_describe()
        .await?
        .declares(capabilities::MESSAGE_CHECKPOINT)
    {
        return Ok(Ending::Inapplicable(
            "runner does not declare message_checkpoint".into(),
        ));
    }
    let missing: Vec<_> = [
        Capability::Queue,
        Capability::TranscriptReads,
        Capability::Compaction,
        Capability::RunOps,
    ]
    .into_iter()
    .filter(|c| !case.declared.contains(c))
    .collect();
    if !missing.is_empty() {
        return Ok(Ending::Skipped {
            missing,
            reason: "message checkpoint cases send and read a compaction session".into(),
        });
    }
    if name == "message_checkpoint_native_marker_carries_both_counters" {
        return native_marker(case).await;
    }
    if name == "message_checkpoint_uncounted_rows_never_served" {
        return uncounted_rows(case).await;
    }
    if name == "message_checkpoint_head_during_setup_uses_marker" {
        return checkpoint_during_setup(case, SetupQuery::Head).await;
    }
    if name == "message_checkpoint_read_during_adoption_setup" {
        return checkpoint_during_setup(case, SetupQuery::AdoptionRead).await;
    }
    if name == "message_checkpoint_missing_counter_is_not_zero" {
        for scenario in [Scenario::MissingMessageCounter, Scenario::MissingTagCounter] {
            let ending = adoption(case, name, scenario).await?;
            if !matches!(ending, Ending::Passed) {
                return Ok(ending);
            }
        }
        return Ok(Ending::Passed);
    }
    let scenario = if name == "message_checkpoint_anchors_after_folded_message" {
        Scenario::CutRemoved
    } else {
        Scenario::Adoption
    };
    adoption(case, name, scenario).await
}

async fn adoption<S>(case: &Case<'_, S>, name: &str, scenario: Scenario) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "message-checkpoint",
            Script {
                turns: vec![crate::compaction::text("retained answer")],
            },
        )
        .await?;
    let Some(fixture) = case
        .subject
        .message_checkpoint_fixture(case.handle.inner(), &session.name, scenario)
        .await
        .map_err(|e| format!("message_checkpoint_fixture: {e}"))?
    else {
        return Ok(Ending::Inapplicable(
            "adapter lacks message_checkpoint_fixture".into(),
        ));
    };
    if fixture.checkpoint.message_count != Some(42)
        || fixture.checkpoint.tag_checkpoint != 500
        || fixture.checkpoint.head_cut.is_none()
    {
        return Err("message_checkpoint_fixture must certify message_count 42, tag_checkpoint 500 and head_cut".into());
    }
    let id = case.mint.next("send");
    let mut params = case.params(&session, "retained prompt", &id, None);
    params
        .as_object_mut()
        .ok_or("send params are not an object")?
        .extend(fixture.plan_fields);
    let reply = call(&session.route, ops::SESSION_SEND, params).await?;
    let missing_counter = matches!(
        scenario,
        Scenario::MissingMessageCounter | Scenario::MissingTagCounter
    );
    if missing_counter && matches!(reply, crate::route::Reply::Error(_)) {
        return no_model(case, &session.name).await;
    }
    let sent: SendReply = decode(
        "message checkpoint send",
        expect_ok("message checkpoint send", reply)?,
    )?;
    session.first = Some((id, "retained prompt".into()));
    crate::compaction::release_model(case.subject, &session.name).await?;
    if missing_counter {
        let result = crate::compaction::result(case, &session, &sent).await?;
        if result.state != "error" {
            return Err(format!(
                "missing counter reload ended {}, expected refusal, never ordinal 0",
                result.state
            ));
        }
        return no_model(case, &session.name).await;
    }
    crate::compaction::result(case, &session, &sent).await?;
    let meta = head(&session.route).await?;
    let (_, messages) = read_all(&session.route).await?;
    match name {
        "message_checkpoint_numbering_starts_at_message_count" => {
            if meta.first_ordinal != Some(42)
                || messages.first().map(|m| m.ordinal) != Some(42)
                || messages
                    .iter()
                    .enumerate()
                    .any(|(i, m)| m.ordinal != 42 + i as u64)
            {
                return Err(format!(
                    "numbering did not start at message_count 42: first {:?}, ordinals {:?}",
                    meta.first_ordinal,
                    messages.iter().map(|m| m.ordinal).collect::<Vec<_>>()
                ));
            }
        }
        "message_checkpoint_head_matches_certified_cut" => {
            if meta.cut != fixture.checkpoint.head_cut {
                return Err(format!(
                    "session.head.cut {:?} differs from certified head_cut {:?}",
                    meta.cut, fixture.checkpoint.head_cut
                ));
            }
        }
        "message_checkpoint_anchors_after_folded_message" => {
            let cut = fixture.checkpoint.head_cut.as_ref().unwrap();
            if meta.first_ordinal != Some(42)
                || meta.cut.as_ref() != Some(cut)
                || messages.first().map(|m| m.ordinal) != Some(42)
                || messages.iter().any(|m| m.mid == cut.mid || m.ordinal < 42)
            {
                return Err(
                    "removed cut did not anchor immediately after head_cut.mid at ordinal 42"
                        .into(),
                );
            }
        }
        other => return Err(format!("unknown message checkpoint case {other}")),
    }
    Ok(Ending::Passed)
}

async fn no_model<S>(case: &Case<'_, S>, session: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
{
    let observation = crate::compaction::observation(case.subject, session).await?;
    if !observation.model_inputs.is_empty() {
        return Err("missing counter was admitted to a model request instead of refusing".into());
    }
    Ok(Ending::Passed)
}

async fn native_marker<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let written = lineage::written_with(case, "message-marker", 3, true).await?;
    let cut = match lineage::apply_cut(case, written, "message-marker").await? {
        lineage::Fixture::Ready(cut) => cut,
        lineage::Fixture::Inapplicable(reason) => return Ok(Ending::Inapplicable(reason)),
    };
    let Some(key) = case
        .subject
        .native_message_checkpoint(&cut.session.name)
        .await
        .map_err(|e| format!("native_message_checkpoint: {e}"))?
    else {
        return Ok(Ending::Inapplicable(
            "adapter lacks native_message_checkpoint".into(),
        ));
    };
    let folded = &cut.before[1];
    let expected = HeadCut::new(
        &folded.mid,
        cortexkit_role_llm_runner::content_digest::content_digest(&folded.message),
        folded
            .written_at
            .ok_or("folded message has no creation time")?,
    );
    let tag_count = cut.before[..2]
        .iter()
        .flat_map(|m| &m.tags)
        .filter_map(|tag| {
            tag.tag
                .as_deref()
                .and_then(cortexkit_role_llm_runner::tags::parse_tag)
        })
        .max()
        .ok_or("native marker fixture has no counted tags")?;
    if key["message_count"] != 2
        || key["head_cut"] != serde_json::to_value(expected).unwrap()
        || key["tag_checkpoint"] != tag_count
        || key["session"] != cut.session.name
    {
        return Err(format!(
            "native marker key lacks the certified counters and cut: {key}"
        ));
    }
    Ok(Ending::Passed)
}

async fn uncounted_rows<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut adoption = case
        .open(
            "uncounted-adoption",
            Script {
                turns: vec![
                    crate::compaction::text("first answer"),
                    crate::compaction::text("counted answer"),
                ],
            },
        )
        .await?;
    let Some(fixture) = case
        .subject
        .message_checkpoint_fixture(case.handle.inner(), &adoption.name, Scenario::Adoption)
        .await
        .map_err(|e| format!("message_checkpoint_fixture: {e}"))?
    else {
        return Ok(Ending::Inapplicable(
            "adapter lacks message_checkpoint_fixture".into(),
        ));
    };
    let id = case.mint.next("send");
    let mut params = case.params(&adoption, "first counted prompt", &id, None);
    params
        .as_object_mut()
        .ok_or("send params are not an object")?
        .extend(fixture.plan_fields);
    let sent: SendReply = decode(
        "uncounted-row fixture send",
        expect_ok(
            "uncounted-row fixture send",
            call(&adoption.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    adoption.first = Some((id, "first counted prompt".into()));
    crate::compaction::release_model(case.subject, &adoption.name).await?;
    crate::compaction::result(case, &adoption, &sent).await?;
    let ending = exclude_summary(case, &mut adoption).await?;
    if !matches!(ending, Ending::Passed) {
        return Ok(ending);
    }

    let written = lineage::written_with(case, "uncounted-native", 4, true).await?;
    let mut native = match lineage::apply_cut(case, written, "uncounted-native").await? {
        lineage::Fixture::Ready(cut) => cut.session,
        lineage::Fixture::Inapplicable(reason) => return Ok(Ending::Inapplicable(reason)),
    };
    exclude_summary(case, &mut native).await
}

async fn exclude_summary<S>(
    case: &Case<'_, S>,
    session: &mut crate::cases::Session<S::Route>,
) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (_, before) = read_all(&session.route).await?;
    let first = head(&session.route)
        .await?
        .first_ordinal
        .ok_or("summary fixture has no lineage")?;
    let Some(summary_mid) = case
        .subject
        .insert_uncounted_host_summary(case.handle.inner(), &session.name)
        .await
        .map_err(|e| format!("insert_uncounted_host_summary: {e}"))?
    else {
        return Ok(Ending::Inapplicable(
            "adapter lacks insert_uncounted_host_summary".into(),
        ));
    };
    lineage::exchange(case, session, "counted prompt after host summary").await?;
    let (_, messages) = read_all(&session.route).await?;
    if messages.iter().any(|m| {
        m.mid == summary_mid
            || ((m.message["summary"] == true || m.message["summary"] == 1)
                && m.message["finish"] == "stop")
    }) {
        return Err("uncounted summary row was served on session.read".into());
    }
    if messages.len() != before.len() + 2
        || messages
            .iter()
            .enumerate()
            .any(|(i, m)| m.ordinal != first + i as u64)
    {
        return Err(format!(
            "served ordinals are not contiguous over counted rows: {:?}",
            messages.iter().map(|m| m.ordinal).collect::<Vec<_>>()
        ));
    }
    if case.declared.contains(&Capability::HostHistory) {
        let history = crate::host_history::walk_older(&session.route).await?;
        if history.iter().any(|m| m.mid == summary_mid) {
            return Err("uncounted summary row was served on session.history".into());
        }
        let ordinals: Vec<_> = history.iter().filter_map(|m| m.ordinal).collect();
        if ordinals.windows(2).any(|pair| pair[1] != pair[0] + 1) {
            return Err(format!(
                "history ordinals are not contiguous over counted rows: {ordinals:?}"
            ));
        }
    }
    Ok(Ending::Passed)
}

/// Keep the reverse reads and cleanup bounded: a runner that waits for
/// Setup before answering the provider otherwise deadlocks the suite.
async fn checkpoint_during_setup<S>(case: &Case<'_, S>, query: SetupQuery) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case
        .open(
            "marker-before-setup",
            Script {
                turns: vec![crate::compaction::text("marker restored")],
            },
        )
        .await?;
    let Some(fixture) = case
        .subject
        .plant_message_checkpoint_marker(case.handle.inner(), &session.name)
        .await
        .map_err(|e| format!("plant_message_checkpoint_marker: {e}"))?
    else {
        return Ok(Ending::Inapplicable(
            "adapter lacks plant_message_checkpoint_marker".into(),
        ));
    };
    if fixture.checkpoint.message_count != Some(42)
        || fixture.checkpoint.tag_checkpoint != 500
        || fixture.checkpoint.head_cut.is_none()
    {
        return Err(
            "planted marker must certify message_count 42, tag_checkpoint 500 and head_cut".into(),
        );
    }
    if let Err(reason) = case.subject.hold_next_compaction_setup(&session.name).await {
        return Ok(Ending::Inapplicable(format!(
            "adapter lacks hold_next_compaction_setup: {reason}"
        )));
    }
    let id = case.mint.next("send");
    let mut params = case.params(&session, "prompt after adoption marker", &id, None);
    params
        .as_object_mut()
        .ok_or("send params are not an object")?
        .extend(fixture.plan_fields);
    let wait = std::time::Duration::from_secs(5);
    let inspect_checkpoint = async {
        let checked = async {
            tokio::time::timeout(wait, case.subject.await_compaction_setup_held(&session.name))
                .await.map_err(|_| "first Setup did not reach its hold within 5s".to_owned())?
                .map_err(|e| format!("await_compaction_setup_held: {e}"))?;
            match query {
                SetupQuery::Head => {
                    let meta = tokio::time::timeout(wait, head(&session.route)).await
                        .map_err(|_| "session.head during Setup did not answer within 5s; it must not wait on Setup or its gates".to_owned())??;
                    if meta.first_ordinal != fixture.checkpoint.message_count || meta.cut != fixture.checkpoint.head_cut {
                        return Err(format!("session.head during Setup did not use the pre-existing marker: first_ordinal {:?}, cut {:?}; expected {:?}, {:?}",
                            meta.first_ordinal, meta.cut, fixture.checkpoint.message_count, fixture.checkpoint.head_cut));
                    }
                }
                SetupQuery::AdoptionRead => {
                    let first = fixture.checkpoint.message_count.unwrap();
                    let page = tokio::time::timeout(wait,
                        crate::drive::read(&session.route, &ReadRequest::range(first).with_limit(1))).await
                        .map_err(|_| "session.read during Setup did not answer within 5s; it must not wait on Setup or its gates".to_owned())??;
                    if page.messages.iter().any(|message| message.ordinal < first) {
                        return Err(format!("session.read during Setup served rows below first_ordinal {first}"));
                    }
                    if page.lineage_id.is_none() || page.messages.first().map(|message| message.ordinal) != Some(first) {
                        return Err(format!("session.read during Setup did not start at first_ordinal {first}"));
                    }
                    let below = serde_json::to_value(ReadRequest::range(first - 1).with_limit(1))
                        .map_err(|e| format!("encoding a read below first_ordinal: {e}"))?;
                    let reply = tokio::time::timeout(wait, call(&session.route, ops::SESSION_READ, below)).await
                        .map_err(|_| "session.read during Setup did not answer a below-first_ordinal request within 5s".to_owned())??;
                    if !matches!(reply, crate::route::Reply::Error(ref error)
                        if error.code == cortexkit_role_llm_runner::errors::BEFORE_LINEAGE_START) {
                        return Err(format!("session.read during Setup did not refuse rows below first_ordinal {first}: {reply:?}"));
                    }
                }
            }
            Ok(())
        }.await;
        let released =
            tokio::time::timeout(wait, case.subject.release_compaction_setup(&session.name))
                .await
                .map_err(|_| "releasing the held Setup did not finish within 5s".to_owned())?
                .map_err(|e| format!("release_compaction_setup: {e}"));
        checked.and(released)
    };
    let (sent, checked) = tokio::time::timeout(
        std::time::Duration::from_secs(16),
        futures_util::future::join(
            call(&session.route, ops::SESSION_SEND, params),
            inspect_checkpoint,
        ),
    )
    .await
    .map_err(|_| {
        "the first send did not settle after checking the checkpoint during Setup".to_owned()
    })?;
    if let Err(reason) = checked {
        let _ = tokio::time::timeout(
            wait,
            crate::compaction::release_model(case.subject, &session.name),
        )
        .await;
        return Err(reason);
    }
    let sent: SendReply = decode(
        "marker-before-Setup send",
        expect_ok("marker-before-Setup send", sent?)?,
    )?;
    crate::compaction::release_model(case.subject, &session.name).await?;
    crate::compaction::result(case, &session, &sent).await?;
    Ok(Ending::Passed)
}
