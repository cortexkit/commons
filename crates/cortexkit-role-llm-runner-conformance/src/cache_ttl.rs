//! The live `cache_ttl` cases (compaction-provider CONTRACT §8.1): which
//! prompt-cache lifetime a runner judges `ttl_expired` by, observed through
//! the advance notices it publishes before each model request.
//!
//! Each case drives a compaction session whose Setup answer carries one
//! `cache_ttl` entry, moves the session's prompt-cache clock with the
//! adapter's hook between prompts, and reads whether the notice of the next
//! step declares `ttl_expired`. A control session first checks that the
//! runner declares it at all for an expired entry of its own model: a
//! runner may never declare it, and then the rules cannot be asked.

use cortexkit_role_compaction_provider::{cache_ttl::CacheTtl, notice::headers};
use cortexkit_role_llm_runner::send::SendReply;

use crate::{
    cases::{Case, Ending, Session},
    compaction::{self, initial, install, observation, release_model, CompactionSetup},
    subject::{LlmRunnerSubject, Script},
    RunnerRoute,
};

/// The token a notice declares an expired prompt cache with.
const TTL_EXPIRED: &str = "ttl_expired";

/// The lifetime of the entries the cases install, and how far the clock
/// moves to pass it.
const SHORT_MS: u64 = 1_000;
const PAST_SHORT_MS: u64 = 5_000;

/// The observed lifetime of the model-change case, and how far the clock
/// then moves to pass it.
const OBSERVED_MS: u64 = 60_000;
const PAST_OBSERVED_MS: u64 = 70_000;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match control(case).await? {
        Ok(()) => {}
        Err(reason) => return Ok(Ending::Inapplicable(reason)),
    }
    let outcome = match name {
        "cache_ttl_entry_for_another_model_ignored" => other_model(case).await?,
        "cache_ttl_configured_null_never_expires" => configured_null(case).await?,
        "cache_ttl_missed_model_change_falls_through_to_observed" => missed_change(case).await?,
        other => return Err(format!("unknown cache_ttl case {other}")),
    };
    Ok(match outcome {
        Ok(()) => Ending::Passed,
        Err(reason) => Ending::Inapplicable(reason),
    })
}

/// A compaction session on `model`, whose Setup answers with `entry` and
/// whose requests carry `observed`; `Err` (inapplicable) with the reason
/// when the adapter lacks a hook.
struct TtlSession<R> {
    session: Session<R>,
    /// Notices read so far.
    seen: usize,
}

async fn open<S>(
    case: &Case<'_, S>,
    model: &str,
    entry: CacheTtl,
    observed: Option<u64>,
    prompts: usize,
) -> Result<Result<TtlSession<S::Route>, String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let marker = case.mint.next("ttl");
    let turns = (0..prompts)
        .map(|n| compaction::text(&format!("{marker}-answer-{n}")))
        .collect();
    let session = install(
        case,
        "ttl",
        CompactionSetup::ReadyWithCacheTtl {
            initial: initial(&marker),
            cache_ttl: entry,
        },
        Script { turns },
    )
    .await?;
    if let Err(e) = case.subject.switch_model(&session.name, model).await {
        return Ok(Err(format!(
            "the adapter cannot choose the session's model: {e}"
        )));
    }
    if let Err(e) = case
        .subject
        .set_observed_cache_lifetime(&session.name, observed)
        .await
    {
        return Ok(Err(format!(
            "the adapter cannot set the lifetime the runner observes in its request: {e}"
        )));
    }
    Ok(Ok(TtlSession { session, seen: 0 }))
}

/// Advance the clock by `advance` milliseconds, run one prompt, and say
/// whether the notices published for it declare `ttl_expired`.
async fn step<S>(
    case: &Case<'_, S>,
    ttl: &mut TtlSession<S::Route>,
    advance: u64,
) -> Result<Result<bool, String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let name = ttl.session.name.clone();
    if advance > 0 {
        if let Err(e) = case
            .subject
            .advance_prompt_cache_clock(&name, advance)
            .await
        {
            return Ok(Err(format!("the adapter has no prompt-cache clock: {e}")));
        }
    }
    let reply: SendReply =
        compaction::send(case, &mut ttl.session, &case.mint.next("ttl-prompt")).await?;
    release_model(case.subject, &name).await?;
    compaction::result(case, &ttl.session, &reply).await?;
    let notices = observation(case.subject, &name).await?.notices;
    let new = &notices[ttl.seen.min(notices.len())..];
    ttl.seen = notices.len();
    if new.is_empty() {
        return Err("the runner published no advance notice for a model request".into());
    }
    Ok(Ok(new.iter().any(|notice| {
        notice
            .get(headers::EVENTS)
            .is_some_and(|events| events.split(',').any(|token| token == TTL_EXPIRED))
    })))
}

