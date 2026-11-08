//! Opening a session is admission without a message, not host-plan confirmation.

use super::*;
use cortexkit_role_llm_runner::{
    delivery_order::{Mode, PlanSender},
    read::{HeadSender, ReadRequest},
};

pub(super) async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    for group in [groups::ADOPT_OPEN, groups::ADOPT, groups::PLANS] {
        if !describe.declares(group) {
            return Err(format!("role.describe does not declare {group}"));
        }
    }
    let stamp = case.subject.owner_stamp();
    if !stamp
        .scope
        .as_ref()
        .is_some_and(|scope| scope.owner == stamp.principal)
    {
        return Err("adopt_open requires an attested plain-scope owner route".into());
    }
    match name {
        "adopt_open_opens_unknown_session" => opens(case, &describe).await,
        "adopt_open_repeat_replays" => repeats(case).await.map(|()| Ending::Passed),
        "adopt_open_existing_session_unchanged" => existing(case).await.map(|()| Ending::Passed),
        "adopt_open_non_owner_refused" => non_owner(case).await,
        other => Err(format!("unknown adopt_open case {other}")),
    }
}

async fn empty<R: RunnerRoute>(route: &R) -> Check {
    let meta = head(route).await?;
    if meta.last_ordinal.is_some() || meta.last_run_state.is_some() {
        return Err(format!(
            "adopt_open started a run or wrote history: {meta:?}"
        ));
    }
    let page = crate::drive::read(route, &ReadRequest::tail()).await?;
    if !page.messages.is_empty() || page.next_from_ordinal.is_some() {
        return Err(format!(
            "adopt_open did not return an empty read page: {page:?}"
        ));
    }
    if page.history_version != meta.history_version || page.incarnation != meta.incarnation {
        return Err("adopt_open head and empty read disagree on history identity".into());
    }
    Ok(())
}

async fn ready<R: RunnerRoute>(route: &R) -> Check {
    let reply: BaselineReply = decode(
        "session.baseline",
        expect_ok("opened session.baseline", baseline(route).await?)?,
    )?;
    let facts = reply
        .baseline
        .as_ref()
        .ok_or("opened session has no baseline")?;
    if reply.state != baseline_states::READY
        || facts.composition_digest.is_none()
        || !facts.plan_consistent()
        || !facts
            .tool_names
            .iter()
            .any(|name| name == crate::SCRIPTED_TOOL)
    {
        return Err(format!(
            "adopt_open did not apply the plan's composition facts: {reply:?}"
        ));
    }
    Ok(())
}

async fn opens<S>(case: &Case<'_, S>, describe: &RoleDescribe) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case.open("adopt-open", Script::default()).await?;
    let grants =
        describe.declares(groups::HEAD_SENDER) && describe.declares(groups::DELIVERY_ORDER);
    let identities = if grants {
        Some(
            case.subject
                .head_sender_identities()
                .ok_or("head_sender declared but adapter supplies no sender routes")?,
        )
    } else {
        None
    };
    let plan = if let Some([a, b]) = &identities {
        if a.module == b.module || a.stamp == b.stamp {
            return Err("adopt_open sender fixture identities must be distinct".into());
        }
        let mut members = Map::new();
        members.insert(
            "senders".into(),
            json!([
                PlanSender::new(&a.module),
                PlanSender::new(&b.module)
                    .with_idle_start()
                    .with_interrupt(),
            ]),
        );
        if describe.declares(groups::APP_READERS) {
            members.insert("readers".into(), json!([&a.module, &b.module]));
        }
        case.subject
            .send_fields_with_plan(&session.name, true, &members)
            .and_then(|fields| fields.get("plan").and_then(Value::as_object).cloned())
            .ok_or("adapter cannot build the adopt_open sender plan")?
    } else {
        host_plan(case, &session.name, false)?
    };
    let reply = adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    if reply.last_ordinal.is_some() {
        return Err("adopt_open reply names a message ordinal".into());
    }
    ready(&session.route).await?;
    empty(&session.route).await?;
    let stranger = stranger(case, &session.name).await?;
    let error = expect_error("non-owner baseline", baseline(&stranger).await?)?;
    expect_code("non-owner baseline", &error, errors::SCOPE_OWNER_MISMATCH)?;
    if let Some([a, b]) = identities {
        let supported: Vec<String> = [
            Mode::ToolResult,
            Mode::TurnSystem,
            Mode::TurnSystemCleared,
            Mode::Steer,
            Mode::IdleStart,
        ]
        .into_iter()
        .filter(|mode| mode.is_declared_by(describe))
        .map(|mode| mode.as_str().to_owned())
        .collect();
        for (identity, mut expected) in [
            (
                a,
                HeadSender::new(
                    supported
                        .iter()
                        .filter(|mode| *mode != "idle_start")
                        .cloned()
                        .collect(),
                    false,
                ),
            ),
            (b, HeadSender::new(supported, true)),
        ] {
            let route = case
                .subject
                .session_route(case.handle.inner(), &session.name, &identity.stamp)
                .await
                .map_err(|e| e.to_string())?;
            let meta = match call(&route, ops::SESSION_HEAD, json!({})).await? {
                Reply::Error(error)
                    if error.code == errors::SCOPE_OWNER_MISMATCH
                        && !describe.declares(groups::APP_READERS) =>
                {
                    return Ok(Ending::PassedWithInapplicableRule(
                        "named-sender grant read not admitted without app_readers; opening and immediate owner reads passed".into(),
                    ));
                }
                reply => crate::drive::checked_head(expect_ok("named sender head", reply)?)?,
            };
            let mut got = meta
                .sender
                .ok_or("opened session omits a named sender's head.sender grant")?;
            got.modes.sort();
            expected.modes.sort();
            if got != expected {
                return Err(format!(
                    "opened session head.sender {got:?}, expected {expected:?}"
                ));
            }
        }
        Ok(Ending::Passed)
    } else {
        Ok(Ending::PassedWithInapplicableRule("named-sender grants not applicable without both head_sender and delivery_order; immediate empty head/read and plan admission passed".into()))
    }
}

