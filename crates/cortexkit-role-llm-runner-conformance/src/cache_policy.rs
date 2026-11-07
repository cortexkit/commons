//! Cache admission and step-report checks without time-dependent warming claims.

use cortexkit_role_llm_runner::{
    describe::RoleDescribe, ops, send::SendReply, step_events::StepFinished,
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_error, expect_ok, wait_run_end},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

fn vectors() -> Value {
    serde_json::from_str(
        cortexkit_role_llm_runner::vectors::file("cache-policy.json")
            .expect("shipped cache vectors"),
    )
    .expect("valid cache vectors")
}

fn policy_with_known_zone<S: LlmRunnerSubject>(case: &Case<'_, S>, value: &Value) -> Value
where
    S::Route: RunnerRoute,
{
    let mut policy = value.clone();
    if let Some(hours) = policy.get_mut("hours").and_then(Value::as_object_mut) {
        hours.insert(
            "tz".into(),
            Value::String(case.subject.cache_policy_zone().into()),
        );
    }
    policy
}

async fn open<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Result<Session<S::Route>, String>
where
    S::Route: RunnerRoute,
{
    case.open(
        "cache-policy",
        Script {
            turns: (0..3)
                .map(|_| ScriptedTurn::new(vec![ScriptedPart::Text("cache-answer".into())]))
                .collect(),
        },
    )
    .await
}

fn params<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    id: &str,
    cache: Option<&Value>,
    plan: bool,
) -> Option<Value>
where
    S::Route: RunnerRoute,
{
    let mut params = case.params(session, "cache prompt", id, None);
    params.as_object_mut().unwrap().remove("cache");
    if plan {
        let object = params.get_mut("plan")?.as_object_mut()?;
        object.remove("cache");
        if let Some(cache) = cache {
            object.insert("cache".into(), cache.clone());
        }
    } else {
        params.as_object_mut().unwrap().remove("plan");
        if let Some(cache) = cache {
            params["cache"] = cache.clone();
        }
    }
    Some(params)
}

async fn accept<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    id: String,
    params: Value,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let reply: SendReply = decode(
        "cache send",
        expect_ok(
            "cache send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first.get_or_insert((id, "cache prompt".into()));
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    Ok(())
}

async fn refuse<R: RunnerRoute>(route: &R, params: Value, field: &str) -> Result<(), String> {
    let before = call(route, ops::SESSION_HEAD, json!({})).await?;
    let error = expect_error(
        "cache refusal",
        call(route, ops::SESSION_SEND, params).await?,
    )?;
    if error.code != "invalid_params"
        || error
            .detail
            .as_ref()
            .and_then(|detail| detail.get("field"))
            .and_then(Value::as_str)
            != Some(field)
    {
        return Err(format!(
            "expected invalid_params field {field}, got {error:?}"
        ));
    }
    if call(route, ops::SESSION_HEAD, json!({})).await? != before {
        return Err("refused cache send changed session state before acceptance".into());
    }
    Ok(())
}

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut session = open(case).await?;
    let described: RoleDescribe = decode(
        "describe",
        expect_ok(
            "describe",
            call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
        )?,
    )?;
    if !described.declares("cache_policy") {
        return Err("adapter declares cache_policy but role.describe does not".into());
    }
    let vectors = vectors();
    let plan = name.contains("_plan_");
    let good = json!({"ttl":"short","keep_warm":"off"});
    let probe = case.mint.next("cache-send");
    if params(case, &session, &probe, Some(&good), plan).is_none() {
        return Ok(Ending::Inapplicable(
            "adapter supplies no admission plan".into(),
        ));
    }
    match name {
        "cache_policy_plan_decode" | "cache_policy_send_decode" => {
            for value in vectors["valid"].as_array().unwrap() {
                let mut session = open(case).await?;
                let id = case.mint.next("cache-valid");
                let policy = policy_with_known_zone(case, value);
                let params = params(case, &session, &id, Some(&policy), plan)
                    .ok_or("adapter supplies no admission plan")?;
                accept(case, &mut session, id, params).await?;
            }
        }
        "cache_policy_plan_refusals" | "cache_policy_send_refusals" => {
            let mut invalid = vectors["invalid"].as_array().unwrap().clone();
            for value in &mut invalid {
                value["cache"] = policy_with_known_zone(case, &value["cache"]);
            }
            invalid.push(json!({"cache":vectors["unknown_zone"],"field":"hours.tz"}));
            for value in invalid {
                let mut session = open(case).await?;
                let id = case.mint.next("cache-invalid");
                let bad = params(case, &session, &id, Some(&value["cache"]), plan)
                    .ok_or("adapter supplies no admission plan")?;
                let field = format!(
                    "{}cache.{}",
                    if plan { "plan." } else { "" },
                    value["field"].as_str().unwrap()
                );
                refuse(&session.route, bad, &field).await?;
                let corrected = params(case, &session, &id, Some(&good), plan)
                    .ok_or("adapter supplies no admission plan")?;
                accept(case, &mut session, id, corrected).await?;
            }
        }
        "cache_policy_continuation_refused" => {
            let initial = params(case, &session, &probe, Some(&good), false).unwrap();
            accept(case, &mut session, probe, initial).await?;
            let id = case.mint.next("cache-continuation");
            refuse(
                &session.route,
                params(case, &session, &id, Some(&good), false).unwrap(),
                "cache",
            )
            .await?;
            let continuation = params(case, &session, &id, None, false).unwrap();
            accept(case, &mut session, id, continuation).await?;
        }
        "cache_policy_plan_and_send_refused" => {
            let mut both = params(case, &session, &probe, Some(&good), true)
                .ok_or("adapter supplies no admission plan")?;
            both["cache"] = good.clone();
            refuse(&session.route, both, "cache").await?;
            let corrected = params(case, &session, &probe, Some(&good), true).unwrap();
            accept(case, &mut session, probe, corrected).await?;
        }
        "cache_policy_step_report" => {
            let initial = params(case, &session, &probe, Some(&good), false).unwrap();
            accept(case, &mut session, probe, initial).await?;
            let observed = case
                .subject
                .step_event_observation(case.handle.inner(), &session.name)
                .await
                .map_err(|e| e.to_string())?;
            if observed.published.is_empty() {
                return Err("no model step report was published".into());
            }
            for published in observed.published {
                let step: StepFinished = decode("step report", published.body)?;
                let cache = step.cache.ok_or("model step omitted cache report")?;
                cache
                    .check()
                    .map_err(|field| format!("invalid step {field}"))?;
                if cache.ttl_ms.is_none() && cache.implicit != Some(true) {
                    return Err("model step omitted ttl_ms or implicit".into());
                }
                if cache.warming.is_none() {
                    return Err("model step omitted session warming state".into());
                }
            }
        }
        _ => return Err(format!("unknown cache case {name}")),
    }
    Ok(Ending::Passed)
}
