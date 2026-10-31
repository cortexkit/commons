//! Pending first views require a notice before publication, not a fallback head.

use cortexkit_role_compaction_provider::{
    notice::{headers, AdvanceNotice},
    ops as provider_ops,
    setup::{VIEW_FEATURE_VIEW_PENDING, VIEW_PENDING},
};
use cortexkit_role_llm_runner::{
    errors::provider_codes::COMPACTION_UNAVAILABLE, run::RunState, send::SendReply,
};
use serde_json::Value;

use crate::{
    cases::{Case, Ending, Session},
    compaction::{self, initial, observation, CompactionSetup},
    drive::{head, read_all, run_result, MAX_POLLS},
    subject::{LlmRunnerSubject, Script},
    view_due, ProviderCall, RunnerRoute, SetupViewDueFixture, SetupViewPendingScenario,
};

fn declares_pending(setup: &Value) -> bool {
    setup["view_features"].as_array().is_some_and(|features| {
        features
            .iter()
            .any(|feature| feature.as_str() == Some(VIEW_FEATURE_VIEW_PENDING))
    })
}

fn cap(setup: &Value) -> Result<u64, String> {
    setup["view_due_max_ms"]
        .as_u64()
        .filter(|cap| *cap > 0)
        .ok_or_else(|| "view_pending support requires a positive view_due_max_ms".into())
}

