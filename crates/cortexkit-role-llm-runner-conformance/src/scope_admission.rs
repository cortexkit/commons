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
