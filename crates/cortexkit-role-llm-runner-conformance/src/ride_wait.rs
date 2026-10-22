//! The provider-set rewrite-ride wait (compaction-provider CONTRACT §4 and
//! §8, `cortexkit-role-llm-runner` CONTRACT §6): a runner that sends
//! `ride_wait_max_ms` in its Setup request holds a step for a rewrite ride
//! as long as Setup's `ride_wait_ms` says, never accepts a `ride_wait_ms`
//! above its own cap, and reports a ride whose answer has been pending for
//! 1 s as `compaction_wait`, never one answered sooner.
//!
//! The three answer-timing cases drive a compaction session whose Setup
//! answer carries a short `cache_ttl` entry for the session's model, so
//! moving the prompt-cache clock past it makes the next step's ride a
//! `ttl_expired` one, and script how the provider answers that ride. The
//! cap case instead answers Setup with a `ride_wait_ms` above the runner's
//! cap and checks that Setup fails. A runner that sends
//! no `ride_wait_max_ms` does not declare provider-set ride waits, and the
//! cases are inapplicable to it.

use std::time::{Duration, Instant};

use cortexkit_role_compaction_provider::{
    cache_ttl::CacheTtl,
    // The Setup member a runner names when it fails Setup over it.
    setup::RIDE_WAIT_MS as RIDE_WAIT_MS_FIELD,
};
use cortexkit_role_llm_runner::{
    errors::provider_codes::COMPACTION_UNAVAILABLE,
    read::HeadMeta,
    run::{RunReason, RunState},
    send::SendReply,
    RUN_REASON_COMPACTION_WAIT,
};

use crate::{
    cases::{Case, Ending, Session},
    compaction::{
        self, initial, install, observation, release_model, CompactionSetup, ScriptedRange,
        ScriptedRide, ScriptedView,
    },
    drive::{head, read_all, run_result},
    subject::{LlmRunnerSubject, Script},
    RunnerRoute,
};

/// The ride wait the cases answer Setup with.
const RIDE_WAIT_MS: u64 = 4_000;

/// When the provider answers the slow rides: the view within the wait (and
/// past a 2 s deadline a runner might fix on its own), and `nothing` past
/// the 1 s report mark.
const SLOW_VIEW_MS: u64 = 3_000;
const SLOW_NOTHING_MS: u64 = 2_500;

/// When the provider answers the fast ride: well under the 1 s report mark.
const FAST_MS: u64 = 200;

/// The prompt-cache lifetime of the Setup entry, and how far the clock moves
/// past it so the next step's ride is a `ttl_expired` one.
const SHORT_MS: u64 = 1_000;
const PAST_SHORT_MS: u64 = 5_000;

/// How long, and how many polls, a case waits for a ride's step to reach
/// the model: well past any wait it scripts.
const RIDE_BUDGET: Duration = Duration::from_secs(30);
const RIDE_POLLS: usize = 10_000;

type Outcome = Result<Result<(), String>, String>;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if let Err(reason) = crate::cache_ttl::control(case).await? {
        return Ok(Ending::Inapplicable(reason));
    }
    let cap = match cap(case).await? {
        Ok(cap) => cap,
        Err(reason) => return Ok(Ending::Inapplicable(reason)),
    };
    let outcome = match name {
        "ride_wait_above_cap_fails_setup" => above_cap(case, cap).await?,
        _ if cap < RIDE_WAIT_MS => Err(format!(
            "the runner's ride_wait_max_ms {cap} is below the {RIDE_WAIT_MS} ms this case answers \
             Setup with"
        )),
        "ride_wait_provider_wait_honoured" => honoured(case).await?,
        "ride_wait_slow_answer_reported" => slow_reported(case).await?,
        "ride_wait_fast_answer_not_reported" => fast_unreported(case).await?,
        other => return Err(format!("unknown ride_wait case {other}")),
    };
    Ok(match outcome {
        Ok(()) => Ending::Passed,
        Err(reason) => Ending::Inapplicable(reason),
    })
}

