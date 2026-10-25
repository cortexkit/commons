//! Native pruning fixtures are scripted independently of the maintenance path.
//! All observations, including repeated pages, come from the management route.

use crate::{
    cases::{Case, Ending},
    drive::{call, expect_ok},
    route::RunnerRoute,
    subject::{
        ArchivePruningPart, Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn,
    },
};
use cortexkit_role_llm_runner::{content_digest::content_digest, ops};
use serde_json::{json, Value};

#[derive(Clone)]
struct Part {
    mid: String,
    key: String,
    oracle: ArchivePruningPart,
}

async fn ok<R: RunnerRoute>(route: &R, method: &str, params: Value) -> Result<Value, String> {
    expect_ok(method, call(route, method, params).await?)
}

async fn pages<R: RunnerRoute>(
    route: &R,
    method: &str,
    mut request: Value,
) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    request["limit"] = json!(1);
    if method == ops::SESSION_READ {
        request["from_ordinal"] = json!(0);
    }
    for _ in 0..1000 {
        let page = ok(route, method, request.clone()).await?;
        let next = if method == ops::SESSION_READ {
            page.get("next_from_ordinal")
        } else {
            page.get("older")
        }
        .cloned();
        result.push(page);
        match next {
            None => return Ok(result),
            Some(cursor) if method == ops::SESSION_READ => request["from_ordinal"] = cursor,
            Some(cursor) => request["cursor"] = cursor,
        }
    }
    Err("archive pruning page walk did not finish".into())
}

fn entries(pages: &[Value]) -> Vec<Value> {
    pages
        .iter()
        .flat_map(|p| p["messages"].as_array().into_iter().flatten().cloned())
        .collect()
}

fn mid(entry: &Value) -> Option<&str> {
    entry["mid"]
        .as_str()
        .or_else(|| entry["source"]["mid"].as_str())
}

fn check(test: bool, why: &str) -> Result<(), String> {
    if test {
        Ok(())
    } else {
        Err(why.into())
    }
}

