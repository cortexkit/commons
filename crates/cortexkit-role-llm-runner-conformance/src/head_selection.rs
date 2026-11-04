//! Durable next-run selection, refusal atomicity and host-side switch ordering.

use cortexkit_role_llm_runner::{
    capabilities,
    describe::RoleDescribe,
    ops,
    read::HeadMeta,
    send::SendReply,
    session_change::{RefreshReply, RefreshRequest},
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_error, expect_ok, run_result, wait_run_end},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

async fn head<R: RunnerRoute>(route: &R) -> Result<HeadMeta, String> {
    decode(
        "head",
        expect_ok("head", call(route, ops::SESSION_HEAD, json!({})).await?)?,
    )
}

async fn send<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    tier: Option<&str>,
) -> Result<SendReply, String>
where
    S::Route: RunnerRoute,
{
    let id = case.mint.next("selection-send");
    let mut params = case.params(session, "answer briefly", &id, None);
    if let Some(tier) = tier {
        params["service_tier"] = json!(tier);
    }
    let sent = decode(
        "send",
        expect_ok(
            "send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first.get_or_insert((id, "answer briefly".into()));
    wait_run_end(case.subject, &session.route, case.declared, None, &sent).await?;
    Ok(sent)
}

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "head-selection",
            Script {
                turns: (0..3)
                    .map(|_| ScriptedTurn::new(vec![ScriptedPart::Text("answer".into())]))
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
    if !describe.declares(capabilities::HEAD_SELECTION) {
        return Ok(Ending::Inapplicable(
            "head_selection is not declared".into(),
        ));
    }
    let Some([model, unsupported]) = case
        .subject
        .head_selection_models(&session.name)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no head-selection model fixture".into(),
        ));
    };
    if matches!(
        name,
        "head_selection_plan_tier"
            | "head_selection_send_overrides"
            | "head_selection_unsupported_tier"
            | "head_selection_absent_preserves_choice"
    ) && !describe.declares(capabilities::SERVICE_TIER)
    {
        return Ok(Ending::Inapplicable("service_tier is not declared".into()));
    }
    if describe.declares(capabilities::SERVICE_TIER)
        && model.service_tier.map(|tier| tier.as_str()) != Some("fast")
    {
        return Err(
            "head-selection fixture must provide a fast route without ultrafast support".into(),
        );
    }
    let params = case.params(
        &session,
        "answer briefly",
        &case.mint.next("initial-selection"),
        None,
    );
    let Some(mut plan) = params.get("plan").and_then(Value::as_object).cloned() else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no admission plan".into(),
        ));
    };
    send(case, &mut session, None).await?;
    let before = head(&session.route).await?;
    if before.service_tier.as_deref().unwrap_or("standard") != "standard" {
        return Err("head-selection fixture must begin with a standard default tier".into());
    }
    if name == "head_selection_immediate_head"
        && before.model.as_ref().is_some_and(|before| {
            before.provider == model.provider
                && before.model == model.model
                && before.variant == model.variant
        })
        && model.service_tier.unwrap_or_default().is_standard()
    {
        return Err("head-selection fixture did not change either choice".into());
    }
    plan.insert("model".into(), json!(model));
    let refusing = matches!(
        name,
        "head_selection_unsupported_tier" | "head_selection_unsupported_model"
    );
    if name == "head_selection_unsupported_tier" {
        plan["model"]["service_tier"] = json!("ultrafast");
    }
    if name == "head_selection_unsupported_model" {
        plan.insert("model".into(), json!(unsupported));
    }
    let request = RefreshRequest::new(plan.clone(), "immediately", 1);
    let reply = call(&session.route, ops::SESSION_REFRESH, json!(request)).await?;
    if refusing {
        let error = expect_error("unsupported plan selection", reply)?;
        let (field, reason) = if name == "head_selection_unsupported_tier" {
            ("plan.model.service_tier", "unsupported_tier")
        } else {
            ("plan.model", "unsupported_model")
        };
        if error.code != "invalid_params"
            || error.detail != Some(json!({"field":field,"reason":reason}))
        {
            return Err(format!(
                "expected invalid_params {field} {reason}, got {error:?}"
            ));
        }
        let after = head(&session.route).await?;
        if after.model != before.model || after.service_tier != before.service_tier {
            return Err("refused refresh changed the head choice".into());
        }
        return Ok(Ending::Passed);
    }
    let accepted: RefreshReply = decode("refresh", expect_ok("refresh", reply)?)?;
    if accepted.accepted_generation != 1 {
        return Err("refresh did not accept generation 1".into());
    }
    match name {
        "head_selection_absent_preserves_choice" => {
            let before = head(&session.route).await?;
            plan.remove("model");
            expect_ok(
                "absent model refresh",
                call(
                    &session.route,
                    ops::SESSION_REFRESH,
                    json!(RefreshRequest::new(plan.clone(), "immediately", 2)),
                )
                .await?,
            )?;
            let after = head(&session.route).await?;
            if after.model != before.model || after.service_tier != before.service_tier {
                return Err("absent plan.model cleared or changed the recorded choice".into());
            }
            let mut standard = model.clone();
            standard.service_tier =
                Some(cortexkit_role_llm_runner::service_tier::ServiceTier::Standard);
            plan.insert("model".into(), json!(standard));
            expect_ok(
                "standard model refresh",
                call(
                    &session.route,
                    ops::SESSION_REFRESH,
                    json!(RefreshRequest::new(plan.clone(), "immediately", 3)),
                )
                .await?,
            )?;
            if head(&session.route)
                .await?
                .service_tier
                .as_deref()
                .unwrap_or("standard")
                != "standard"
            {
                return Err("explicit standard did not reset the recorded default tier".into());
            }
            plan.insert("model".into(), json!(model));
            expect_ok(
                "restore fast selection",
                call(
                    &session.route,
                    ops::SESSION_REFRESH,
                    json!(RefreshRequest::new(plan.clone(), "immediately", 4)),
                )
                .await?,
            )?;
            let mut omitted = model.clone();
            omitted.service_tier = None;
            plan.insert("model".into(), json!(omitted));
            expect_ok(
                "omitted tier refresh",
                call(
                    &session.route,
                    ops::SESSION_REFRESH,
                    json!(RefreshRequest::new(plan, "immediately", 5)),
                )
                .await?,
            )?;
            if head(&session.route).await?.service_tier.is_some() {
                return Err(
                    "present plan.model with omitted tier did not reset fast to standard".into(),
                );
            }
        }
        "head_selection_immediate_head" => {
            let after = head(&session.route).await?;
            let selected = after
                .model
                .as_ref()
                .ok_or("accepted selection missing from head.model")?;
            if selected.provider != model.provider
                || selected.model != model.model
                || selected.variant != model.variant
                || after.service_tier.as_deref().unwrap_or("standard")
                    != model.service_tier.unwrap_or_default().as_str()
            {
                return Err(format!(
                    "head did not immediately reflect accepted selection: {after:?}"
                ));
            }
        }
        "head_selection_unchanged_after_host_switch" => {
            let Some(switched) = case
                .subject
                .switch_head_model(&session.name)
                .await
                .map_err(|e| e.to_string())?
            else {
                return Ok(Ending::Inapplicable(
                    "adapter cannot record a host-side model switch".into(),
                ));
            };
            if switched.provider == model.provider
                && switched.model == model.model
                && switched.variant == model.variant
            {
                return Err("host-side switch fixture did not change the model".into());
            }
            let request = RefreshRequest::new(plan, "immediately", 2);
            expect_ok(
                "unchanged refresh",
                call(&session.route, ops::SESSION_REFRESH, json!(request)).await?,
            )?;
            if head(&session.route).await?.model.as_ref() != Some(&switched) {
                return Err("unchanged refresh overrode the later host-side switch".into());
            }
        }
        "head_selection_plan_tier" | "head_selection_send_overrides" => {
            let explicit = (name == "head_selection_send_overrides").then_some("standard");
            let sent = send(case, &mut session, explicit).await?;
            let result = run_result(
                &session.route,
                sent.run_id.as_deref().ok_or("send has no run id")?,
            )
            .await?;
            let usage = result.usage.ok_or("run has no usage")?;
            if usage.service_tier_applied.as_deref().unwrap_or("standard")
                != explicit.unwrap_or("fast")
            {
                return Err(format!("run did not apply the expected tier: {usage:?}"));
            }
        }
        _ => return Err(format!("unknown head selection case {name}")),
    }
    Ok(Ending::Passed)
}
