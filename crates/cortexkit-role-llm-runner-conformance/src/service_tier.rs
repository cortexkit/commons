//! Tier refusals, truthful usage, and cross-provider ride arrival order.

use cortexkit_role_llm_runner::{
    capabilities, describe::RoleDescribe, ops, send::SendReply, service_tier::ServiceTier,
};
use serde_json::json;

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_error, expect_ok, run_result, wait_run_end},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

async fn send<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    tier: ServiceTier,
) -> Result<SendReply, String>
where
    S::Route: RunnerRoute,
{
    let id = case.mint.next("tier-send");
    let mut params = case.params(session, "answer briefly", &id, None);
    params["service_tier"] = json!(tier);
    let reply = decode(
        "tier send",
        expect_ok(
            "tier send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first.get_or_insert((id, "answer briefly".into()));
    Ok(reply)
}

async fn refusal<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    tier: &str,
    reason: &str,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let mut params = case.params(
        session,
        "answer briefly",
        &case.mint.next("tier-refused"),
        None,
    );
    params["service_tier"] = json!(tier);
    let error = expect_error(
        "tier send",
        call(&session.route, ops::SESSION_SEND, params).await?,
    )?;
    if error.code != "invalid_params"
        || error.detail != Some(json!({"field":"service_tier","reason":reason}))
    {
        return Err(format!(
            "expected invalid_params service_tier {reason}, got {error:?}"
        ));
    }
    Ok(())
}

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "service-tier",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::Text("first".into())]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("second".into())]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("third".into())]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("fourth".into())]),
                ],
            },
        )
        .await?;
    let describe: RoleDescribe = decode(
        "role.describe",
        expect_ok(
            "role.describe",
            call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
        )?,
    )?;
    if !describe.declares(capabilities::SERVICE_TIER) {
        return Ok(Ending::Inapplicable(
            "role.describe does not declare service_tier".into(),
        ));
    }
    if !case
        .subject
        .configure_service_tier(&session.name, name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(
            "configure_service_tier cannot script this tier fixture".into(),
        ));
    }
    match name {
        "service_tier_unsupported_before_model" => {
            let before = case
                .subject
                .model_invocations(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            for tier in [ServiceTier::Ultrafast.as_str(), "future-tier"] {
                refusal(case, &session, tier, "unsupported_tier").await?;
            }
            let after = case
                .subject
                .model_invocations(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            if before != 0 || after != before {
                return Err(format!(
                    "unsupported tier reached model: {before} -> {after} calls"
                ));
            }
        }
        "service_tier_run_fixed" => {
            case.subject
                .hold_next_model_step(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            let reply = send(case, &mut session, ServiceTier::Fast).await?;
            case.subject
                .await_model_step(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            let mut result = refusal(
                case,
                &session,
                ServiceTier::Standard.as_str(),
                "run_tier_fixed",
            )
            .await;
            if result.is_ok() && describe.declares(capabilities::STEER) {
                let mut params = case.params(
                    &session,
                    "steer",
                    &case.mint.next("tier-steer"),
                    Some("steer"),
                );
                params["service_tier"] = json!("standard");
                result = match expect_error(
                    "tier steer",
                    call(&session.route, ops::SESSION_SEND, params).await?,
                ) {
                    Ok(error)
                        if error.code == "invalid_params"
                            && error.detail
                                == Some(
                                    json!({"field":"service_tier","reason":"run_tier_fixed"}),
                                ) =>
                    {
                        Ok(())
                    }
                    other => Err(format!(
                        "tier steer did not refuse run_tier_fixed: {other:?}"
                    )),
                };
            }
            case.subject
                .release_model_step(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            result?;
            wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
        }
        "service_tier_ride_before_changed_request" => {
            for tier in [
                ServiceTier::Standard,
                ServiceTier::Fast,
                ServiceTier::Fast,
                ServiceTier::Standard,
            ] {
                let reply = send(case, &mut session, tier).await?;
                wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
            }
            let requests = case
                .subject
                .model_requests(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            if requests.len() != 4 {
                return Err(format!(
                    "expected four tier requests, got {}",
                    requests.len()
                ));
            }
            for (request, (tier, rides)) in requests.iter().zip([
                (ServiceTier::Standard, 0),
                (ServiceTier::Fast, 1),
                (ServiceTier::Fast, 1),
                (ServiceTier::Standard, 2),
            ]) {
                if request
                    .service_tier_applied
                    .as_deref()
                    .unwrap_or("standard")
                    != tier.as_str()
                    || request.service_tier_change_rides != Some(rides)
                {
                    return Err(format!(
                        "tier ride did not precede exactly the changed request: {request:?}"
                    ));
                }
            }
        }
        "service_tier_unknown_never_echoed" => {
            let reply = send(case, &mut session, ServiceTier::Fast).await?;
            let end =
                wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
            let result = run_result(&session.route, &end.run_id).await?;
            let usage = result.usage.ok_or("fast run omitted usage")?;
            if usage.service_tier_applied.as_deref() != Some(ServiceTier::Fast.as_str())
                || usage.service_tier_served.is_some()
            {
                return Err(format!(
                    "unknown wire must report applied fast and no served tier: {usage:?}"
                ));
            }
        }
        _ => return Err(format!("unknown service-tier case {name}")),
    }
    Ok(Ending::Passed)
}