async fn repeats<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case.open("adopt-open-repeat", Script::default()).await?;
    let plan = host_plan(case, &session.name, false)?;
    let other = host_plan(case, &session.name, true)?;
    if other == plan {
        return Err("alternate plan equals the opening plan".into());
    }
    let send_id = case.mint.next("adopt");
    let first = expect_ok(
        "opening adopt",
        adopt_raw(&session.route, &send_id, &plan).await?,
    )?;
    let before = transcript(&session.route).await?;
    let repeat = expect_ok(
        "repeat adopt",
        adopt_raw(&session.route, &send_id, &plan).await?,
    )?;
    if first != repeat {
        return Err("adopt_open repeat did not replay the original reply".into());
    }
    let error = expect_error(
        "adopt_open reused plan",
        adopt_raw(&session.route, &send_id, &other).await?,
    )?;
    expect_code("adopt_open reused plan", &error, errors::SEND_ID_REUSE)?;
    expect_field(
        "adopt_open reused plan",
        &error,
        errors::SEND_ID_REUSE,
        PLAN_FIELD,
    )?;
    let again = expect_ok(
        "repeat after reuse refusal",
        adopt_raw(&session.route, &send_id, &plan).await?,
    )?;
    if first != again {
        return Err("adopt_open reuse refusal changed the stored reply".into());
    }
    unchanged(
        &session.route,
        &before,
        "adopt_open repeat and reuse refusal",
    )
    .await
}

async fn existing<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case.open("adopt-open-existing", Script::default()).await?;
    let plan = host_plan(case, &session.name, false)?;
    let other = host_plan(case, &session.name, true)?;
    if other == plan {
        return Err("alternate plan equals the opening plan".into());
    }
    let first = adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    let before = transcript(&session.route).await?;
    let error = expect_error(
        "existing session adopt",
        adopt_raw(&session.route, &case.mint.next("adopt"), &other).await?,
    )?;
    expect_code("existing session adopt", &error, errors::PLAN_DRIFT)?;
    let detail: PlanDriftDetail = decode("plan_drift", error.detail.unwrap_or(Value::Null))?;
    if detail.frozen.as_deref() != Some(&first.applied_plan_digest)
        || detail.sent == first.applied_plan_digest
    {
        return Err(format!(
            "existing session plan_drift has wrong identities: {detail:?}"
        ));
    }
    let again = adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    same_reply("existing session after drift refusal", &first, &again)?;
    unchanged(&session.route, &before, "existing session drift refusal").await
}

async fn non_owner<S>(case: &Case<'_, S>) -> Result<Ending, String>
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
            .is_some_and(|scope| scope.owner == owner.principal)
    {
        return Err(
            "plain-scope fixture must separate carrier and owner under the same scope".into(),
        );
    }
    let name = case.mint.next("adopt-open-carrier");
    case.subject
        .install_script(&name, Script::default())
        .await
        .map_err(|e| e.to_string())?;
    let route = case
        .subject
        .session_route(case.handle.inner(), &name, &carrier)
        .await
        .map_err(|e| e.to_string())?;
    let owner_route = case
        .subject
        .session_route(case.handle.inner(), &name, &owner)
        .await
        .map_err(|e| e.to_string())?;
    let plan = host_plan(case, &name, false)?;
    let other = host_plan(case, &name, true)?;
    if plan == other {
        return Err("alternate plan equals the opening plan".into());
    }
    let send_id = case.mint.next("adopt");
    let error = expect_error(
        "non-owner adopt_open",
        adopt_raw(&route, &send_id, &plan).await?,
    )?;
    expect_code("non-owner adopt_open", &error, errors::SCOPE_OWNER_MISMATCH)?;
    let not_yet: BaselineReply = decode(
        "baseline after refusal",
        expect_ok("baseline after refusal", baseline(&owner_route).await?)?,
    )?;
    if !not_yet.is_not_yet() || not_yet.baseline.is_some() {
        return Err("non-owner adopt_open wrote a baseline".into());
    }
    // A refused caller reserves neither the plan nor the id: the real owner
    // can open with a different plan under the same id.
    adopt(&owner_route, &send_id, &other).await?;
    ready(&owner_route).await?;
    empty(&owner_route).await?;
    Ok(Ending::Passed)
}