/// The `ride_wait_max_ms` the runner sends in its Setup request, read from a
/// session of one exchange. `Ok(Err(reason))` when it sends none, or the
/// adapter cannot say.
async fn cap<S>(case: &Case<'_, S>) -> Result<Result<u64, String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let marker = case.mint.next("ride-cap");
    let mut session = install(
        case,
        "ride-cap",
        CompactionSetup::Ready(initial(&marker)),
        Script {
            turns: vec![compaction::text(&format!("{marker}-answer"))],
        },
    )
    .await?;
    exchange(case, &mut session).await?;
    Ok(
        match case.subject.observed_ride_wait_max_ms(&session.name).await {
            Err(e) => Err(format!(
                "the adapter does not report the ride_wait_max_ms of the runner's Setup request: \
                 {e}"
            )),
            Ok(None) => Err(
                "the runner sends no ride_wait_max_ms in its Setup request, so it \
                 does not declare provider-set ride waits"
                    .into(),
            ),
            Ok(Some(cap)) => Ok(cap),
        },
    )
}

/// Send one prompt, release its held model answer and wait for the run.
async fn exchange<S>(case: &Case<'_, S>, session: &mut Session<S::Route>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let reply: SendReply = compaction::send(case, session, &case.mint.next("ride-prompt")).await?;
    release_model(case.subject, &session.name).await?;
    compaction::result(case, session, &reply).await?;
    Ok(())
}

/// What the case saw while a ride's step waited for its answer.
struct RideStep {
    /// Whether `session.head` named `compaction_wait` for the run.
    reported: bool,
    /// The canonical model input of the ride's step.
    input: String,
}

/// A compaction session whose Setup answers with `ride_wait_ms` and a short
/// `cache_ttl` entry, after one exchange; then the provider answers the
/// next ride as `ride` says (built from the session's transcript), the
/// prompt-cache clock passes the entry, and one more prompt goes out. Reads
/// the run's state while the ride's step waits. `Ok(Err(reason))` when the
/// adapter lacks a hook.
async fn ride_step<S>(
    case: &Case<'_, S>,
    ride: impl FnOnce(&HeadMeta, &str, u64) -> ScriptedRide,
) -> Result<Result<RideStep, String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let marker = case.mint.next("ride");
    let model = case.mint.next("model");
    let mut session = install(
        case,
        "ride",
        CompactionSetup::ReadyWithRideWait {
            initial: initial(&marker),
            cache_ttl: Some(CacheTtl::new(&model, Some(SHORT_MS), true)),
            ride_wait_ms: RIDE_WAIT_MS,
        },
        Script {
            turns: (0..2)
                .map(|n| compaction::text(&format!("{marker}-answer-{n}")))
                .collect(),
        },
    )
    .await?;
    if let Err(e) = case.subject.switch_model(&session.name, &model).await {
        return Ok(Err(format!(
            "the adapter cannot choose the session's model: {e}"
        )));
    }
    if let Err(e) = case
        .subject
        .set_observed_cache_lifetime(&session.name, None)
        .await
    {
        return Ok(Err(format!(
            "the adapter cannot set the lifetime the runner observes in its request: {e}"
        )));
    }
    exchange(case, &mut session).await?;
    let now = head(&session.route).await?;
    let (_, messages) = read_all(&session.route).await?;
    let last = messages
        .last()
        .ok_or("the session reads empty after an exchange")?;
    let scripted = ride(&now, &last.mid, last.ordinal);
    if let Err(e) = case.subject.script_next_ride(&session.name, scripted).await {
        return Ok(Err(format!(
            "the adapter cannot script a ride's answer: {e}"
        )));
    }
    if let Err(e) = case
        .subject
        .advance_prompt_cache_clock(&session.name, PAST_SHORT_MS)
        .await
    {
        return Ok(Err(format!("the adapter has no prompt-cache clock: {e}")));
    }
    let before = observation(case.subject, &session.name)
        .await?
        .model_inputs
        .len();
    let reply: SendReply =
        compaction::send(case, &mut session, &case.mint.next("ride-prompt")).await?;
    let asks_status = crate::ceiling::serves_run_status(&session.route).await?;
    let started = Instant::now();
    let mut reported = false;
    let mut input = None;
    for _ in 0..RIDE_POLLS {
        let observed = observation(case.subject, &session.name).await?;
        if let Some(step) = observed.model_inputs.get(before) {
            input = Some(serde_json::to_string(step).expect("entries serialize"));
            break;
        }
        if started.elapsed() > RIDE_BUDGET {
            break;
        }
        let last = head(&session.route).await?.last_run_state;
        if let Some(last) = last.filter(|last| !RunState::parse(&last.state).is_terminal()) {
            if last.reason.as_deref() == Some(RUN_REASON_COMPACTION_WAIT) {
                reported = true;
                if asks_status {
                    let status = crate::ceiling::status(&session.route, &last.run_id).await?;
                    // The ride may resolve between the two reads; a status
                    // that still names the wait carries its deadline.
                    if status.reason == Some(RunReason::CompactionWait)
                        && status.deadline_ms.is_none_or(|deadline| deadline == 0)
                    {
                        return Err(format!(
                            "while a step waited on its ride, run.status answered {status:?}: \
                             compaction_wait carries deadline_ms, the absolute time the ride's \
                             deadline ends"
                        ));
                    }
                }
            }
        }
        case.subject.pause().await;
    }
    let input = input.ok_or(
        "the step after the prompt cache expired never reached the model, though its ride was \
         answered within the Setup's ride_wait_ms",
    )?;
    release_model(case.subject, &session.name).await?;
    let ended = compaction::result(case, &session, &reply).await?;
    if ended.run_state() != RunState::Completed {
        return Err(format!(
            "the run whose step waited on a rewrite ride ended {}, not completed: a rewrite ride \
             never ends a run",
            ended.state
        ));
    }
    Ok(Ok(RideStep { reported, input }))
}

