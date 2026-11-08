//! Common route discovery and scripted send helpers.

use crate::{
    cases::{Case, Session},
    drive::{call, decode, expect_ok, head, wait_run_end},
    route::RunnerRoute,
    subject::LlmRunnerSubject,
};
use cortexkit_role_harness::RouteStamp;
use cortexkit_role_llm_runner::{describe::RoleDescribe, ops, send::SendReply};
use serde_json::{json, Map, Value};
type Check = Result<(), String>;

/// Plans name bare modules; route stamps attest the corresponding principals.
pub(crate) fn stamp_module<'a>(case: &str, stamp: &'a RouteStamp) -> Result<&'a str, String> {
    stamp.principal.strip_prefix("reserved:").ok_or_else(|| {
        format!(
            "{case}: fixture route principal {:?} lacks the reserved: prefix",
            stamp.principal
        )
    })
}

/// Applicability follows the plan the kit sends, not numbers reported by the runner.
/// Without a compaction item there is no compaction A3 numbering to test.
pub(crate) fn has_compaction_item(fields: &Map<String, Value>) -> bool {
    fields
        .get("plan")
        .and_then(|plan| plan.get("compaction_item"))
        .is_some_and(|item| !item.is_null())
}

pub(crate) async fn require_declared<R: RunnerRoute>(route: &R, group: &str, op: &str) -> Check {
    let describe: RoleDescribe = decode(
        "role.describe",
        expect_ok(
            "role.describe",
            call(route, ops::ROLE_DESCRIBE, json!({})).await?,
        )?,
    )?;
    if describe.declares(group) {
        Ok(())
    } else {
        Err(format!(
            "role.describe does not declare {group}, so the suite does not send {op}"
        ))
    }
}

/// The params of a send into `session`: the subject's fields (or `fields`,
/// when given, for the first send), then the role's, then `extra`.
pub(crate) fn params<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    fields: Option<&Map<String, Value>>,
    prompt: &str,
    send_id: &str,
    extra: &Map<String, Value>,
) -> Value {
    let first = session.first.is_none();
    let mut params = match fields {
        Some(fields) if first => fields.clone(),
        _ => case.subject.send_fields(&session.name, first),
    };
    params.insert("prompt".into(), json!(prompt));
    params.insert("send_id".into(), json!(send_id));
    params.extend(extra.clone());
    Value::Object(params)
}

/// Send `prompt` (with `extra` beside the role's members) and answer the
/// send's reply, without waiting for the run.
pub(crate) async fn send<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    fields: Option<&Map<String, Value>>,
    prompt: &str,
    extra: &Map<String, Value>,
) -> Result<SendReply, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let send_id = case.mint.next("send");
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(
                &session.route,
                ops::SESSION_SEND,
                params(case, session, fields, prompt, &send_id, extra),
            )
            .await?,
        )?,
    )?;
    if session.first.is_none() {
        session.first = Some((send_id, prompt.to_owned()));
    }
    Ok(reply)
}

/// Send `prompt` (with `extra` beside the role's members) and wait for the
/// run it starts to end. Answers the run's id.
pub(crate) async fn send_and_wait<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    fields: Option<&Map<String, Value>>,
    prompt: &str,
    extra: &Map<String, Value>,
) -> Result<String, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let before = head(&session.route)
        .await?
        .last_run_state
        .map(|last| last.run_id);
    let reply = send(case, session, fields, prompt, extra).await?;
    let end = wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    Ok(end.run_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(principal: &str) -> RouteStamp {
        RouteStamp {
            principal: principal.into(),
            scope: None,
        }
    }

    #[test]
    fn stamp_module_strips_exactly_one_reserved_prefix() {
        assert_eq!(
            stamp_module("sender-case", &stamp("reserved:owner")),
            Ok("owner")
        );
        assert_eq!(
            stamp_module("sender-case", &stamp("reserved:reserved:owner")),
            Ok("reserved:owner")
        );
    }

    #[test]
    fn stamp_module_missing_reserved_prefix_is_a_case_named_fixture_error() {
        for principal in ["owner", "other:owner", "prefix:reserved:owner"] {
            let error = stamp_module("sender-case", &stamp(principal)).unwrap_err();
            assert!(error.contains("sender-case"), "{error}");
            assert!(error.contains("fixture"), "{error}");
            assert!(error.contains(principal), "{error}");
        }
    }
}