/// The runner declares `ttl_expired` once its own model's configured
/// lifetime has passed. `Ok(Err(reason))` when it does not, or when the
/// adapter lacks a hook: the cases are then inapplicable.
pub(crate) async fn control<S>(case: &Case<'_, S>) -> Result<Result<(), String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let model = case.mint.next("model");
    let mut ttl = match open(
        case,
        &model,
        CacheTtl::new(&model, Some(SHORT_MS), true),
        None,
        2,
    )
    .await?
    {
        Ok(ttl) => ttl,
        Err(reason) => return Ok(Err(reason)),
    };
    match step(case, &mut ttl, 0).await? {
        Ok(false) => {}
        Ok(true) => {
            return Err(
                "the runner declared ttl_expired on a session's first request, before any time \
                 passed"
                    .into(),
            )
        }
        Err(reason) => return Ok(Err(reason)),
    }
    match step(case, &mut ttl, PAST_SHORT_MS).await? {
        Ok(true) => Ok(Ok(())),
        Ok(false) => Ok(Err(
            "the runner never declares ttl_expired, even once its own model's configured \
             lifetime has passed, so which lifetime it judges by cannot be observed"
                .into(),
        )),
        Err(reason) => Ok(Err(reason)),
    }
}

/// An entry naming another model is never used: with no other lifetime,
/// the runner declares no `ttl_expired`, however long the cache idles.
async fn other_model<S>(case: &Case<'_, S>) -> Result<Result<(), String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let model = case.mint.next("model");
    let other = case.mint.next("model");
    let mut ttl = match open(
        case,
        &model,
        CacheTtl::new(&other, Some(SHORT_MS), true),
        None,
        2,
    )
    .await?
    {
        Ok(ttl) => ttl,
        Err(reason) => return Ok(Err(reason)),
    };
    for advance in [0, PAST_SHORT_MS] {
        match step(case, &mut ttl, advance).await? {
            Ok(false) => {}
            Ok(true) => {
                return Err(format!(
                    "the runner declared ttl_expired on {model} by a cache_ttl entry for {other}: \
                     a runner uses an entry only for a request to the model it names"
                ))
            }
            Err(reason) => return Ok(Err(reason)),
        }
    }
    Ok(Ok(()))
}

/// A configured entry whose `ms` is `null` means never: no `ttl_expired`,
/// even when the runner observed a lifetime that has passed.
async fn configured_null<S>(case: &Case<'_, S>) -> Result<Result<(), String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let model = case.mint.next("model");
    let mut ttl = match open(
        case,
        &model,
        CacheTtl::new(&model, None, true),
        Some(SHORT_MS),
        2,
    )
    .await?
    {
        Ok(ttl) => ttl,
        Err(reason) => return Ok(Err(reason)),
    };
    for advance in [0, PAST_SHORT_MS] {
        match step(case, &mut ttl, advance).await? {
            Ok(false) => {}
            Ok(true) => {
                return Err(
                    "the runner declared ttl_expired under a configured cache_ttl entry with ms \
                     null, by the lifetime it observed: a configured null means never declare \
                     ttl_expired, ahead of any observed lifetime"
                        .into(),
                )
            }
            Err(reason) => return Ok(Err(reason)),
        }
    }
    Ok(Ok(()))
}

/// After a switch whose `model_change` ride brought no entry, the runner
/// never keeps the old model's lifetime: it judges by the lifetime it
/// observes in its request, so no `ttl_expired` before that lifetime
/// passes, and `ttl_expired` once it has.
async fn missed_change<S>(case: &Case<'_, S>) -> Result<Result<(), String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let first = case.mint.next("model");
    let second = case.mint.next("model");
    let mut ttl = match open(
        case,
        &first,
        CacheTtl::new(&first, Some(SHORT_MS), true),
        None,
        3,
    )
    .await?
    {
        Ok(ttl) => ttl,
        Err(reason) => return Ok(Err(reason)),
    };
    if let Err(reason) = step(case, &mut ttl, 0).await? {
        return Ok(Err(reason));
    }
    let name = ttl.session.name.clone();
    if let Err(e) = case.subject.switch_model(&name, &second).await {
        return Ok(Err(format!(
            "the adapter cannot switch the session's model: {e}"
        )));
    }
    if let Err(e) = case
        .subject
        .set_observed_cache_lifetime(&name, Some(OBSERVED_MS))
        .await
    {
        return Ok(Err(format!(
            "the adapter cannot set the lifetime the runner observes: {e}"
        )));
    }
    match step(case, &mut ttl, PAST_SHORT_MS).await? {
        Ok(false) => {}
        Ok(true) => {
            return Err(format!(
                "after a switch from {first} to {second} whose model_change ride brought no \
                 entry, the runner declared ttl_expired by {first}'s lifetime: a runner never \
                 keeps a previous model's value, and falls through to the lifetime it observes"
            ))
        }
        Err(reason) => return Ok(Err(reason)),
    }
    match step(case, &mut ttl, PAST_OBSERVED_MS).await? {
        Ok(true) => Ok(Ok(())),
        Ok(false) => Err(format!(
            "after a switch to {second} with no entry for it, the lifetime the runner observed \
             in its request ({OBSERVED_MS} ms) passed without ttl_expired: it falls through to \
             the observed lifetime"
        )),
        Err(reason) => Ok(Err(reason)),
    }
}
