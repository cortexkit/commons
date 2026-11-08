//! Optional host-start probes use the live extension advertisement, not a group.

use cortexkit_role_llm_runner::{
    errors,
    host_start::{advertises_host_start, HostStartReply, HostStartRequest},
    ops,
    read::HeadMeta,
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_field, expect_ok},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script},
};

async fn start<R: RunnerRoute>(route: &R, request: &Value) -> Result<Value, String> {
    let raw = expect_ok(
        "harness.host_start",
        call(route, ops::HARNESS_HOST_START, request.clone()).await?,
    )?;
    let reply: HostStartReply = decode("harness.host_start", raw.clone())?;
    if !reply.started || reply.applied_plan_digest.is_empty() {
        return Err(format!(
            "host start did not confirm a recorded plan: {reply:?}"
        ));
    }
    Ok(raw)
}

fn with_alternate_plan<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &str,
    request: &Value,
) -> Result<Value, String> {
    let alternate = case
        .subject
        .host_plan(session, true)
        .ok_or("host_start advertised but adapter supplies no alternate plan")?;
    if json!(alternate) == request["plan"] {
        return Err("host-start alternate plan is identical to the original".into());
    }
    let mut changed = request.clone();
    changed["plan"] = json!(alternate);
    Ok(changed)
}

pub(crate) async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    if !advertises_host_start(&describe) {
        return Ok(Ending::Skipped {
            missing: vec![],
            reason: "role.describe.extensions does not list harness.host_start".into(),
        });
    }
    let stamp = case.subject.owner_stamp();
    if !stamp
        .scope
        .as_ref()
        .is_some_and(|scope| scope.owner == stamp.principal)
    {
        return Err("host_start requires an attested plain-scope owner route".into());
    }
    let session = case.open("host-start", Script::default()).await?;
    let plan = case
        .subject
        .host_plan(&session.name, false)
        .ok_or("host_start advertised but adapter supplies no host plan")?;
    let model = case
        .subject
        .host_start_model(&session.name)
        .ok_or("host_start advertised but adapter supplies no host-start model")?;
    let request = serde_json::to_value(HostStartRequest::new(
        case.mint.next("host-start-id"),
        plan.clone(),
        model,
    ))
    .map_err(|e| e.to_string())?;
    if name == "host_start_invalid_params_leaves_nothing" {
        let mut bad = request.clone();
        bad["extra"] = json!(true);
        let error = expect_error(
            "host start with extra field",
            call(&session.route, ops::HARNESS_HOST_START, bad).await?,
        )?;
        expect_code(
            "host start with extra field",
            &error,
            errors::INVALID_PARAMS,
        )?;
        let head: HeadMeta = decode(
            "head after refused host start",
            expect_ok(
                "head after refused host start: the contract keeps empty success answers for a never-written session",
                call(&session.route, ops::SESSION_HEAD, json!({})).await?,
            )?,
        )?;
        if head.history_version != 0 || head.last_run_state.is_some() {
            return Err(format!(
                "head after refused host start is not empty: history_version={}, last_run_state={:?}",
                head.history_version, head.last_run_state
            ));
        }
        let mut changed = with_alternate_plan(case, &session.name, &request)?;
        changed["send_id"] = json!(case.mint.next("host-start-new-id"));
        start(&session.route, &changed).await?;
        return Ok(Ending::Passed);
    }
    let first = start(&session.route, &request).await?;
    if name == "host_start_exact_repeat_replays" {
        let repeated = start(&session.route, &request).await?;
        if repeated != first {
            return Err(format!(
                "exact host-start repeat changed reply: {first} -> {repeated}"
            ));
        }
        return Ok(Ending::Passed);
    }
    let mut changed = with_alternate_plan(case, &session.name, &request)?;
    match name {
        "host_start_send_id_reuse_names_plan" => {
            let error = expect_error(
                "host start changed plan under same id",
                call(&session.route, ops::HARNESS_HOST_START, changed).await?,
            )?;
            expect_code(
                "host start changed plan under same id",
                &error,
                errors::SEND_ID_REUSE,
            )?;
            expect_field(
                "host start changed plan under same id",
                &error,
                errors::SEND_ID_REUSE,
                "plan",
            )?;
        }
        "host_start_new_id_plan_drift" => {
            changed["send_id"] = json!(case.mint.next("host-start-new-id"));
            let error = expect_error(
                "host start new id changed plan",
                call(&session.route, ops::HARNESS_HOST_START, changed).await?,
            )?;
            expect_code("host start new id changed plan", &error, errors::PLAN_DRIFT)?;
        }
        other => return Err(format!("unknown host_start case {other}")),
    }
    Ok(Ending::Passed)
}
