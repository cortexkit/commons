//! The next-send delivery grant must be scoped to the route's attested caller.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    delivery_order::{Mode, PlanSender},
    read::HeadSender,
};
use serde_json::{json, Map};

use crate::{
    cases::{Case, Ending},
    drive::head,
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedTurn},
    tags::send_and_wait,
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
    let fields = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
        .ok_or(
            "head.sender identities supplied but adapter cannot build a plan carrying senders",
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
        check(label, head(&route).await?.sender, Some(expected))?;
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
    check("unnamed caller", head(&stranger).await?.sender, None)?;
    check(
        "owner",
        head(&session.route).await?.sender,
        Some(HeadSender::for_owner(supported)),
    )?;
    Ok(Ending::Passed)
}