fn setup(calls: &[ProviderCall]) -> Result<&Value, String> {
    calls
        .iter()
        .find(|call| call.method == provider_ops::COMPACTION_SETUP)
        .map(|call| &call.params)
        .ok_or_else(|| "provider observation contains no Setup request".into())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let scenario = match name {
        "view_pending_first_notice_then_view" => SetupViewPendingScenario::PublishAfterNotice,
        "view_pending_hold_expires" => SetupViewPendingScenario::HoldExpires,
        "view_pending_both_members_fail_setup" => SetupViewPendingScenario::BothMembers,
        "view_pending_undeclared_fails_setup" => SetupViewPendingScenario::Undeclared,
        _ => return Err(format!("unknown pending-view case {name}")),
    };
    let mut control = compaction::install(
        case,
        "pending-control",
        CompactionSetup::Ready(initial("pending-control-initial")),
        Script {
            turns: vec![compaction::text("control-answer")],
        },
    )
    .await?;
    let reply = compaction::send(case, &mut control, "pending-control-prompt").await?;
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
    let request = setup(&calls)?;
    if !declares_pending(request) {
        return Ok(Ending::Inapplicable(
            "runner sends no view_pending in Setup view_features".into(),
        ));
    }
    cap(request)?;
    let marker = case.mint.next("pending");
    let mut session = compaction::install(
        case,
        "pending",
        CompactionSetup::Ready(initial(&format!("{marker}-initial"))),
        Script {
            turns: vec![compaction::text("pending-answer")],
        },
    )
    .await?;
    let fixture = match case
        .subject
        .install_setup_view_pending_fixture(
            &session.name,
            initial(&format!("{marker}-initial")),
            vec![format!("{marker}-published-view")],
            scenario,
        )
        .await
    {
        Ok(fixture) => fixture,
        Err(e) => {
            return Ok(Ending::Inapplicable(format!(
                "adapter lacks install_setup_view_pending_fixture: {e}"
            )))
        }
    };
    if !fixture.model_only_decorations.is_empty() {
        return Ok(Ending::Inapplicable(format!(
            "pending-view fixture plan enables model-only decorations: {}",
            fixture.model_only_decorations.join(", ")
        )));
    }
    if fixture.version == 0 || fixture.rendering.is_empty() {
        return Err("adapter returned an invalid or empty pending-view fixture".into());
    }
    let started = tokio::time::Instant::now();
    let reply = compaction::send(case, &mut session, &format!("{marker}-first-prompt")).await?;
    if matches!(
        scenario,
        SetupViewPendingScenario::BothMembers | SetupViewPendingScenario::Undeclared
    ) {
        failed_setup(case, &session, &reply).await?;
    } else {
        notice_before_publication(case, &session, &fixture).await?;
        if scenario == SetupViewPendingScenario::PublishAfterNotice {
            if let Err(e) = case.subject.publish_setup_view_pending(&session.name).await {
                return Ok(Ending::Inapplicable(format!(
                    "adapter lacks publish_setup_view_pending: {e}"
                )));
            }
            view_due::applied(case, &session, &reply, &fixture).await?;
        } else {
            view_due::refused_without_model(case, &session, &reply).await?;
        }
        view_due::require_fetch(case, &session, &fixture, 1).await?;
    }
    let calls = case
        .subject
        .compaction_provider_calls(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let request = setup(&calls)?;
    if declares_pending(request) != (scenario != SetupViewPendingScenario::Undeclared) {
        return Err(
            "fixture runner's actual Setup view_pending declaration does not match the scenario"
                .into(),
        );
    }
    let hold_ms = cap(request)?;
    if scenario == SetupViewPendingScenario::HoldExpires
        && started.elapsed().as_millis() < u128::from(hold_ms)
    {
        return Err("runner refused the pending view before its hold expired".into());
    }
    let fetches = calls
        .iter()
        .filter(|call| call.method == provider_ops::VIEW_GET)
        .count();
    let expected = usize::from(matches!(
        scenario,
        SetupViewPendingScenario::PublishAfterNotice | SetupViewPendingScenario::HoldExpires
    ));
    if fetches != expected {
        return Err(format!("pending first view needs {expected} fetch requests, not polling or a fetch after failed Setup; got {fetches}"));
    }
    Ok(Ending::Passed)
}

async fn notice_before_publication<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    fixture: &SetupViewDueFixture,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for _ in 0..MAX_POLLS {
        let observed = observation(case.subject, &session.name).await?;
        if !observed.model_inputs.is_empty() {
            compaction::release_model(case.subject, &session.name).await?;
            return Err("runner called the model before the pending view was published".into());
        }
        let notices: Vec<_> = observed
            .notices
            .iter()
            .map(|headers| {
                AdvanceNotice::from_headers(headers)
                    .map_err(|e| format!("malformed first notice: {e:?}"))
            })
            .collect::<Result<_, _>>()?;
        let (_, rows) = read_all(&session.route).await?;
        if let Some(first_write) = rows.iter().find(|row| row.ordinal > fixture.covers_through) {
            if observed.notices.iter().zip(&notices).any(|(raw, notice)| {
                raw.get(headers::SESSION_REF) == Some(&fixture.session_ref)
                    && notice.through_ordinal == first_write.ordinal
                    && notice.through_digest
                        == cortexkit_role_llm_runner::content_digest::content_digest(
                            &first_write.message,
                        )
            }) {
                let calls = case
                    .subject
                    .compaction_provider_calls(&session.name)
                    .await
                    .map_err(|e| e.to_string())?;
                if calls
                    .iter()
                    .any(|call| call.method == provider_ops::VIEW_GET)
                {
                    return view_due::require_fetch(case, session, fixture, 1).await;
                }
            }
        }
        case.subject.pause().await;
    }
    Err("runner did not expose its first readable write's matching notice and fetch while the view was unpublished".into())
}

async fn failed_setup<S>(
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
    for _ in 0..MAX_POLLS {
        if !observation(case.subject, &session.name)
            .await?
            .model_inputs
            .is_empty()
        {
            compaction::release_model(case.subject, &session.name).await?;
            return Err(
                "runner called the model instead of failing Setup naming view_pending".into(),
            );
        }
        let result = run_result(&session.route, &id).await?;
        if result.run_state().is_terminal() {
            if result.run_state() != RunState::Error
                || result
                    .error
                    .as_ref()
                    .map(|error| error.provider_code.as_str())
                    != Some(COMPACTION_UNAVAILABLE)
                || result
                    .error
                    .as_ref()
                    .and_then(|error| error.reason.as_deref())
                    != Some(VIEW_PENDING)
            {
                return Err(format!(
                    "failed Setup must name view_pending with compaction_unavailable, got {} {:?}",
                    result.state, result.error
                ));
            }
            if !observation(case.subject, &session.name)
                .await?
                .model_inputs
                .is_empty()
            {
                return Err("runner called the model after failed Setup".into());
            }
            return Ok(());
        }
        case.subject.pause().await;
    }
    Err("runner did not fail Setup naming view_pending".into())
}