/// A view of the session as it stands: a head of `text` before every
/// message, and every message kept.
fn ride_view(head: &HeadMeta, mid: &str, ordinal: u64, text: &str) -> ScriptedView {
    ScriptedView {
        lineage_id: head.lineage_id.clone().unwrap_or_default(),
        covers_through: ordinal,
        covers_through_mid: mid.to_owned(),
        ranges: vec![
            ScriptedRange::Replace {
                from: head.first_ordinal.unwrap_or_default(),
                to: head.first_ordinal.unwrap_or_default(),
                replacement: vec![text.to_owned()],
            },
            ScriptedRange::Keep {
                from: head.first_ordinal.unwrap_or_default(),
                to: ordinal + 1,
            },
        ],
        cut: None,
    }
}

/// Setup answers `ride_wait_ms: 4000`; the provider answers the
/// `ttl_expired` ride with a view after 3 s, and that view applies at the
/// ride's step.
async fn honoured<S>(case: &Case<'_, S>) -> Outcome
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let text = case.mint.next("ride-view-head");
    let step = match ride_step(case, |head, mid, ordinal| {
        ScriptedRide::view_after(SLOW_VIEW_MS, ride_view(head, mid, ordinal, &text))
    })
    .await?
    {
        Ok(step) => step,
        Err(reason) => return Ok(Err(reason)),
    };
    if !step.input.contains(&text) {
        return Err(format!(
            "the provider answered the ttl_expired ride with a view after {SLOW_VIEW_MS} ms, \
             within the Setup's ride_wait_ms {RIDE_WAIT_MS}, but the ride's step went to the \
             model without it: a runner holds the step for a rewrite ride as long as Setup's \
             ride_wait_ms says, and the ride's view applies at that step"
        ));
    }
    Ok(Ok(()))
}

