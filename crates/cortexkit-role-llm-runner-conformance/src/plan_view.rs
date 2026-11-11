//! Applied-plan inspection through ordinary sends and the session-bound read.

use cortexkit_role_llm_runner::{
    capabilities,
    describe::RoleDescribe,
    errors, ops,
    plan_view::{PlanView, PlanViewRequest, System},
    send::SendReply,
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_ok, wait_run_end},
    route::{Reply, RunnerRoute},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let session = case
        .open(
            "plan-view",
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(
                    "plan-answer".into(),
                )])],
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
    if !describe.declares(capabilities::PLAN_VIEW) {
        return Ok(Ending::Inapplicable("plan_view is not declared".into()));
    }
    let no_plan = name == "plan_view_no_plan";
    let id = case.mint.next("plan-view-send");
    let mut params = case.params(&session, "plan prompt", &id, None);
    if no_plan {
        params.as_object_mut().unwrap().remove("plan");
    } else if params.get("plan").and_then(Value::as_object).is_none() {
        return Ok(Ending::Inapplicable(
            "adapter supplies no admission plan".into(),
        ));
    }
    let sent = call(&session.route, ops::SESSION_SEND, params).await?;
    if no_plan
        && matches!(&sent, Reply::Error(error) if error.code == errors::INVALID_PARAMS
            && error.detail.as_ref().and_then(|detail| detail.get("field")).and_then(Value::as_str) == Some("plan"))
    {
        return Ok(Ending::Inapplicable(
            "runner requires an admission plan".into(),
        ));
    }
    let send: SendReply = decode(
        "plan_view creating send",
        expect_ok("plan_view creating send", sent)?,
    )?;
    wait_run_end(case.subject, &session.route, case.declared, None, &send).await?;
    let reply = call(
        &session.route,
        ops::SESSION_PLAN_VIEW,
        serde_json::to_value(PlanViewRequest::new()).unwrap(),
    )
    .await?;
    match name {
        "plan_view_reply" => {
            let _: PlanView = decode("plan_view reply", expect_ok("plan_view reply", reply)?)?;
        }
        "plan_view_items" => {
            let value = expect_ok("plan_view items", reply)?;
            let system: System = decode(
                "plan_view system",
                value
                    .get("system")
                    .cloned()
                    .ok_or("plan_view lacks system")?,
            )?;
            system
                .check()
                .map_err(|field| format!("plan_view invalid {field}"))?;
        }
        "plan_view_no_plan" => {
            let error = expect_error("plan_view without plan", reply)?;
            expect_code("plan_view without plan", &error, errors::NO_PLAN)?;
        }
        _ => return Err(format!("unknown plan_view case {name}")),
    }
    Ok(Ending::Passed)
}
