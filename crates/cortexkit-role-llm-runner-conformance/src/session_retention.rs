//! Policy checks observe accepted state and deletion over the management route.

use crate::{
    cases::{Case, Ending, Session},
    drive::{decode, expect_code, expect_error, expect_field, expect_ok, wait_run_end, Mint},
    route::{Reply, RunnerRoute},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use cortexkit_role_harness::{CrashDriver, KillPoint, Trigger};
use cortexkit_role_llm_runner::{describe::check_describe, errors, ops, points, send::SendReply};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};

fn policy(keep: Value, dumps: bool) -> Value {
    json!({"keep":keep,"archive_after_days":0,"backup":false,"request_dumps":dumps})
}

async fn raw<R: RunnerRoute>(r: &R, method: &str, params: Value) -> Result<Reply, String> {
    r.request(method, params).await.map_err(|e| e.message)
}
async fn ok<R: RunnerRoute>(r: &R, method: &str, params: Value) -> Result<Value, String> {
    expect_ok(method, raw(r, method, params).await?)
}
async fn code<R: RunnerRoute>(
    r: &R,
    method: &str,
    params: Value,
    expected: &str,
) -> Result<(), String> {
    expect_code(
        method,
        &expect_error(method, raw(r, method, params).await?)?,
        expected,
    )
}
fn check(test: bool, message: &str) -> Result<(), String> {
    if test {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn release<R: RunnerRoute>(r: &R, expected: &str) -> Result<(), String> {
    let answer = ok(r, ops::SESSION_RELEASE, json!({})).await?;
    check(
        answer == json!({"state":expected}),
        &format!("release expected {expected}, got {answer}"),
    )
}
async fn state<R: RunnerRoute>(r: &R, expected: &Value, source: &str) -> Result<Value, String> {
    let head = ok(r, ops::SESSION_HEAD, json!({})).await?;
    check(
        head["retention_policy"] == *expected,
        &format!("head rewrote or ignored retention_policy: {head}"),
    )?;
    check(
        head["retention_state"]["source"] == source,
        "head has wrong retention source",
    )?;
    Ok(head)
}

async fn open<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Result<Session<S::Route>, String>
where
    S::Route: RunnerRoute,
{
    case.open(
        "session-retention",
        Script {
            turns: (0..12)
                .map(|_| ScriptedTurn::new(vec![ScriptedPart::Text("policy-answer".into())]))
                .collect(),
        },
    )
    .await
}

async fn send<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    s: &mut Session<S::Route>,
    p: Option<Value>,
    settle: bool,
) -> Result<SendReply, String>
where
    S::Route: RunnerRoute,
{
    let id = case.mint.next("policy-send");
    let prompt = case.mint.next("policy-prompt");
    let mut params = case.params(s, &prompt, &id, None);
    params.as_object_mut().unwrap().remove("retention");
    params.as_object_mut().unwrap().remove("retention_policy");
    if let Some(p) = p {
        params["retention_policy"] = p;
    }
    let reply: SendReply = decode(
        "policy send",
        ok(&s.route, ops::SESSION_SEND, params).await?,
    )?;
    if s.first.is_none() {
        s.first = Some((id, prompt));
    }
    if settle {
        wait_run_end(case.subject, &s.route, case.declared, None, &reply).await?;
    }
    Ok(reply)
}

async fn gone<R: RunnerRoute>(r: &R, checksum: Option<&str>) -> Result<(), String> {
    code(r, ops::SESSION_HEAD, json!({}), errors::UNKNOWN_SESSION).await?;
    code(r, ops::SESSION_READ, json!({}), errors::UNKNOWN_SESSION).await?;
    code(
        r,
        ops::SESSION_READ,
        json!({"from_ordinal":0}),
        errors::UNKNOWN_SESSION,
    )
    .await?;
    if let Some(hash) = checksum {
        code(
            r,
            ops::SESSION_MEDIA_READ,
            json!({"sha256":hash,"offset":0,"length":1}),
            errors::UNKNOWN_MEDIA,
        )
        .await?;
    }
    Ok(())
}

async fn wait_gone<S: LlmRunnerSubject>(subject: &S, route: &S::Route) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    for _ in 0..crate::drive::MAX_POLLS {
        if matches!(raw(route, ops::SESSION_HEAD, json!({})).await?, Reply::Error(e) if e.code == errors::UNKNOWN_SESSION)
        {
            return Ok(());
        }
        subject.pause().await;
    }
    Err("recorded release did not delete the session after the run and queue ended".into())
}

