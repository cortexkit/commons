//! Native pruning fixtures are scripted independently of the maintenance path.
//! All observations, including repeated pages, come from the management route.

use crate::{
    cases::{Case, Ending},
    compaction::{
        CompactionMessage, CompactionSetup, ScriptedPartEdit, ScriptedRange, ScriptedView,
    },
    drive::{call, expect_ok},
    route::RunnerRoute,
    subject::{
        ArchivePruningPart, Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn,
    },
};
use cortexkit_role_compaction_provider::notice::{AdvanceNotice, Event};
use cortexkit_role_llm_runner::{
    archive_prunes_reasoning::{kinds as wire_kinds, ReasoningPruned},
    content_digest::content_digest,
    ops,
};
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
    match name {
        "archive_pruning_generation" => return generation(case, kinds).await,
        "archive_pruning_restored_view_refused"
        | "archive_pruning_restored_edit_refused"
        | "archive_pruning_setup_after_prune"
        | "archive_pruning_setup_initial_replaced" => return enrolled(case, name).await,
        _ => {}
    }
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

/// The Setup acceptance that lets a runner prune an enrolled session.
const ACCEPTS: &str = cortexkit_role_compaction_provider::setup::ACCEPTS_PRUNED_REASONING;

async fn head_state<R: RunnerRoute>(route: &R) -> Result<Option<ReasoningPruned>, String> {
    let head = ok(route, ops::SESSION_HEAD, json!({})).await?;
    head.get("reasoning_pruned")
        .map(|value| {
            serde_json::from_value::<ReasoningPruned>(value.clone())
                .map_err(|e| format!("session.head's reasoning_pruned does not decode: {e}"))
        })
        .transpose()
}

/// The ordinal of every message the pages list a pruned part for.
fn pruned_ordinals(pages: &[Value]) -> Vec<u64> {
    let mut ordinals = Vec::new();
    for page in pages {
        for row in page["pruned_reasoning"].as_array().into_iter().flatten() {
            if let Some(ordinal) = page["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| mid(entry) == row["mid"].as_str())
                .and_then(|entry| entry["ordinal"].as_u64())
            {
                ordinals.push(ordinal);
            }
        }
    }
    ordinals
}

/// The head's prune state names the highest pruned ordinal, and every
/// served raw and model page repeats it.
async fn check_carried<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    route: &S::Route,
    state: &ReasoningPruned,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let raw = pages(route, ops::SESSION_READ, json!({})).await?;
    check(
        pruned_ordinals(&raw).into_iter().max() == Some(state.through_ordinal),
        "reasoning_pruned.through_ordinal is not the highest ordinal a prune touched",
    )?;
    let expected = serde_json::to_value(state).unwrap();
    let mut served = raw;
    if case.declared.contains(&Capability::ModelView) {
        served.extend(pages(route, ops::SESSION_READ, json!({"view":"model"})).await?);
    }
    for page in &served {
        check(
            page.get("reasoning_pruned") == Some(&expected),
            "a session.read page does not repeat session.head's reasoning_pruned",
        )?;
    }
    Ok(())
}

async fn notices<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
) -> Result<Vec<AdvanceNotice>, String> {
    let observed = subject
        .compaction_observation(session)
        .await
        .map_err(|e| e.to_string())?;
    observed
        .notices
        .iter()
        .map(|headers| {
            AdvanceNotice::from_headers(headers)
                .map_err(|e| format!("the runner published a malformed notice {headers:?}: {e:?}"))
        })
        .collect()
}

async fn provider_calls<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
) -> Result<Vec<crate::ProviderCall>, String> {
    subject
        .compaction_provider_calls(session)
        .await
        .map_err(|e| e.to_string())
}

