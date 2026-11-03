use cortexkit_role_llm_runner::{baseline::BaselineReply, errors, ops, read::ReadRequest};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_ok, head, read},
    subject::{LlmRunnerSubject, Script, ScriptedTurn},
    RunnerRoute,
};

pub(crate) async fn run<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let Some((carrier, owner)) = case.subject.scoped_session_start_stamps() else {
        return Ok(Ending::Inapplicable(
            "adapter cannot open a non-owner carrier route under a plain scope".into(),
        ));
    };
    if carrier.principal == owner.principal
        || carrier.scope != owner.scope
        || !owner
            .scope
            .as_ref()
            .is_some_and(|s| s.owner == owner.principal)
    {
        return Err(
            "plain-scope fixture must separate carrier and owner under the same scope".into(),
        );
    }
    let session = case
        .open(
            "scoped-start",
            Script {
                turns: vec![ScriptedTurn::new(vec![case.mint.text("scoped-answer").0])],
            },
        )
        .await?;
    let carrier_route = case
        .subject
        .session_route(case.handle.inner(), &session.name, &carrier)
        .await
        .map_err(|e| e.to_string())?;
    let owner_route = case
        .subject
        .session_route(case.handle.inner(), &session.name, &owner)
        .await
        .map_err(|e| e.to_string())?;
    let mut params = case.subject.send_fields(&session.name, true);
    params.extend(
        json!({"prompt": "carrier cannot create an owner's session",
        "send_id": case.mint.next("scoped-start-send"), "delivery": "queue"})
        .as_object()
        .unwrap()
        .clone(),
    );
    expect_code(
        "plain-scope carrier start",
        &expect_error(
            "plain-scope carrier start",
            call(&carrier_route, ops::SESSION_SEND, Value::Object(params)).await?,
        )?,
        errors::SCOPE_OWNER_MISMATCH,
    )?;
    let page = read(&owner_route, &ReadRequest::range(0)).await?;
    let meta = head(&owner_route).await?;
    if !page.messages.is_empty() || meta.last_ordinal.is_some() || meta.last_run_state.is_some() {
        return Err("refused carrier start left history or a run visible to the owner".into());
    }
    let baseline: BaselineReply = decode(
        "session.baseline",
        expect_ok(
            "owner baseline",
            call(&owner_route, ops::SESSION_BASELINE, json!({})).await?,
        )?,
    )?;
    if baseline.baseline.is_some() || baseline.state == "ready" {
        return Err(
            "refused carrier start left a visible session (ready baseline) for the owner".into(),
        );
    }
    Ok(Ending::Passed)
}

/// Plan grants cannot widen the sender frozen by flow-scope admission.
pub(crate) async fn plan_senders<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    use crate::{drive::expect_field, send_helpers::send_and_wait};
    use cortexkit_role_llm_runner::{delivery_order::PlanSender, rungs};
    use serde_json::Map;

    let Some((opener, owner)) = case.subject.flow_session_start_stamps() else {
        return Ok(Ending::Inapplicable(
            "adapter cannot open sessions under a flow scope".into(),
        ));
    };
    if opener.scope != owner.scope
        || !owner
            .scope
            .as_ref()
            .is_some_and(|scope| scope.owner == owner.principal)
    {
        return Err("flow fixture must attest opener and owner under the same scope".into());
    }
    let mut session = case
        .open(
            "frozen-plan",
            Script {
                turns: vec![ScriptedTurn::new(vec![case.mint.text("flow-finished").0])],
            },
        )
        .await?;
    session.route = case
        .subject
        .session_route(case.handle.inner(), &session.name, &opener)
        .await
        .map_err(|e| e.to_string())?;
    let owner_route = case
        .subject
        .session_route(case.handle.inner(), &session.name, &owner)
        .await
        .map_err(|e| e.to_string())?;
    let members = json!({"senders":[PlanSender::new(&opener.principal).with_idle_start()]})
        .as_object()
        .unwrap()
        .clone();
    let Some(fields) = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
    else {
        return Ok(Ending::Inapplicable(
            "adapter cannot supply a flow-session plan with senders".into(),
        ));
    };
    let mut plan = fields
        .get("plan")
        .and_then(Value::as_object)
        .cloned()
        .ok_or("flow fixture supplied no plan object")?;
    send_and_wait(
        case,
        &mut session,
        Some(&fields),
        "open the flow session",
        &Map::new(),
    )
    .await?;
    let before_head = head(&owner_route).await?;
    let before_page = read(&owner_route, &ReadRequest::range(0)).await?;
    let before_baseline = expect_ok(
        "session.baseline",
        call(&owner_route, ops::SESSION_BASELINE, json!({})).await?,
    )?;
    let other = if opener.principal == owner.principal {
        "other-sender"
    } else {
        &owner.principal
    };
    plan.insert(
        "senders".into(),
        json!([PlanSender::new(other).with_idle_start()]),
    );
    let reply = if name == "frozen_plan_senders_refresh_refused" {
        call(
            &owner_route,
            ops::SESSION_REFRESH,
            json!({"plan":plan, "policy":rungs::IMMEDIATELY, "generation":1}),
        )
        .await?
    } else {
        let mut params = case.subject.send_fields(&session.name, false);
        params.extend(json!({"prompt":"cannot widen the frozen sender", "send_id":case.mint.next("frozen-send"), "delivery":"queue", "plan":plan}).as_object().unwrap().clone());
        call(&session.route, ops::SESSION_SEND, Value::Object(params)).await?
    };
    expect_field(
        name,
        &expect_error(name, reply)?,
        errors::INVALID_PARAMS,
        "plan.senders",
    )?;
    if head(&owner_route).await? != before_head
        || read(&owner_route, &ReadRequest::range(0)).await? != before_page
    {
        return Err("refused frozen-sender plan changed head or history".into());
    }
    if expect_ok(
        "session.baseline",
        call(&owner_route, ops::SESSION_BASELINE, json!({})).await?,
    )? != before_baseline
    {
        return Err("refused frozen-sender plan changed the baseline".into());
    }
    Ok(Ending::Passed)
}