fn check_pages(pages: &[Value], parts: &[Part]) -> Result<(), String> {
    for page in pages {
        let served_mids: Vec<_> = page["messages"]
            .as_array()
            .ok_or("page omitted messages")?
            .iter()
            .filter_map(mid)
            .collect();
        let expected: Vec<_> = parts
            .iter()
            .filter(|p| served_mids.contains(&p.mid.as_str()))
            .collect();
        if expected.is_empty() {
            check(
                page.get("pruned_reasoning").is_none(),
                "pruned_reasoning must be absent on an unaffected page",
            )?;
            continue;
        }
        let listed = page["pruned_reasoning"]
            .as_array()
            .ok_or("page omitted pruned_reasoning")?;
        check(
            listed.len() == expected.len(),
            "page did not list exactly its pruned parts",
        )?;
        for part in expected {
            check(
                listed
                    .iter()
                    .filter(|r| r["mid"] == part.mid && r["part"] == part.key)
                    .count()
                    == 1,
                "page omitted or duplicated a pruned part",
            )?;
            let row = listed
                .iter()
                .find(|r| r["mid"] == part.mid && r["part"] == part.key)
                .unwrap();
            check(
                row["kind"] == part.oracle.kind
                    && row["sha256"] == content_digest(&part.oracle.original)
                    && row["bytes"]
                        == serde_jcs::to_vec(&part.oracle.original).unwrap().len() as u64,
                "page attestation does not describe the original part",
            )?;
        }
    }
    Ok(())
}

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    let kinds = &describe
        .archive_prunes_reasoning
        .as_ref()
        .ok_or("describe omitted archive_prunes_reasoning")?
        .kinds;
    let mut session = case.open(name, Script::default()).await?;
    let accepts = match name {
        "archive_pruning_no_provider" => None,
        "archive_pruning_setup_opt_in" => Some(vec![]),
        _ => Some(vec!["pruned_reasoning".into()]),
    };
    let oracle = case
        .subject
        .archive_pruning_fixture(&session.name, accepts)
        .await
        .map_err(|e| e.to_string())?;
    let before_pages = pages(&session.route, ops::SESSION_READ, json!({})).await?;
    let before = entries(&before_pages);
    let mut parts = Vec::new();
    for entry in &before {
        for (key, value) in case.subject.read_parts(&entry["message"]) {
            if let Some(expected) = oracle.iter().find(|o| o.original == value) {
                check(
                    kinds.contains(&expected.kind),
                    "fixture used an undeclared kind",
                )?;
                check(
                    expected.original != expected.served,
                    "fixture's pruning oracle changed nothing",
                )?;
                parts.push(Part {
                    mid: entry["mid"].as_str().ok_or("entry omitted mid")?.into(),
                    key,
                    oracle: expected.clone(),
                });
            }
        }
    }
    for kind in kinds {
        let mids: std::collections::BTreeSet<_> = parts
            .iter()
            .filter(|p| &p.oracle.kind == kind)
            .map(|p| &p.mid)
            .collect();
        check(
            mids.len() >= 2,
            "fixture must exercise every declared kind in at least two messages",
        )?;
    }
    check(
        before_pages
            .iter()
            .all(|p| p.get("pruned_reasoning").is_none()),
        "fixture was already pruned",
    )?;
    let before_head = ok(&session.route, ops::SESSION_HEAD, json!({})).await?;
    let history_cursor = if name == "archive_pruning_history_cursor" {
        let page = ok(&session.route, ops::SESSION_HISTORY, json!({"limit":1})).await?;
        Some(
            page["older"]
                .as_str()
                .ok_or("fixture history did not provide an older cursor")?
                .to_owned(),
        )
    } else {
        None
    };
    let mut resolves = Vec::new();
    if name == "archive_pruning_identity" && case.declared.contains(&Capability::Tags) {
        for entry in &before {
            for tag in entry["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r["tag"].as_str())
            {
                resolves.push((
                    tag.to_owned(),
                    ok(&session.route, ops::SESSION_RESOLVE, json!({"tag":tag})).await?,
                ));
            }
        }
    }
    let mut blocked = name == "archive_pruning_setup_opt_in";
    if name == "archive_pruning_active_run" || name == "archive_pruning_queued_send" {
        if name == "archive_pruning_queued_send" {
            if !case
                .subject
                .hold_retention_queue(&session.name)
                .await
                .map_err(|e| e.to_string())?
            {
                return Ok(Ending::Inapplicable(
                    "the adapter cannot hold an idle queue".into(),
                ));
            }
        } else {
            case.subject
                .hold_next_model_step(&session.name)
                .await
                .map_err(|e| e.to_string())?;
        }
        case.subject
            .install_script(
                &session.name,
                Script {
                    turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(
                        "A new answer.".into(),
                    )])],
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let params = case.params(
            &session,
            "A new prompt.",
            &case.mint.next("busy-send"),
            None,
        );
        ok(&session.route, ops::SESSION_SEND, params).await?;
        if name == "archive_pruning_active_run" {
            case.subject
                .await_model_step(&session.name)
                .await
                .map_err(|e| e.to_string())?;
        }
        blocked = true;
    }
    case.subject
        .prune_archived_session(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let after_pages = pages(&session.route, ops::SESSION_READ, json!({})).await?;
    let after = entries(&after_pages);
    if blocked {
        for old in &before {
            check(
                after.iter().find(|e| e["mid"] == old["mid"]) == Some(old),
                "pruned a session while busy or without Setup acceptance",
            )?;
        }
        check(
            after_pages
                .iter()
                .all(|p| p.get("pruned_reasoning").is_none()),
            "blocked pruning listed pruned parts",
        )?;
        if name == "archive_pruning_active_run" {
            case.subject
                .release_model_step(&session.name)
                .await
                .map_err(|e| e.to_string())?;
        }
        if name == "archive_pruning_queued_send" {
            case.subject
                .release_retention_queue(&session.name)
                .await
                .map_err(|e| e.to_string())?;
        }
        return Ok(Ending::Passed);
    }
    check(
        before.len() == after.len(),
        "pruning added or removed messages",
    )?;
    if let Some(cursor) = history_cursor {
        let page = ok(
            &session.route,
            ops::SESSION_HISTORY,
            json!({"cursor":cursor,"limit":1}),
        )
        .await?;
        check(
            !page["messages"]
                .as_array()
                .ok_or("history page omitted messages")?
                .is_empty(),
            "pre-prune cursor returned an empty page",
        )?;
        check_pages(&[page], &parts)?;
        return Ok(Ending::Passed);
    }
    if name == "archive_pruning_reopen" {
        case.subject
            .install_script(
                &session.name,
                Script {
                    turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(
                        "Continue without reasoning.".into(),
                    )])],
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        case.run_prompt(&mut session, &case.mint.next("reopen"))
            .await?;
        let reopened_pages = pages(&session.route, ops::SESSION_READ, json!({})).await?;
        let reopened = entries(&reopened_pages);
        for entry in &after {
            check(
                reopened.iter().find(|r| r["mid"] == entry["mid"]) == Some(entry),
                "reopening restored or changed pruned history",
            )?;
        }
        check_pages(&reopened_pages, &parts)?;
        return Ok(Ending::Passed);
    }
    match name {
        "archive_pruning_every_page" => {
            for _ in 0..2 {
                check_pages(
                    &pages(&session.route, ops::SESSION_READ, json!({})).await?,
                    &parts,
                )?;
                if case.declared.contains(&Capability::ModelView) {
                    check_pages(
                        &pages(&session.route, ops::SESSION_READ, json!({"view":"model"})).await?,
                        &parts,
                    )?;
                }
                if case.declared.contains(&Capability::HostHistory) {
                    check_pages(
                        &pages(&session.route, ops::SESSION_HISTORY, json!({})).await?,
                        &parts,
                    )?;
                }
                if case.declared.contains(&Capability::HistoryDisplay) {
                    check_pages(
                        &pages(
                            &session.route,
                            ops::SESSION_HISTORY,
                            json!({"form":"display"}),
                        )
                        .await?,
                        &parts,
                    )?;
                }
            }
        }
        "archive_pruning_declared_kinds" | "archive_pruning_original_attestation" => {
            for page in &after_pages {
                for listed in page["pruned_reasoning"]
                    .as_array()
                    .ok_or("missing pruning attestation")?
                {
                    let part = parts
                        .iter()
                        .find(|p| listed["mid"] == p.mid && listed["part"] == p.key)
                        .ok_or("listed an unknown pruned part")?;
                    if name == "archive_pruning_declared_kinds" {
                        check(
                            kinds.iter().any(|k| listed["kind"] == *k)
                                && listed["kind"] == part.oracle.kind,
                            "applied an undeclared or incorrect pruning kind",
                        )?;
                    } else {
                        check(
                            listed["sha256"] == content_digest(&part.oracle.original),
                            "listed sha256 differs from the write-time part digest",
                        )?;
                        check(
                            listed["bytes"]
                                == serde_jcs::to_vec(&part.oracle.original).unwrap().len() as u64,
                            "listed bytes differs from original JCS length",
                        )?;
                    }
                }
            }
        }
        _ => {
            for (old, new) in before.iter().zip(&after) {
                let own: Vec<_> = parts.iter().filter(|p| old["mid"] == p.mid).collect();
                match name {
                    "archive_pruning_payloads" | "archive_pruning_no_provider" => {
                        let served = case.subject.read_parts(&new["message"]);
                        for part in own {
                            check(served.iter().any(|(key, value)| key == &part.key && value == &part.oracle.served), "pruned payload differs from the native fixture (required fields must be emptied, not removed)")?;
                        }
                    }
                    "archive_pruning_identity" => {
                        for field in ["mid", "ordinal", "tags", "role", "written_at"] {
                            check(
                                old[field] == new[field],
                                "pruning changed part/message identity or numbering",
                            )?;
                        }
                        let old_parts = case.subject.read_parts(&old["message"]);
                        let new_parts = case.subject.read_parts(&new["message"]);
                        check(
                            old_parts.len() == new_parts.len()
                                && old_parts
                                    .iter()
                                    .zip(&new_parts)
                                    .all(|((a, av), (b, bv))| a == b && av["type"] == bv["type"]),
                            "pruning moved a part or changed its type",
                        )?;
                    }
                    "archive_pruning_write_time_digests" => {
                        for field in ["digest", "part_digests"] {
                            check(
                                old[field] == new[field],
                                "pruning changed write-time digests",
                            )?;
                        }
                    }
                    "archive_pruning_only_listed_parts" => {
                        let listed: Vec<_> = after_pages
                            .iter()
                            .filter(|page| {
                                page["messages"].as_array().is_some_and(|rows| {
                                    rows.iter().any(|entry| entry["mid"] == old["mid"])
                                })
                            })
                            .flat_map(|p| p["pruned_reasoning"].as_array().into_iter().flatten())
                            .collect();
                        let mut expected = old["message"].clone();
                        for part in own.iter().filter(|p| {
                            listed
                                .iter()
                                .any(|r| r["mid"] == p.mid && r["part"] == p.key)
                        }) {
                            check(
                                case.subject.replace_read_part(
                                    &mut expected,
                                    &part.key,
                                    part.oracle.served.clone(),
                                ),
                                "adapter could not replace the listed native part by key",
                            )?;
                        }
                        check(
                            expected == new["message"],
                            "a part changed without being listed on its page",
                        )?;
                    }
                    _ => return Err(format!("unknown archive pruning case {name}")),
                }
            }
            if name == "archive_pruning_identity" {
                for (before, after) in before_pages.iter().zip(&after_pages) {
                    check(
                        before["lineage_id"] == after["lineage_id"],
                        "pruning changed a page's lineage",
                    )?;
                }
                let after_head = ok(&session.route, ops::SESSION_HEAD, json!({})).await?;
                for field in ["lineage_id", "history_version"] {
                    check(
                        before_head[field] == after_head[field],
                        "pruning changed lineage or history_version",
                    )?;
                }
                for (tag, before) in resolves {
                    let after =
                        ok(&session.route, ops::SESSION_RESOLVE, json!({"tag":tag})).await?;
                    check(
                        before == after,
                        "signature pruning changed a numbered part's resolve identity or text",
                    )?;
                }
            }
        }
    }
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests;
