//! Controlled idle-time checks at the scripted provider boundary.

use std::time::Duration;

use cortexkit_role_llm_runner::{describe::RoleDescribe, ops, send::SendReply};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_ok, head, read_all, run_result, wait_run_end},
    subject::{LlmRunnerSubject, Script, ScriptedTurn},
    RunnerRoute, ScriptedUsage,
};

/// Cache prices installed in the runner's scripted-provider model fixture.
/// Both rates use the same integer currency unit per million tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeepWarmPrices {
    pub cache_read: u64,
    pub cache_write: u64,
}

impl KeepWarmPrices {
    pub fn new(cache_read: u64, cache_write: u64) -> Self {
        Self {
            cache_read,
            cache_write,
        }
    }
}

/// One completed request captured outside the runner at provider ingress.
/// There is deliberately no warm/real label: the kit knows when it sends.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeepWarmRequest {
    pub at_ms: u64,
    pub request_bytes: Vec<u8>,
    pub cache_prefix: Vec<u8>,
    pub usage: ScriptedUsage,
}

impl KeepWarmRequest {
    pub fn new(
        at_ms: u64,
        request_bytes: Vec<u8>,
        cache_prefix: Vec<u8>,
        usage: ScriptedUsage,
    ) -> Self {
        Self {
            at_ms,
            request_bytes,
            cache_prefix,
            usage,
        }
    }
}

/// A contiguous slice of the session's provider journal after a cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeepWarmRequests {
    pub now_ms: u64,
    pub next_cursor: u64,
    pub requests: Vec<KeepWarmRequest>,
}

impl KeepWarmRequests {
    pub fn new(now_ms: u64, next_cursor: u64, requests: Vec<KeepWarmRequest>) -> Self {
        Self {
            now_ms,
            next_cursor,
            requests,
        }
    }
}

const TTL_MS: u64 = 10_000;
const CACHED_TOKENS: u64 = 1_000;

fn fixture_usage() -> ScriptedUsage {
    // Every provider request is a cache hit. A rewrite of the same prefix would
    // be charged at the write rate, so no ordinary input/output price is needed.
    ScriptedUsage::new(0, CACHED_TOKENS, 0, 0)
}

fn validate_idle(
    real: &KeepWarmRequest,
    idle: &[KeepWarmRequest],
    until_ms: u64,
    prices: &KeepWarmPrices,
    exhaust: bool,
) -> Result<(), String> {
    let rewrite = u128::from(CACHED_TOKENS) * u128::from(prices.cache_write);
    let mut spent = 0_u128;
    let mut touched = real.at_ms;
    for request in idle {
        if spent >= rewrite {
            return Err("provider received an extra warm after one rewrite's cost".into());
        }
        if request.at_ms <= touched || request.at_ms - touched >= TTL_MS || request.at_ms > until_ms
        {
            return Err("idle request did not touch the cache before its lifetime ended".into());
        }
        if request.request_bytes != real.request_bytes || request.cache_prefix != real.cache_prefix
        {
            return Err(
                "idle request did not resend the last real request's bytes and prefix".into(),
            );
        }
        if request.usage != fixture_usage() {
            return Err(format!(
                "idle provider usage differs from fixture: {:?}",
                request.usage
            ));
        }
        spent += u128::from(request.usage.cache_read_tokens) * u128::from(prices.cache_read)
            + u128::from(request.usage.cache_write_tokens) * u128::from(prices.cache_write);
        touched = request.at_ms;
    }
    if idle.is_empty() {
        return Err("no idle warm reached the provider before cache expiry".into());
    }
    if exhaust && spent != rewrite {
        return Err(format!(
            "idle warming stopped at cost {spent}, expected exactly one rewrite ({rewrite})"
        ));
    }
    Ok(())
}

async fn snapshot<R: RunnerRoute>(route: &R, run_id: &str) -> Result<Value, String> {
    let (_, messages) = read_all(route).await?;
    let head = head(route).await?;
    let result = run_result(route, run_id).await?;
    if result.usage.as_ref().is_none_or(|usage| usage.steps != 1) {
        return Err("the real run must report exactly one model step".into());
    }
    Ok(json!({
        "messages": messages,
        "history_version": head.history_version,
        "last_run_state": head.last_run_state,
        "result": result,
    }))
}

async fn journal<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &str,
    cursor: u64,
) -> Result<KeepWarmRequests, String>
where
    S::Route: RunnerRoute,
{
    let observed = case
        .subject
        .keep_warm_requests(session, cursor)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("provider journal disappeared after opting into the fixture")?;
    if observed.next_cursor != cursor + observed.requests.len() as u64
        || observed
            .requests
            .iter()
            .any(|request| request.at_ms > observed.now_ms)
    {
        return Err("provider journal has an inconsistent cursor or clock".into());
    }
    Ok(observed)
}