async fn wait_models<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
    count: usize,
) -> Result<(), String> {
    for _ in 0..crate::drive::MAX_POLLS {
        if subject
            .model_invocations(session)
            .await
            .map_err(|e| e.to_string())?
            >= count
        {
            return Ok(());
        }
        subject.pause().await;
    }
    Err(format!(
        "queued send was never delivered to the model ({count} invocations required)"
    ))
}

// A keep-none session may disappear between its terminal record and the first
// read. Either observation ends the wait without requiring a surviving terminal.
async fn wait_end_or_gone<S: LlmRunnerSubject>(
    subject: &S,
    route: &S::Route,
) -> Result<bool, String>
where
    S::Route: RunnerRoute,
{
    for _ in 0..crate::drive::MAX_POLLS {
        match raw(route, ops::SESSION_HEAD, json!({})).await? {
            Reply::Error(e) if e.code == errors::UNKNOWN_SESSION => return Ok(true),
            Reply::Error(e) => return Err(format!("waiting for run end: {}", e.code)),
            Reply::Response(head) => {
                if head["last_run_state"]["state"].as_str().is_some_and(|s| {
                    cortexkit_role_llm_runner::run::RunState::parse(s).is_terminal()
                }) {
                    return Ok(false);
                }
            }
        }
        subject.pause().await;
    }
    Err("run did not end before the retention wait bound".into())
}