async fn one_run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &mut crate::cases::Session<S::Route>,
    turn: Vec<ScriptedPart>,
    label: &str,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    case.subject
        .install_script(
            &session.name,
            Script {
                turns: vec![ScriptedTurn::new(turn)],
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    case.run_prompt(session, &case.mint.next(label)).await?;
    Ok(())
}

/// `reasoning_pruned` is absent before the first prune, 1 after it, one
/// higher after a later prune that removes more, and repeated on pages and
/// notices.
async fn generation<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    kinds: &[String],
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    if !kinds.iter().any(|k| k == wire_kinds::REASONING) {
        return Ok(Ending::Inapplicable(
            "the runner does not declare the reasoning kind, so a scripted reasoning turn gives \
             a later prune nothing to remove"
                .into(),
        ));
    }
    let enrolled = case.declared.contains(&Capability::Compaction);
    let mut session = case.open("archive-generation", Script::default()).await?;
    case.subject
        .archive_pruning_fixture(&session.name, enrolled.then(|| vec![ACCEPTS.into()]))
        .await
        .map_err(|e| e.to_string())?;
    let route = &session.route;
    check(
        head_state(route).await?.is_none(),
        "session.head carries reasoning_pruned before the session's first prune",
    )?;
    check(
        pages(route, ops::SESSION_READ, json!({}))
            .await?
            .iter()
            .all(|p| p.get("reasoning_pruned").is_none()),
        "a page carries reasoning_pruned before the session's first prune",
    )?;
    case.subject
        .prune_archived_session(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let first = head_state(route)
        .await?
        .ok_or("session.head omits reasoning_pruned after the session's first prune")?;
    check(
        first.generation == 1,
        "the first prune's generation is not 1",
    )?;
    check_carried(case, route, &first).await?;
    one_run(
        case,
        &mut session,
        vec![
            ScriptedPart::Reasoning("A later thought to prune.".into()),
            ScriptedPart::Text("A later answer.".into()),
        ],
        "archive-generation-later",
    )
    .await?;
    if enrolled {
        let seen = notices(case.subject, &session.name).await?;
        let last = seen
            .last()
            .ok_or("the runner published no advance notice for a run after the prune")?;
        check(
            last.reasoning_pruned == Some(first),
            "an advance notice after the prune does not carry session.head's reasoning_pruned",
        )?;
    }
    let listed_before =
        pruned_ordinals(&pages(&session.route, ops::SESSION_READ, json!({})).await?);
    case.subject
        .prune_archived_session(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let listed_after = pruned_ordinals(&pages(&session.route, ops::SESSION_READ, json!({})).await?);
    let second = head_state(&session.route)
        .await?
        .ok_or("session.head lost reasoning_pruned after a later prune")?;
    if listed_after == listed_before {
        check(
            second.generation == first.generation,
            "a prune that removed nothing raised the generation",
        )?;
        return Ok(Ending::Inapplicable(
            "the later prune removed nothing from the scripted reasoning turn".into(),
        ));
    }
    check(
        second.generation == first.generation + 1,
        "a later prune did not raise the generation by one",
    )?;
    check(
        second.through_ordinal > first.through_ordinal,
        "a later prune of a newer message did not raise through_ordinal",
    )?;
    check_carried(case, &session.route, &second).await?;
    Ok(Ending::Passed)
}

/// The cases on an enrolled session whose provider accepted pruned reasoning.
async fn enrolled<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut session = case.open(name, Script::default()).await?;
    let oracle = case
        .subject
        .archive_pruning_fixture(&session.name, Some(vec![ACCEPTS.into()]))
        .await
        .map_err(|e| e.to_string())?;
    let text = |label: &str| vec![ScriptedPart::Text(format!("An answer for {label}."))];
    match name {
        "archive_pruning_setup_after_prune" => {
            // The lineage's own Setup comes first, so the one after the prune
            // is a second Setup the runner owes only because of the prune.
            one_run(case, &mut session, text("before"), "archive-setup-before").await?;
            let before = provider_calls(case.subject, &session.name).await?.len();
            case.subject
                .prune_archived_session(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            let state = head_state(&session.route)
                .await?
                .ok_or("session.head omits reasoning_pruned after the prune")?;
            one_run(case, &mut session, text("after"), "archive-setup-after").await?;
            let calls = provider_calls(case.subject, &session.name).await?;
            let first = calls
                .get(before..)
                .and_then(<[_]>::first)
                .ok_or("the runner made no call to the compaction provider after the prune")?;
            check(
                first.method == cortexkit_role_compaction_provider::ops::COMPACTION_SETUP,
                &format!(
                    "the first provider call after the prune was {}, not a Setup",
                    first.method
                ),
            )?;
            check(
                first.params.get("reasoning_pruned") == Some(&serde_json::to_value(state).unwrap()),
                "the post-prune Setup does not carry session.head's reasoning_pruned",
            )?;
        }
        "archive_pruning_restored_edit_refused" => {
            // A provider that kept a pre-prune copy of one part sends it
            // back as a keep-range part edit; the runner must refuse that
            // view and apply the same edit carrying the pruned part.
            let before = entries(&pages(&session.route, ops::SESSION_READ, json!({})).await?);
            let head = ok(&session.route, ops::SESSION_HEAD, json!({})).await?;
            let lineage = head["lineage_id"]
                .as_str()
                .ok_or("session.head omits lineage_id")?
                .to_owned();
            let first_ordinal = head["first_ordinal"].as_u64().unwrap_or(0);
            let (ordinal, part_mid, key, expected) = before
                .iter()
                .filter(|e| e["ordinal"].as_u64().is_some_and(|o| o >= first_ordinal))
                .find_map(|entry| {
                    case.subject
                        .read_parts(&entry["message"])
                        .into_iter()
                        .find_map(|(key, value)| {
                            let expected = oracle
                                .iter()
                                .find(|o| o.original == value && o.original != o.served)?;
                            Some((
                                entry["ordinal"].as_u64()?,
                                mid(entry)?.to_owned(),
                                key,
                                expected.clone(),
                            ))
                        })
                })
                .ok_or("the fixture has no prunable part in the current lineage")?;
            let last = before.last().ok_or("the fixture wrote no messages")?;
            let last_ordinal = last["ordinal"].as_u64().ok_or("entry omitted ordinal")?;
            case.subject
                .prune_archived_session(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            let edit = |part_value: Value| {
                let mut ranges = Vec::new();
                if first_ordinal > 0 {
                    // A lineage after a cut starts with its head.
                    ranges.push(ScriptedRange::Replace {
                        from: first_ordinal,
                        to: first_ordinal,
                        replacement: vec!["The history before the cut.".into()],
                    });
                }
                ranges.push(ScriptedRange::KeepWithVerbatimEdits {
                    from: first_ordinal,
                    to: last_ordinal + 1,
                    edits: vec![ScriptedPartEdit::new(
                        ordinal,
                        part_mid.clone(),
                        key.clone(),
                        part_value,
                    )],
                });
                ScriptedView {
                    lineage_id: lineage.clone(),
                    covers_through: last_ordinal,
                    covers_through_mid: mid(last).unwrap_or_default().to_owned(),
                    ranges,
                    cut: None,
                }
            };
            let restoring = case
                .subject
                .publish_compaction_view(&session.name, edit(expected.original.clone()))
                .await
                .map_err(|e| e.to_string())?;
            one_run(
                case,
                &mut session,
                text("restoring"),
                "archive-edit-restoring",
            )
            .await?;
            let seen = notices(case.subject, &session.name).await?;
            check(
                refused_restoring(&seen, restoring),
                &format!(
                    "a part edit restoring pruned content was not refused \
                     view_refused:{restoring}:pruned_reasoning_restored"
                ),
            )?;
            check(
                !seen
                    .iter()
                    .any(|n| n.applied_view_version == Some(restoring)),
                "the runner applied a part edit that restores pruned content",
            )?;
            let clean = case
                .subject
                .publish_compaction_view(&session.name, edit(expected.served.clone()))
                .await
                .map_err(|e| e.to_string())?;
            one_run(case, &mut session, text("clean"), "archive-edit-clean").await?;
            check(
                notices(case.subject, &session.name)
                    .await?
                    .iter()
                    .any(|n| n.applied_view_version == Some(clean)),
                "the same edit carrying the pruned part did not apply, so the refusal was not \
                 about the restored content",
            )?;
        }
        "archive_pruning_setup_initial_replaced" => {
            let head = ok(&session.route, ops::SESSION_HEAD, json!({})).await?;
            let first_ordinal = head["first_ordinal"].as_u64().unwrap_or(0);
            let initial = |marker: String| {
                CompactionSetup::Ready(CompactionMessage {
                    compaction_id: marker,
                    version: 1,
                    from: first_ordinal,
                    to: first_ordinal,
                    replacement: vec!["A head the provider built.".into()],
                })
            };
            let old = case.mint.next("pre-prune-head");
            case.subject
                .install_compaction_setup(&session.name, initial(old.clone()))
                .await
                .map_err(|e| e.to_string())?;
            one_run(case, &mut session, text("before"), "archive-initial-before").await?;
            let rendered = |page: Value| page["compaction_id"].as_str().map(str::to_owned);
            check(
                rendered(ok(&session.route, ops::SESSION_READ, json!({"view":"model"})).await?)
                    == Some(old),
                "the runner does not render its Setup's initial before the prune",
            )?;
            case.subject
                .prune_archived_session(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            let new = case.mint.next("post-prune-head");
            case.subject
                .install_compaction_setup(&session.name, initial(new.clone()))
                .await
                .map_err(|e| e.to_string())?;
            one_run(case, &mut session, text("after"), "archive-initial-after").await?;
            check(
                rendered(ok(&session.route, ops::SESSION_READ, json!({"view":"model"})).await?)
                    == Some(new),
                "after the post-prune Setup answered a new initial, the runner still renders the \
                 pre-prune one",
            )?;
        }
        _ => {
            // A provider that kept its pre-prune copies replays them in a
            // fold; the runner must refuse that view and apply the same fold
            // built from the pruned copies.
            let before = entries(&pages(&session.route, ops::SESSION_READ, json!({})).await?);
            case.subject
                .prune_archived_session(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            let after = entries(&pages(&session.route, ops::SESSION_READ, json!({})).await?);
            let head = ok(&session.route, ops::SESSION_HEAD, json!({})).await?;
            let lineage = head["lineage_id"]
                .as_str()
                .ok_or("session.head omits lineage_id")?
                .to_owned();
            let first_ordinal = head["first_ordinal"].as_u64().unwrap_or(0);
            let last = before.last().ok_or("the fixture wrote no messages")?;
            let last_ordinal = last["ordinal"].as_u64().ok_or("entry omitted ordinal")?;
            let fold = |copies: &[Value]| ScriptedView {
                lineage_id: lineage.clone(),
                covers_through: last_ordinal,
                covers_through_mid: mid(last).unwrap_or_default().to_owned(),
                ranges: vec![ScriptedRange::ReplaceVerbatim {
                    from: first_ordinal,
                    to: last_ordinal + 1,
                    messages: copies
                        .iter()
                        .filter(|e| e["ordinal"].as_u64().is_some_and(|o| o >= first_ordinal))
                        .map(|e| e["message"].clone())
                        .collect(),
                }],
                cut: None,
            };
            let restoring = case
                .subject
                .publish_compaction_view(&session.name, fold(&before))
                .await
                .map_err(|e| e.to_string())?;
            one_run(
                case,
                &mut session,
                text("restoring"),
                "archive-view-restoring",
            )
            .await?;
            let seen = notices(case.subject, &session.name).await?;
            check(
                refused_restoring(&seen, restoring),
                &format!(
                    "a view restoring pruned content was not refused \
                     view_refused:{restoring}:pruned_reasoning_restored"
                ),
            )?;
            check(
                !seen
                    .iter()
                    .any(|n| n.applied_view_version == Some(restoring)),
                "the runner applied a view that restores pruned content",
            )?;
            let clean = case
                .subject
                .publish_compaction_view(&session.name, fold(&after))
                .await
                .map_err(|e| e.to_string())?;
            one_run(case, &mut session, text("clean"), "archive-view-clean").await?;
            check(
                notices(case.subject, &session.name)
                    .await?
                    .iter()
                    .any(|n| n.applied_view_version == Some(clean)),
                "the same fold built from the pruned copies did not apply, so the refusal was \
                 not about the restored content",
            )?;
        }
    }
    Ok(Ending::Passed)
}

/// Whether a notice refused view `version` `pruned_reasoning_restored`.
fn refused_restoring(seen: &[AdvanceNotice], version: u64) -> bool {
    seen.iter().any(|notice| {
        notice.events.iter().any(|event| {
            matches!(event, Event::ViewRefused { view_version, reason }
                if *view_version == version
                    && reason == cortexkit_role_llm_runner::archive_prunes_reasoning::PRUNED_REASONING_RESTORED)
        })
    })
}

#[cfg(test)]
mod tests;
