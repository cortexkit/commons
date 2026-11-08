//! The next-send delivery grant must be scoped to the route's attested caller.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    delivery_order::{Mode, PlanSender},
    errors, ops,
    read::{HeadMeta, HeadSender},
};
use serde_json::{json, Map};

use crate::{
    cases::{Case, Ending},
    drive::{call, checked_head, head},
    route::{Reply, RunnerRoute},
    send_helpers::send_and_wait,
    subject::{LlmRunnerSubject, Script, ScriptedTurn},
};

fn check(label: &str, got: Option<HeadSender>, expected: Option<HeadSender>) -> Result<(), String> {
    let sorted = |grant: Option<HeadSender>| {
        grant.map(|mut g| {
            g.modes.sort();
            g
        })
    };
    let got = sorted(got);
    let expected = sorted(expected);
    if got != expected {
        return Err(format!(
            "{label} head.sender {got:?}, expected its own grant {expected:?}"
        ));
    }
    Ok(())
}

/// One `session.head` request for a caller the read rule may refuse. A
/// `scope_owner_mismatch` refusal returns `None`; any other refusal is an
/// error; an answer is decoded and checked as [`head`] does. §4.3 reports
/// `sender` only on reads the runner admits, and whether it admits them is the
/// read rule (§9.19): a plan without `readers` keeps the runner's own read
/// admission, which may refuse non-owners.
async fn head_if_admitted<R: RunnerRoute>(route: &R) -> Result<Option<HeadMeta>, String> {
    match call(route, ops::SESSION_HEAD, json!({})).await? {
        Reply::Error(error) if error.code == errors::SCOPE_OWNER_MISMATCH => Ok(None),
        Reply::Error(error) => Err(format!(
            "session.head was refused {} {:?}: {}",
            error.code, error.detail, error.message
        )),
        Reply::Response(body) => checked_head(body).map(Some),
    }
}

pub async fn run<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    if !describe.declares(groups::HEAD_SENDER) {
        return Ok(Ending::Inapplicable(
            "runner does not declare head_sender".into(),
        ));
    }
    let Some([a, b]) = case.subject.head_sender_identities() else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no two-identity head.sender fixture".into(),
        ));
    };
    if a.module == b.module || a.stamp == b.stamp {
        return Err("head.sender fixture identities must be distinct".into());
    }
    let supported: Vec<String> = [
        Mode::ToolResult,
        Mode::TurnSystem,
        Mode::TurnSystemCleared,
        Mode::Steer,
        Mode::IdleStart,
    ]
    .into_iter()
    .filter(|m| m.is_declared_by(&describe))
    .map(|m| m.as_str().to_owned())
    .collect();
    let mut session = case
        .open(
            "head-sender",
            Script {
                turns: vec![ScriptedTurn::new(vec![case.mint.text("sender-answer").0])],
            },
        )
        .await?;
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
    // A runner with `app_readers` must admit A's and B's reads once the plan
    // lists them. Without that group the plan can't name readers, so whether
    // A and B may read is the runner's own read admission.
    let lists_readers = describe.declares(groups::APP_READERS);
    if lists_readers {
        members.insert("readers".into(), json!([&a.module, &b.module]));
    }
    let fields = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
        .ok_or(
            "head.sender identities supplied but adapter cannot build the requested plan (A and B as senders and, with app_readers, as readers)",
        )?;
    let prompt = case.mint.next("sender-opening");
    send_and_wait(case, &mut session, Some(&fields), &prompt, &Map::new()).await?;
    for (identity, expected, label) in [
        (
            a,
            HeadSender::new(
                supported
                    .iter()
                    .filter(|m| *m != "idle_start")
                    .cloned()
                    .collect(),
                false,
            ),
            "A",
        ),
        (b, HeadSender::new(supported.clone(), true), "B"),
    ] {
        let route = case
            .subject
            .session_route(case.handle.inner(), &session.name, &identity.stamp)
            .await
            .map_err(|e| e.to_string())?;
        let Some(meta) = head_if_admitted(&route).await? else {
            if lists_readers {
                return Err(format!(
                    "{label}'s session.head was refused scope_owner_mismatch although the plan lists it in readers"
                ));
            }
            return Ok(Ending::Inapplicable(format!(
                "plan sender {label}'s session.head was refused scope_owner_mismatch, and without app_readers the case can't request read access through the plan"
            )));
        };
        check(label, meta.sender, Some(expected))?;
    }
    let stranger = case
        .subject
        .session_route(
            case.handle.inner(),
            &session.name,
            &case.subject.stranger_stamp(),
        )
        .await
        .map_err(|e| e.to_string())?;
    // Accept a scope_owner_mismatch refusal, or require no sender grant. This
    // checks grant disclosure only; the app_readers cases check §9.19's
    // refusal of an unlisted reader.
    if let Some(meta) = head_if_admitted(&stranger).await? {
        check("unnamed caller", meta.sender, None)?;
    }
    check(
        "owner",
        head(&session.route).await?.sender,
        Some(HeadSender::for_owner(supported)),
    )?;
    Ok(Ending::Passed)
}
