//! Setup's opt-in due view must apply before the model, never fall back to
//! `initial`, and survive a failed fetch so the next step retries it.

#[cfg(test)]
mod tests;

use cortexkit_role_compaction_provider::{ops as provider_ops, setup::VIEW_DUE_UNAPPLIED};
use cortexkit_role_llm_runner::{
    errors::provider_codes::COMPACTION_UNAVAILABLE,
    read::{EntrySource, ModelEntry},
    run::RunState,
    send::SendReply,
};

use crate::{
    cases::{Case, Ending, Session},
    compaction::{self, initial, observation, CompactionSetup},
    drive::{head, read_all, run_result},
    subject::{LlmRunnerSubject, Script},
    RunnerRoute, SetupViewDueFixture,
};

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut control = compaction::install(
        case,
        "due-control",
        CompactionSetup::Ready(initial("due-control-initial")),
        Script {
            turns: vec![compaction::text("control-answer")],
        },
    )
    .await?;
    let reply = compaction::send(case, &mut control, "due-control-prompt").await?;
    compaction::release_model(case.subject, &control.name).await?;
    compaction::result(case, &control, &reply).await?;
    let calls = match case.subject.compaction_provider_calls(&control.name).await {
        Ok(calls) => calls,
        Err(e) => {
            return Ok(Ending::Inapplicable(format!(
                "adapter lacks Setup request observation: {e}"
            )))
        }
    };
    let setup = calls
        .iter()
        .find(|call| call.method == provider_ops::COMPACTION_SETUP)
        .ok_or("provider observation contains no Setup request")?;
    match setup.params.get("view_due_max_ms") {
        None => {
            return Ok(Ending::Inapplicable(
                "runner sends no view_due_max_ms in Setup".into(),
            ))
        }
        Some(cap) if cap.as_u64().is_some_and(|cap| cap >= 1) => {}
        Some(cap) => {
            return Err(format!(
                "view_due_max_ms must be a u64 at least 1, got {cap}"
            ))
        }
    }
    let marker = case.mint.next("due");
    let mut session = compaction::install(
        case,
        "due",
        CompactionSetup::Ready(initial(&format!("{marker}-initial"))),
        Script {
            turns: vec![compaction::text("due-answer")],
        },
    )
    .await?;
    let refused = name != "view_due_first_model_uses_view";
    let fixture = match case
        .subject
        .install_setup_view_due_fixture(
            &session.name,
            initial(&format!("{marker}-initial")),
            vec![format!("{marker}-published-view")],
            refused,
        )
        .await
    {
        Ok(fixture) => fixture,
        Err(e) => {
            return Ok(Ending::Inapplicable(format!(
                "adapter lacks install_setup_view_due_fixture: {e}"
            )))
        }
    };
    if !fixture.model_only_decorations.is_empty() {
        return Ok(Ending::Inapplicable(format!(
            "due-view fixture plan enables model-only decorations: {}",
            fixture.model_only_decorations.join(", ")
        )));
    }
    if fixture.version == 0
        || !cortexkit_role_compaction_provider::digest::is_sha256_hex(&fixture.digest)
        || fixture.rendering.is_empty()
    {
        return Err("adapter returned an invalid or empty due-view fixture".into());
    }
    let reply = compaction::send(case, &mut session, &format!("{marker}-first-prompt")).await?;
    if !refused {
        return applied(case, &session, &reply, &fixture)
            .await
            .map(|()| Ending::Passed);
    }
    refused_without_model(case, &session, &reply).await?;
    require_fetch(case, &session, &fixture, 1).await?;
    if name == "view_due_refused_fetch_blocks_model" {
        return Ok(Ending::Passed);
    }
    if name != "view_due_later_step_retries" {
        return Err(format!("unknown due-view case {name}"));
    }
    if let Err(e) = case
        .subject
        .set_setup_view_due_fetch_refused(&session.name, false)
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "adapter lacks set_setup_view_due_fetch_refused: {e}"
        )));
    }
    let before = case
        .subject
        .compaction_provider_calls(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let reply = compaction::send(case, &mut session, &format!("{marker}-retry-prompt")).await?;
    applied(case, &session, &reply, &fixture).await?;
    let after = case
        .subject
        .compaction_provider_calls(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    if after
        .iter()
        .filter(|call| call.method == provider_ops::COMPACTION_SETUP)
        .count()
        != before
            .iter()
            .filter(|call| call.method == provider_ops::COMPACTION_SETUP)
            .count()
    {
        return Err("later step repeated Setup instead of retaining the due view".into());
    }
    if !after
        .get(before.len()..)
        .unwrap_or_default()
        .iter()
        .any(|call| {
            call.method == provider_ops::VIEW_GET
                && call.params["view_version"].as_u64() == Some(fixture.version)
                && call.params["session_ref"].as_str() == Some(fixture.session_ref.as_str())
        })
    {
        return Err("later step did not retry view.get for the due view".into());
    }
    Ok(Ending::Passed)
}

async fn require_fetch<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    fixture: &SetupViewDueFixture,
    minimum: usize,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let calls = case
        .subject
        .compaction_provider_calls(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let count = calls
        .iter()
        .filter(|call| {
            call.method == provider_ops::VIEW_GET
                && call.params["view_version"].as_u64() == Some(fixture.version)
                && call.params["session_ref"].as_str() == Some(fixture.session_ref.as_str())
        })
        .count();
    if count < minimum {
        return Err(format!(
            "expected at least {minimum} view.get calls for due version {}, got {count}",
            fixture.version
        ));
    }
    Ok(())
}

async fn refused_without_model<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    reply: &SendReply,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let id = match &reply.run_id {
        Some(id) => id.clone(),
        None => {
            head(&session.route)
                .await?
                .last_run_state
                .ok_or("send has no run")?
                .run_id
        }
    };
    for _ in 0..crate::drive::MAX_POLLS {
        if !observation(case.subject, &session.name)
            .await?
            .model_inputs
            .is_empty()
        {
            compaction::release_model(case.subject, &session.name).await?;
            return Err("runner called the model after a refused due-view fetch".into());
        }
        let result = run_result(&session.route, &id).await?;
        if result.run_state().is_terminal() {
            let error = &result.error;
            if result.run_state() != RunState::Error
                || error.as_ref().map(|error| error.provider_code.as_str())
                    != Some(COMPACTION_UNAVAILABLE)
                || error.as_ref().and_then(|error| error.reason.as_deref())
                    != Some(VIEW_DUE_UNAPPLIED)
            {
                return Err(format!("refused due-view fetch must end compaction_unavailable with reason view_due_unapplied, got {} {error:?}", result.state));
            }
            if !observation(case.subject, &session.name)
                .await?
                .model_inputs
                .is_empty()
            {
                compaction::release_model(case.subject, &session.name).await?;
                return Err("runner called the model after a refused due-view fetch".into());
            }
            return Ok(());
        }
        case.subject.pause().await;
    }
    Err("run never ended after refused due-view fetch".into())
}