/// Legacy lifetime scenarios need an admissible retention-only first send.
pub(crate) async fn inapplicable<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Option<String>, String>
where
    S::Route: RunnerRoute,
{
    if (name.starts_with("retention_") || name == "crash_at_RetentionTombstoned")
        && case.declared.contains(&Capability::Retention)
        && case.declared.contains(&Capability::SessionRetention)
    {
        let s = open(case).await?;
        let described = check_describe(&ok(&s.route, ops::ROLE_DESCRIBE, json!({})).await?)
            .map_err(|e| format!("describe: {e:?}"))?;
        if described.declares("retention")
            && described.declares("session_retention")
            && described.session_retention.is_some_and(|l| l.required)
        {
            return Ok(Some(
                "session_retention required: legacy retention-only first sends are refused".into(),
            ));
        }
    }
    crate::delivery_order::inapplicable(case, name).await
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut s = open(case).await?;
    let described = check_describe(&ok(&s.route, ops::ROLE_DESCRIBE, json!({})).await?)
        .map_err(|e| format!("describe: {e:?}"))?;
    check(
        described.declares("session_retention"),
        "adapter declares session_retention but runner does not",
    )?;
    let limits = described
        .session_retention
        .ok_or("missing session_retention limits")?;
    let p = policy(json!("forever"), false);
    match name {
        "session_retention_honoured" => {
            let p = policy(json!("none"), false);
            case.subject
                .hold_next_model_step(&s.name)
                .await
                .map_err(|e| e.to_string())?;
            send(case, &mut s, Some(p.clone()), false).await?;
            case.subject
                .await_model_step(&s.name)
                .await
                .map_err(|e| e.to_string())?;
            state(&s.route, &p, "starter").await?;
            case.subject
                .release_model_step(&s.name)
                .await
                .map_err(|e| e.to_string())?;
            let already_gone = wait_end_or_gone(case.subject, &s.route).await?;
            let ms = limits.max_unreleased_hold_ms.saturating_add(1);
            if already_gone {
                gone(&s.route, None).await?;
                return Ok(Ending::Passed);
            } else if case.declared.contains(&Capability::RetentionClock) {
                case.subject
                    .advance_retention_clock(&s.name, ms)
                    .await
                    .map_err(|e| e.to_string())?;
            } else if ms <= 30_000 {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            } else {
                return Ok(Ending::Skipped {
                    missing: vec![Capability::RetentionClock],
                    reason: "fallback hold exceeds the suite's 30-second real wait bound".into(),
                });
            }
            gone(&s.route, None).await?;
        }
        "session_retention_days_and_archive" => {
            let mut p = policy(json!({"days":limits.max_days.min(2)}), false);
            p["archive_after_days"] = json!(if limits.max_days >= 2 { 1 } else { 0 });
            send(case, &mut s, Some(p.clone()), true).await?;
            let before = ok(&s.route, ops::SESSION_READ, json!({"from_ordinal":0})).await?;
            if limits.max_days >= 2 {
                case.subject
                    .advance_retention_clock(&s.name, 86_400_000)
                    .await
                    .map_err(|e| e.to_string())?;
                check(
                    ok(&s.route, ops::SESSION_READ, json!({"from_ordinal":0})).await? == before,
                    "archive changed read results",
                )?;
            }
            case.subject
                .advance_retention_clock(&s.name, 86_400_001)
                .await
                .map_err(|e| e.to_string())?;
            gone(&s.route, None).await?;
        }
        "session_retention_required" => {
            let mut params = case.params(&s, "first-without-policy", &case.mint.next("send"), None);
            params.as_object_mut().unwrap().remove("retention_policy");
            if limits.required {
                code(
                    &s.route,
                    ops::SESSION_SEND,
                    params,
                    errors::RETENTION_REQUIRED,
                )
                .await?;
                // The same name must still admit its first policy, not conflict
                // with a record written by the refused send.
                send(case, &mut s, Some(p.clone()), true).await?;
                state(&s.route, &p, "starter").await?;
            } else {
                ok(&s.route, ops::SESSION_SEND, params).await?;
            }
        }
        "session_retention_legacy_required" => {
            if !described.declares("retention") || !limits.required {
                return Ok(Ending::Inapplicable(
                    "requires both retention groups and session_retention required:true".into(),
                ));
            }
            let mut params = case.params(&s, "legacy-only", &case.mint.next("send"), None);
            params.as_object_mut().unwrap().remove("retention_policy");
            params["retention"] = json!(1);
            code(
                &s.route,
                ops::SESSION_SEND,
                params,
                errors::RETENTION_REQUIRED,
            )
            .await?;
            send(case, &mut s, Some(p.clone()), true).await?;
            state(&s.route, &p, "starter").await?;
        }
        "session_retention_conflict" => {
            send(case, &mut s, Some(p.clone()), true).await?;
            let mut params = case.params(&s, "conflict", &case.mint.next("send"), None);
            params["retention_policy"] = policy(json!({"days":1}), false);
            let e = expect_error("conflict", raw(&s.route, ops::SESSION_SEND, params).await?)?;
            expect_code("conflict", &e, errors::RETENTION_CONFLICT)?;
            check(
                e.detail.as_ref().and_then(|d| d.get("frozen")) == Some(&p),
                "conflict lacks frozen policy",
            )?;
            send(case, &mut s, Some(p.clone()), true).await?;
            state(&s.route, &p, "starter").await?;
        }
        "session_retention_owner_can_release" | "session_retention_parent_cannot_release" => {
            let Some((starter, parent)) = case.subject.retention_scope_stamps() else {
                return Ok(Ending::Inapplicable(
                    "adapter cannot open cross-principal scoped routes".into(),
                ));
            };
            check(
                starter.principal != parent.principal
                    && starter.scope == parent.scope
                    && starter
                        .scope
                        .as_ref()
                        .is_some_and(|scope| scope.owner == parent.principal),
                "scope fixture must separate starter and parent owner",
            )?;
            s.route = case
                .subject
                .session_route(case.handle.inner(), &s.name, &starter)
                .await
                .map_err(|e| e.to_string())?;
            send(case, &mut s, Some(p.clone()), true).await?;
            if name == "session_retention_parent_cannot_release" {
                let other = case
                    .subject
                    .session_route(case.handle.inner(), &s.name, &parent)
                    .await
                    .map_err(|e| e.to_string())?;
                code(
                    &other,
                    ops::SESSION_RELEASE,
                    json!({}),
                    errors::SCOPE_OWNER_MISMATCH,
                )
                .await?;
                state(&s.route, &p, "starter").await?;
            } else {
                release(&s.route, "deleted").await?;
            }
        }
        "session_retention_release_idempotent" | "session_retention_deletion_observable" => {
            send(case, &mut s, Some(p.clone()), true).await?;
            let before = state(&s.route, &p, "starter").await?;
            release(&s.route, "deleted").await?;
            if name == "session_retention_release_idempotent" {
                release(&s.route, "absent").await?;
                for keep in [json!("none"), json!({"days":1})] {
                    let mut other = open(case).await?;
                    let no_keep = keep == "none";
                    if no_keep {
                        case.subject
                            .hold_next_model_step(&other.name)
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                    send(case, &mut other, Some(policy(keep, false)), !no_keep).await?;
                    if no_keep {
                        case.subject
                            .await_model_step(&other.name)
                            .await
                            .map_err(|e| e.to_string())?;
                        release(&other.route, "pending").await?;
                        case.subject
                            .release_model_step(&other.name)
                            .await
                            .map_err(|e| e.to_string())?;
                        wait_gone(case.subject, &other.route).await?;
                    } else {
                        release(&other.route, "deleted").await?;
                    }
                    release(&other.route, "absent").await?;
                }
            } else {
                gone(&s.route, None).await?;
                let old_prompt = s.first.as_ref().unwrap().1.clone();
                s.first = None;
                send(case, &mut s, Some(policy(json!({"days":1}), false)), true).await?;
                let after = ok(&s.route, ops::SESSION_HEAD, json!({})).await?;
                check(
                    before["incarnation"].is_string()
                        && before["incarnation"] != after["incarnation"],
                    "recreated session reused or omitted incarnation",
                )?;
                let page = ok(&s.route, ops::SESSION_READ, json!({"from_ordinal":0})).await?;
                check(
                    !page.to_string().contains(&old_prompt),
                    "new incarnation replays deleted history",
                )?;
            }
        }
        "session_retention_backup_none_refused" => {
            let mut params = case.params(&s, "invalid-backup", &case.mint.next("send"), None);
            let mut bad = policy(json!("none"), false);
            bad["backup"] = json!(true);
            params["retention_policy"] = bad;
            let e = expect_error(
                "backup none",
                raw(&s.route, ops::SESSION_SEND, params).await?,
            )?;
            expect_field(
                "backup none",
                &e,
                errors::INVALID_PARAMS,
                "retention_policy.backup",
            )?;
            send(case, &mut s, Some(p.clone()), true).await?;
            state(&s.route, &p, "starter").await?;
        }
        "session_retention_dumps_preserved" => {
            let dumps = policy(json!("forever"), true);
            send(case, &mut s, Some(dumps.clone()), true).await?;
            let head = state(&s.route, &dumps, "starter").await?;
            if case.subject.request_dumps_unsupported() {
                check(
                    head["retention_state"]["request_dumps_unsupported"] == true,
                    "missing request_dumps_unsupported flag",
                )?;
            }
        }
        "session_retention_runner_replaced_once" => {
            if !case
                .subject
                .prepare_runner_retention(case.handle.inner(), &s.name)
                .await
                .map_err(|e| e.to_string())?
            {
                return Ok(Ending::Inapplicable(
                    "runner has no hosted or pre-existing session fixture".into(),
                ));
            }
            let original = ok(&s.route, ops::SESSION_HEAD, json!({})).await?;
            check(
                original["retention_state"]["source"] == "runner"
                    && original["retention_policy"].is_object(),
                "fixture has no runner-assigned policy",
            )?;
            let replacement = if original["retention_policy"] == p {
                policy(json!({"days":1}), false)
            } else {
                p.clone()
            };
            send(case, &mut s, Some(replacement.clone()), true).await?;
            state(&s.route, &replacement, "starter").await?;
            let mut params = case.params(&s, "second replacement", &case.mint.next("send"), None);
            params["retention_policy"] = original["retention_policy"].clone();
            code(
                &s.route,
                ops::SESSION_SEND,
                params,
                errors::RETENTION_CONFLICT,
            )
            .await?;
            state(&s.route, &replacement, "starter").await?;
        }
        "session_retention_adopt_policy" => {
            let plan = case
                .subject
                .host_plan(&s.name, false)
                .ok_or("adopt requires a host plan")?;
            case.subject
                .start_host_session(case.handle.inner(), &s.name, &plan, None)
                .await
                .map_err(|e| e.to_string())?;
            let original = ok(&s.route, ops::SESSION_HEAD, json!({})).await?;
            let omitted = json!({"send_id":case.mint.next("adopt"),"plan":plan});
            if limits.required {
                code(
                    &s.route,
                    ops::SESSION_ADOPT,
                    omitted,
                    errors::RETENTION_REQUIRED,
                )
                .await?;
                check(
                    ok(&s.route, ops::SESSION_HEAD, json!({})).await? == original,
                    "refused adopt wrote state",
                )?;
            } else {
                ok(&s.route, ops::SESSION_ADOPT, omitted).await?;
            }
            ok(
                &s.route,
                ops::SESSION_ADOPT,
                json!({"send_id":case.mint.next("adopt"),"plan":plan,"retention_policy":p}),
            )
            .await?;
            state(&s.route, &p, "starter").await?;
            ok(
                &s.route,
                ops::SESSION_ADOPT,
                json!({"send_id":case.mint.next("adopt"),"plan":plan,"retention_policy":p}),
            )
            .await?;
            let other = policy(json!({"days":1}), false);
            let e = expect_error(
                "adopt conflict",
                raw(
                    &s.route,
                    ops::SESSION_ADOPT,
                    json!({"send_id":case.mint.next("adopt"),"plan":plan,"retention_policy":other}),
                )
                .await?,
            )?;
            expect_code("adopt conflict", &e, errors::RETENTION_CONFLICT)?;
            check(
                e.detail.as_ref().and_then(|d| d.get("frozen")) == Some(&p),
                "adopt conflict lacks frozen policy",
            )?;
            release(&s.route, "deleted").await?;
        }
        "session_retention_active_never_deleted" => {
            for cause in ["days", "release", "fallback"] {
                let mut s = open(case).await?;
                let keep = match cause {
                    "days" => json!({"days":1}),
                    "fallback" => json!("none"),
                    _ => json!("forever"),
                };
                let p = policy(keep, false);
                case.subject
                    .hold_next_model_step(&s.name)
                    .await
                    .map_err(|e| e.to_string())?;
                send(case, &mut s, Some(p.clone()), false).await?;
                case.subject
                    .await_model_step(&s.name)
                    .await
                    .map_err(|e| e.to_string())?;
                // Queue a second accepted send behind the active run as well.
                let queued = send(case, &mut s, Some(p.clone()), false).await?;
                check(
                    queued.state == "pending" && queued.submission_id.is_some(),
                    "second send did not queue behind the held run",
                )?;
                if cause == "release" {
                    release(&s.route, "pending").await?;
                    release(&s.route, "pending").await?;
                    let mut params = case.params(&s, "released", &case.mint.next("send"), None);
                    params["retention_policy"] = p.clone();
                    code(
                        &s.route,
                        ops::SESSION_SEND,
                        params,
                        errors::SESSION_RELEASED,
                    )
                    .await?;
                } else {
                    let ms = if cause == "days" {
                        86_400_001
                    } else {
                        limits.max_unreleased_hold_ms.saturating_add(1)
                    };
                    case.subject
                        .advance_retention_clock(&s.name, ms)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                state(&s.route, &p, "starter").await?;
                case.subject
                    .release_model_step(&s.name)
                    .await
                    .map_err(|e| e.to_string())?;
                wait_models(case.subject, &s.name, 2).await?;
                if cause == "release" {
                    wait_gone(case.subject, &s.route).await?;
                } else if cause == "days" {
                    check(
                        !wait_end_or_gone(case.subject, &s.route).await?,
                        "days retention deleted immediately after run end",
                    )?;
                    state(&s.route, &p, "starter").await?;
                    case.subject
                        .advance_retention_clock(&s.name, 43_200_000)
                        .await
                        .map_err(|e| e.to_string())?;
                    state(&s.route, &p, "starter").await?;
                    case.subject
                        .advance_retention_clock(&s.name, 43_200_001)
                        .await
                        .map_err(|e| e.to_string())?;
                    gone(&s.route, None).await?;
                } else {
                    // Very short fallback holds may elapse before a read reaches
                    // the runner. The held run above, not a post-run read, proves deferral.
                    if !wait_end_or_gone(case.subject, &s.route).await? {
                        case.subject
                            .advance_retention_clock(
                                &s.name,
                                limits.max_unreleased_hold_ms.saturating_add(1),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                    gone(&s.route, None).await?;
                }
            }
        }
        "session_retention_media_deleted" => {
            let hash = seed_media(case, &mut s, &p).await?;
            release(&s.route, "deleted").await?;
            gone(&s.route, Some(&hash)).await?;
        }
        "session_retention_queued_never_deleted" => {
            for cause in ["days", "release", "fallback"] {
                let mut s = open(case).await?;
                if !case
                    .subject
                    .hold_retention_queue(&s.name)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    return Ok(Ending::Inapplicable(
                        "adapter cannot hold a queued send with no active run".into(),
                    ));
                }
                let p = policy(
                    match cause {
                        "days" => json!({"days":1}),
                        "fallback" => json!("none"),
                        _ => json!("forever"),
                    },
                    false,
                );
                let reply = send(case, &mut s, Some(p.clone()), false).await?;
                check(
                    reply.state == "pending" && reply.submission_id.is_some(),
                    "queue fixture did not produce a durably pending send",
                )?;
                check(
                    case.subject
                        .model_invocations(&s.name)
                        .await
                        .map_err(|e| e.to_string())?
                        == 0,
                    "queued fixture already started a model request",
                )?;
                if cause == "release" {
                    release(&s.route, "pending").await?;
                } else {
                    let ms = if cause == "days" {
                        86_400_001
                    } else {
                        limits.max_unreleased_hold_ms.saturating_add(1)
                    };
                    case.subject
                        .advance_retention_clock(&s.name, ms)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                state(&s.route, &p, "starter").await?;
                case.subject
                    .release_retention_queue(&s.name)
                    .await
                    .map_err(|e| e.to_string())?;
                wait_models(case.subject, &s.name, 1).await?;
                if cause == "release" {
                    wait_gone(case.subject, &s.route).await?;
                }
            }
        }
        _ => return Err(format!("unknown session retention case {name}")),
    }
    Ok(Ending::Passed)
}

async fn seed_media<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    s: &mut Session<S::Route>,
    p: &Value,
) -> Result<String, String>
where
    S::Route: RunnerRoute,
{
    use base64::{engine::general_purpose::STANDARD, Engine};
    use cortexkit_role_llm_runner::content_digest::sha256_hex;
    send(case, s, Some(p.clone()), true).await?;
    let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=").unwrap();
    let hash = sha256_hex(&bytes);
    let begin = ok(
        &s.route,
        ops::SESSION_UPLOAD_BEGIN,
        json!({"mime":"image/png","bytes":bytes.len(),"sha256":hash}),
    )
    .await?;
    let id = begin["upload_id"].as_str().ok_or("upload has no id")?;
    ok(
        &s.route,
        ops::SESSION_UPLOAD_APPEND,
        json!({"upload_id":id,"offset":0,"data":STANDARD.encode(&bytes)}),
    )
    .await?;
    ok(
        &s.route,
        ops::SESSION_UPLOAD_COMMIT,
        json!({"upload_id":id}),
    )
    .await?;
    let mut params = case.params(s, "media prompt", &case.mint.next("send"), None);
    params["retention_policy"] = p.clone();
    params["content"] = json!([{"type":"upload","upload_id":id}]);
    if let Some(fields) = case.subject.attachment_model_fields(&s.name, true) {
        params.as_object_mut().unwrap().extend(fields);
    }
    let sent: SendReply = decode("media send", ok(&s.route, ops::SESSION_SEND, params).await?)?;
    wait_run_end(case.subject, &s.route, case.declared, None, &sent).await?;
    let read = ok(
        &s.route,
        ops::SESSION_MEDIA_READ,
        json!({"sha256":hash,"offset":0,"length":1}),
    )
    .await?;
    check(
        read["data"] == STANDARD.encode(&bytes[..1]),
        "checksum was not readable before deletion",
    )?;
    Ok(hash)
}

/// Kill after the pending reply, not merely after an in-memory release flag.
pub(crate) async fn crash<S: LlmRunnerSubject>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let root = work_dir.join("crash-ReleasePending");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let handle = driver.spawn(&root).await.map_err(|e| e.to_string())?;
    let case = Case {
        subject,
        handle: &handle,
        declared,
        mint,
    };
    let mut s = open(&case).await?;
    let p = policy(json!("forever"), false);
    let hash = seed_media(&case, &mut s, &p).await?;
    subject
        .hold_next_model_step(&s.name)
        .await
        .map_err(|e| e.to_string())?;
    send(&case, &mut s, Some(p.clone()), false).await?;
    subject
        .await_model_step(&s.name)
        .await
        .map_err(|e| e.to_string())?;
    let before_queue = subject
        .model_invocations(&s.name)
        .await
        .map_err(|e| e.to_string())?;
    let queued = send(&case, &mut s, Some(p.clone()), false).await?;
    check(
        queued.state == "pending" && queued.submission_id.is_some(),
        "crash scenario has no queued send",
    )?;
    let pending = std::sync::Mutex::new(None);
    let trigger: Trigger<'_> = Box::pin(async {
        let reply = release(&s.route, "pending").await;
        *pending.lock().unwrap() = Some(reply);
    });
    driver
        .kill_at(handle, &KillPoint::new(points::RELEASE_PENDING), trigger)
        .await
        .map_err(|e| e.to_string())?;
    pending
        .into_inner()
        .unwrap()
        .ok_or("runner killed before pending reply was observed")??;
    let kill = driver
        .ledger()
        .kills()
        .last()
        .ok_or("release made no kill")?;
    check(
        kill.mechanism.is_real_process_kill(),
        "ReleasePending requires a real process kill after the pending reply",
    )?;
    let handle = driver.restart(&root).await.map_err(|e| e.to_string())?;
    subject
        .release_model_step(&s.name)
        .await
        .map_err(|e| e.to_string())?;
    wait_models(subject, &s.name, before_queue + 1).await?;
    let route = subject
        .session_route(handle.inner(), &s.name, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    // Recovery is autonomous: reads must become unavailable without a second
    // release request.
    wait_gone(subject, &route).await?;
    gone(&route, Some(&hash)).await?;
    release(&route, "absent").await?;
    release(&route, "absent").await?;
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests;