/// The provider answers the ride after 2.5 s: while it is pending past
/// 1 s, the run reads `compaction_wait`.
async fn slow_reported<S>(case: &Case<'_, S>) -> Outcome
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let step = match ride_step(case, |_, _, _| ScriptedRide::nothing_after(SLOW_NOTHING_MS)).await?
    {
        Ok(step) => step,
        Err(reason) => return Ok(Err(reason)),
    };
    if !step.reported {
        return Err(format!(
            "the provider answered the ttl_expired ride after {SLOW_NOTHING_MS} ms, but the run \
             never read {RUN_REASON_COMPACTION_WAIT} while the step waited: a runner reports a \
             ride whose answer has been pending for 1 s"
        ));
    }
    Ok(Ok(()))
}

/// The provider answers the ride within 200 ms: the run never reads
/// `compaction_wait`.
async fn fast_unreported<S>(case: &Case<'_, S>) -> Outcome
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let step = match ride_step(case, |_, _, _| ScriptedRide::nothing_after(FAST_MS)).await? {
        Ok(step) => step,
        Err(reason) => return Ok(Err(reason)),
    };
    if step.reported {
        return Err(format!(
            "the provider answered the ttl_expired ride within {FAST_MS} ms, yet the run read \
             {RUN_REASON_COMPACTION_WAIT}: a ride answered within 1 s is never reported"
        ));
    }
    Ok(Ok(()))
}

/// Setup answers a `ride_wait_ms` one above the runner's own cap: the
/// runner treats it as a failed Setup naming `ride_wait_ms`, and makes no
/// model call.
async fn above_cap<S>(case: &Case<'_, S>, cap: u64) -> Outcome
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let marker = case.mint.next("ride-over-cap");
    let mut session = install(
        case,
        "ride-over-cap",
        CompactionSetup::ReadyWithRideWait {
            initial: initial(&marker),
            cache_ttl: None,
            ride_wait_ms: cap.saturating_add(1),
        },
        Script {
            turns: vec![compaction::text(&format!("{marker}-answer"))],
        },
    )
    .await?;
    let reply: SendReply =
        compaction::send(case, &mut session, &case.mint.next("ride-prompt")).await?;
    let run_id = match &reply.run_id {
        Some(id) => id.clone(),
        None => {
            head(&session.route)
                .await?
                .last_run_state
                .ok_or("the send has no observable run")?
                .run_id
        }
    };
    for _ in 0..crate::drive::MAX_POLLS {
        if !observation(case.subject, &session.name)
            .await?
            .model_inputs
            .is_empty()
        {
            release_model(case.subject, &session.name).await?;
            return Err(format!(
                "the runner accepted a Setup answer whose ride_wait_ms {} is above the \
                 ride_wait_max_ms {cap} its own Setup request carried, and made a model call: \
                 such an answer is a failed Setup naming {RIDE_WAIT_MS_FIELD}",
                cap.saturating_add(1)
            ));
        }
        let result = run_result(&session.route, &run_id).await?;
        if result.run_state().is_terminal() {
            let error = result.error.clone();
            let named = error
                .as_ref()
                .and_then(|error| error.reason.as_deref())
                .is_some_and(|reason| reason.contains(RIDE_WAIT_MS_FIELD));
            if result.run_state() != RunState::Error
                || error.as_ref().map(|error| error.provider_code.as_str())
                    != Some(COMPACTION_UNAVAILABLE)
                || !named
            {
                return Err(format!(
                    "a Setup answer whose ride_wait_ms is above the runner's cap {cap} must fail \
                     as compaction_unavailable with a reason naming {RIDE_WAIT_MS_FIELD}; the run \
                     ended {} with {error:?}",
                    result.state
                ));
            }
            return Ok(Ok(()));
        }
        case.subject.pause().await;
    }
    Err("the run whose Setup answered a ride_wait_ms above the cap never ended".into())
}
