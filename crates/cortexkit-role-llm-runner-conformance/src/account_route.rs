//! Run-fixed account routing names, observed at the real routing peer.

use cortexkit_role_llm_runner::{capabilities, errors, ops, read::ReadRequest, send::AccountRoute};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, expect_error, expect_field, head, read},
    send_helpers::{send, send_and_wait},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RecordingRoutingPeer, RunnerRoute,
};

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    if !describe.declares(capabilities::ACCOUNT_ROUTE) {
        return Ok(Ending::Inapplicable(
            "account_route is not declared on the wire".into(),
        ));
    }
    let methods = describe
        .routed_accounts
        .ok_or("account_route requires routed_accounts")?;
    for method in methods {
        let fixture = case
            .subject
            .routed_account_fixture(&method)
            .await
            .map_err(|error| format!("routed_account_fixture({method}): {error}"))?;
        let provider = fixture
            .get("provider")
            .and_then(Value::as_str)
            .ok_or("routed_account_fixture requires provider")?;
        let names_case = name == "account_route_names_the_route";
        let route = if names_case {
            AccountRoute::new(
                format!("kit-pool-{provider}"),
                format!("kit-pool-{}", case.mint.next("model")),
            )
        } else {
            AccountRoute::new(provider, "kit-pool-fixed-model")
        };
        let peer = RecordingRoutingPeer::new(&method, fixture).map_err(|e| e.to_string())?;
        let peer = if names_case {
            peer.with_account_route(route.clone())
        } else {
            peer
        };
        let mut session = case
            .open(
                name,
                Script {
                    turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text("answer".into())])],
                },
            )
            .await?;
        case.subject
            .install_routing_peer(&session.name, peer.clone())
            .await
            .map_err(|error| format!("install_routing_peer({method}): {error}"))?;
        let extra = json!({"account_route": route}).as_object().unwrap().clone();
        match name {
            "account_route_names_the_route" => {
                let run_id =
                    send_and_wait(case, &mut session, None, "answer briefly", &extra).await?;
                let admissions: Vec<_> = peer
                    .calls()
                    .into_iter()
                    .filter(|call| call.method == "route.account_for")
                    .collect();
                if admissions.is_empty() {
                    return Err(format!(
                        "listed method {method} sent no account_route admission"
                    ));
                }
                for call in admissions {
                    if call.body.get("provider") != Some(&json!(route.provider))
                        || call.body.get("model") != Some(&json!(route.model))
                    {
                        return Err(format!(
                            "account_route was not forwarded verbatim for {method}: {}",
                            call.body
                        ));
                    }
                }
                let head = head(&session.route).await?;
                if let Some(model) = head.model {
                    if model.provider == route.provider || model.model == route.model {
                        return Err("account_route fixture did not keep route names distinct from vendor names".into());
                    }
                }
                if !head
                    .last_run_state
                    .is_some_and(|last| last.run_id == run_id && last.state == "completed")
                {
                    return Err(format!("account_route run for {method} did not complete"));
                }
                if case
                    .subject
                    .model_invocations(&session.name)
                    .await
                    .map_err(|e| e.to_string())?
                    == 0
                {
                    return Err(format!(
                        "account_route run for {method} never reached the scripted model"
                    ));
                }
            }
            "account_route_fixed_per_run" => {
                case.subject
                    .hold_next_model_step(&session.name)
                    .await
                    .map_err(|e| e.to_string())?;
                send(case, &mut session, None, "start", &extra).await?;
                case.subject
                    .await_model_step(&session.name)
                    .await
                    .map_err(|e| e.to_string())?;
                let result = async {
                    let before = serde_json::to_value(
                        read(&session.route, &ReadRequest::tail()).await?.messages,
                    )
                    .map_err(|e| e.to_string())?;
                    for different in [
                        AccountRoute::new(format!("{}-different", route.provider), &route.model),
                        AccountRoute::new(&route.provider, format!("{}-different", route.model)),
                    ] {
                        let mut joining = case.params(
                            &session,
                            "join",
                            &case.mint.next("join-send"),
                            Some("steer"),
                        );
                        joining["account_route"] = json!(different);
                        let reply = call(&session.route, ops::SESSION_SEND, joining).await?;
                        let after = serde_json::to_value(
                            read(&session.route, &ReadRequest::tail()).await?.messages,
                        )
                        .map_err(|e| e.to_string())?;
                        if before != after {
                            return Err(
                                "joining send with a different account_route wrote a message"
                                    .into(),
                            );
                        }
                        let error = expect_error("joining account_route send", reply)?;
                        expect_field(
                            "joining account_route send",
                            &error,
                            errors::INVALID_PARAMS,
                            "account_route",
                        )?;
                    }
                    Ok::<(), String>(())
                }
                .await;
                let release = case
                    .subject
                    .release_model_step(&session.name)
                    .await
                    .map_err(|e| e.to_string());
                result?;
                release?;
            }
            _ => return Err(format!("unknown account-route case {name}")),
        }
    }
    Ok(Ending::Passed)
}