async fn real_send<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    mode: &str,
    previous: Option<&str>,
) -> Result<String, String>
where
    S::Route: RunnerRoute,
{
    let prompt = case.mint.next("warm-real-prompt");
    let id = case.mint.next("warm-real-send");
    let mut params = case.params(session, &prompt, &id, None);
    if session.first.is_none() {
        params.as_object_mut().unwrap().remove("cache");
        params
            .get_mut("plan")
            .and_then(Value::as_object_mut)
            .ok_or("adapter supplies no plan for keep-warm fixture")?
            .insert("cache".into(), json!({"ttl":"short","keep_warm":mode}));
    }
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first.get_or_insert((id, prompt));
    let end = wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        previous,
        &reply,
    )
    .await?;
    Ok(end.run_id)
}

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "keep-warm",
            Script {
                turns: (0..2)
                    .map(|_| {
                        ScriptedTurn::new(vec![case.mint.text("warm-answer").0])
                            .with_usage(fixture_usage())
                    })
                    .collect(),
            },
        )
        .await?;
    let describe: RoleDescribe = decode(
        "describe",
        expect_ok(
            "describe",
            call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
        )?,
    )?;
    if !describe.declares("cache_policy") {
        return Err("adapter declares cache_policy but role.describe does not".into());
    }
    let prices = KeepWarmPrices::new(1, 3);
    let Some(applied) = case
        .subject
        .configure_keep_warm_prices(case.handle.inner(), &session.name, &prices)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(Ending::Inapplicable(
            "adapter has no idle keep-warm price fixture".into(),
        ));
    };
    if applied != prices {
        return Err(format!(
            "runner used {applied:?}, not fixture prices {prices:?}"
        ));
    }
    if case
        .subject
        .advance_warm_clock(case.handle.inner(), Duration::ZERO)
        .await
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Ok(Ending::Inapplicable(
            "adapter has no keep-warm scheduler clock".into(),
        ));
    }
    let Some(initial) = case
        .subject
        .keep_warm_requests(&session.name, 0)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(Ending::Inapplicable(
            "adapter has no independent provider journal".into(),
        ));
    };
    if initial.next_cursor != 0 || !initial.requests.is_empty() {
        return Err("fresh session already has provider requests before a real send".into());
    }
    case.subject
        .set_observed_cache_lifetime(&session.name, Some(TTL_MS))
        .await
        .map_err(|e| e.to_string())?;
    let off = name == "keep_warm_off_sends_no_warms";
    let mode = if off { "off" } else { "auto" };
    let mut cursor = 0;
    let mut previous = None;
    let mut last_prefix = None;
    let stretches = if name == "keep_warm_auto_rearms_after_real_request" {
        2
    } else {
        1
    };
    for _ in 0..stretches {
        let run_id = real_send(case, &mut session, mode, previous.as_deref()).await?;
        let real = journal(case, &session.name, cursor).await?;
        if real.requests.len() != 1 {
            return Err(format!(
                "one real send made {} provider requests",
                real.requests.len()
            ));
        }
        let request = &real.requests[0];
        if request.request_bytes.is_empty()
            || request.cache_prefix.is_empty()
            || request.usage != fixture_usage()
        {
            return Err(
                "real request is missing bytes/prefix or the scripted cache-hit usage".into(),
            );
        }
        if request.at_ms != real.now_ms {
            return Err("provider clock moved without the kit advancing time".into());
        }
        if last_prefix.as_ref() == Some(&request.cache_prefix) {
            return Err("second real send did not establish a different cache prefix".into());
        }
        let before = snapshot(&session.route, &run_id).await?;
        let span = if name == "keep_warm_auto_warms_idle_session" {
            TTL_MS - 1
        } else {
            4 * TTL_MS
        };
        case.subject
            .advance_warm_clock(case.handle.inner(), Duration::from_millis(span))
            .await
            .map_err(|e| e.to_string())?
            .ok_or("scheduler clock disappeared after opting into the fixture")?;
        let idle = journal(case, &session.name, real.next_cursor).await?;
        if idle.now_ms.checked_sub(real.now_ms) != Some(span) {
            return Err("scheduler clock did not advance by the requested duration".into());
        }
        if off {
            if !idle.requests.is_empty() {
                return Err("keep_warm off sent an idle provider request".into());
            }
        } else {
            validate_idle(
                request,
                &idle.requests,
                idle.now_ms,
                &prices,
                name != "keep_warm_auto_warms_idle_session",
            )?;
        }
        if snapshot(&session.route, &run_id).await? != before {
            return Err("idle warming changed the transcript, run or real-step usage".into());
        }
        cursor = idle.next_cursor;
        previous = Some(run_id);
        last_prefix = Some(request.cache_prefix.clone());
    }
    Ok(Ending::Passed)
}
