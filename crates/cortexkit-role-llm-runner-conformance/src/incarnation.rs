//! A key's log identity outlives cuts, but never a new creation of that log.

use crate::{
    cases::{Case, Ending},
    drive::head,
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedTurn},
    tags::send_and_wait,
};
use serde_json::Map;

pub async fn run<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "incarnation",
            Script {
                turns: vec![ScriptedTurn::new(vec![
                    case.mint.text("incarnation-answer").0,
                ])],
            },
        )
        .await?;
    let prompt = case.mint.next("incarnation-prompt");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let before = head(&session.route).await?;
    let Some(original) = before.incarnation else {
        return Ok(Ending::Inapplicable(
            "runner does not report incarnation".into(),
        ));
    };
    if !case
        .subject
        .cut_incarnation_log(&session.name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(
            "adapter cannot cut the incarnation log".into(),
        ));
    }
    let cut = head(&session.route).await?;
    if cut.cut.is_none() || cut.lineage_id == before.lineage_id {
        return Err("incarnation fixture did not apply a compaction cut".into());
    }
    if cut.incarnation.as_ref() != Some(&original) {
        return Err(format!(
            "cut changed incarnation from {original:?} to {:?}",
            cut.incarnation
        ));
    }
    if !case
        .subject
        .recreate_incarnation_log(&session.name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(
            "adapter cannot recreate the key's log".into(),
        ));
    }
    let recreated = head(&session.route).await?;
    match recreated.incarnation {
        Some(value) if value != original => Ok(Ending::Passed),
        value => Err(format!(
            "re-created log kept or omitted incarnation: before {original:?}, after {value:?}"
        )),
    }
}