async fn applied<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    reply: &SendReply,
    fixture: &SetupViewDueFixture,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut inputs = Vec::new();
    for _ in 0..crate::drive::MAX_POLLS {
        inputs = observation(case.subject, &session.name).await?.model_inputs;
        if !inputs.is_empty() {
            break;
        }
        case.subject.pause().await;
    }
    if inputs.len() != 1 {
        return Err(format!(
            "due view must reach exactly one first model request, got {}",
            inputs.len()
        ));
    }
    let (_, transcript) = read_all(&session.route).await?;
    let mut expected = fixture.rendering.clone();
    expected.extend(
        transcript
            .into_iter()
            .filter(|message| message.ordinal > fixture.covers_through)
            .map(|message| {
                ModelEntry::new(
                    EntrySource::Message {
                        ordinal: message.ordinal,
                        mid: message.mid,
                    },
                    message.message,
                )
            }),
    );
    if inputs[0] != expected {
        compaction::release_model(case.subject, &session.name).await?;
        return Err(format!("first model request must equal the due view rendering, never initial: got {:?}, expected {expected:?}", inputs[0]));
    }
    require_fetch(case, session, fixture, 1).await?;
    compaction::release_model(case.subject, &session.name).await?;
    let result = compaction::result(case, session, reply).await?;
    if result.run_state() != RunState::Completed {
        return Err(format!("applied due view run ended {}", result.state));
    }
    Ok(())
}
