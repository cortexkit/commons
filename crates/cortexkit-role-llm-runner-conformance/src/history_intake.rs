//! Initial host-history intake, observed while the adapter holds its progress.

use cortexkit_role_llm_runner::{capabilities, history::HistoryPage, ops, read::ReadMessage};
use serde_json::{json, Map};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, read_all, wait_run_end},
    send_helpers::send,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    Capability, RunnerRoute,
};

async fn snapshot<R: RunnerRoute>(
    route: &R,
    version: u64,
    history: bool,
) -> Result<Vec<ReadMessage>, String> {
    let (pages, messages) = read_all(route).await?;
    if pages.iter().any(|page| page.history_version != version)
        || head(route).await?.history_version != version
    {
        return Err("history_version changed during append-only history intake".into());
    }
    if history {
        let mut params = json!({"from_ordinal":1,"limit":100});
        let mut entries = Vec::new();
        for _ in 0..64 {
            let page: HistoryPage = decode(
                "intake history",
                expect_ok(
                    "intake history",
                    call(route, ops::SESSION_HISTORY, params).await?,
                )?,
            )?;
            if page.history_version != version {
                return Err("raw history_version changed during intake".into());
            }
            entries.extend(page.messages);
            if let Some(cursor) = page.newer {
                params = json!({"cursor":cursor});
            } else {
                break;
            }
        }
        if entries.len() != messages.len()
            || entries.iter().zip(&messages).any(|(entry, message)| {
                entry.mid != message.mid
                    || entry.ordinal != message.ordinal
                    || entry.message != message.message
            })
        {
            return Err(
                "raw history does not expose the same growing intake prefix as reads".into(),
            );
        }
    }
    Ok(messages)
}

fn extends(before: &[ReadMessage], after: &[ReadMessage]) -> Result<(), String> {
    if after.len() <= before.len()
        || before
            .iter()
            .zip(after)
            .any(|(a, b)| a.mid != b.mid || a.ordinal != b.ordinal || a.message != b.message)
    {
        return Err("history intake did not grow an append-only prefix".into());
    }
    Ok(())
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    if !case
        .checked_describe()
        .await?
        .declares(capabilities::HISTORY_INTAKE)
    {
        return Ok(Ending::Inapplicable(
            "history_intake is not declared".into(),
        ));
    }
    if ![
        Capability::TranscriptReads,
        Capability::Queue,
        Capability::RunOps,
    ]
    .iter()
    .all(|need| case.declared.contains(need))
    {
        return Ok(Ending::Inapplicable(
            "history intake fixture requires transcript_reads, queue and run_ops".into(),
        ));
    }
    let mut session = case
        .open(
            name,
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(
                    "intake answer".into(),
                )])],
            },
        )
        .await?;
    let texts: Vec<String> = (0..3).map(|_| case.mint.next("imported-history")).collect();
    if !case
        .subject
        .begin_history_intake(&session.name, &texts)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(
            "adapter supplies no history-intake fixture".into(),
        ));
    }
    let first_head = head(&session.route).await?;
    if first_head.history_intake.as_deref() != Some("running") {
        return Err("history_intake is missing while the seeded intake is running".into());
    }
    let history = case.declared.contains(&Capability::HostHistory);
    let first = snapshot(&session.route, first_head.history_version, history).await?;
    if !serde_json::to_string(&first).unwrap().contains(&texts[0]) {
        return Err("initial intake prefix omits its seeded entry".into());
    }
    let prompt = case.mint.next("queued-during-intake");
    let receipt = if name == "history_intake_send_waits_for_history" {
        let receipt = send(case, &mut session, None, &prompt, &Map::new()).await?;
        if case
            .subject
            .model_invocations(&session.name)
            .await
            .map_err(|e| e.to_string())?
            != 0
        {
            return Err("send started a model request before imported history was complete".into());
        }
        Some(receipt)
    } else {
        None
    };
    if !case
        .subject
        .advance_history_intake(&session.name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("intake fixture cannot advance after beginning".into());
    }
    let next_head = head(&session.route).await?;
    if next_head.history_intake.as_deref() != Some("running") {
        return Err("history_intake was dropped before the whole import was stored".into());
    }
    let next = snapshot(&session.route, first_head.history_version, history).await?;
    extends(&first, &next)?;
    if !serde_json::to_string(&next).unwrap().contains(&texts[1]) {
        return Err("advanced intake prefix omits its next seeded entry".into());
    }
    if receipt.is_some()
        && case
            .subject
            .model_invocations(&session.name)
            .await
            .map_err(|e| e.to_string())?
            != 0
    {
        return Err("queued run started while intake was still running".into());
    }
    if !case
        .subject
        .finish_history_intake(&session.name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("intake fixture cannot finish after beginning".into());
    }
    if let Some(receipt) = receipt {
        wait_run_end(case.subject, &session.route, case.declared, None, &receipt).await?;
        let requests = case
            .subject
            .model_requests(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        let rendered = requests
            .first()
            .ok_or("queued send never reached the model after intake")?
            .rendered
            .join("\n");
        let mut offset = 0;
        for text in texts.iter().chain(std::iter::once(&prompt)) {
            let position = rendered[offset..]
                .find(text)
                .ok_or("queued run did not receive all imported history before its user turn")?;
            offset += position + text.len();
        }
    }
    if head(&session.route).await?.history_intake.is_some() {
        return Err("history_intake remained present after the import completed".into());
    }
    let complete = snapshot(&session.route, first_head.history_version, history).await?;
    extends(&next, &complete)?;
    if !serde_json::to_string(&complete)
        .unwrap()
        .contains(&texts[2])
    {
        return Err("completed intake omits its final seeded entry".into());
    }
    Ok(Ending::Passed)
}
