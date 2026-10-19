//! `tags` cases: runner-owned part tags and `session.resolve`, run only when
//! the subject declares `tags`.
//!
//! Every case reads tags only from `session.read` entries and
//! `session.resolve` answers, never from a message body, whose schema is the
//! runner's own.

use std::collections::{BTreeMap, BTreeSet};

use cortexkit_role_llm_runner::{
    capabilities as groups,
    describe::RoleDescribe,
    errors, ops,
    read::ReadMessage,
    send::SendReply,
    tags::{self, CharRange, ResolveReply, ResolveRequest, RANGE_FIELD, TAG_FIELD},
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, holding, read_all,
        wait_run_end,
    },
    route::{Reply, RunnerRoute},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

type Check = Result<(), String>;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "tags_on_every_read_entry" => on_every_entry(case).await,
        "tags_never_reused_across_runs" => never_reused(case).await,
        "tags_exist_whether_or_not_rendered" => whether_rendered(case).await,
        "tags_resolve_returns_stored_content" => resolve_content(case).await,
        "tags_resolve_unknown_tag_refused" => resolve_unknown(case).await,
        "tags_resolve_range_refused" => resolve_range_refused(case).await,
        "tags_model_written_tags_stored_verbatim" => model_written(case).await,
        other => Err(format!("unknown tags case {other}")),
    }
}

/// The suite sends `session.resolve` only to a runner whose live
/// `role.describe` declares `tags`, as a consumer must.
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

/// Send `prompt` (with `extra` beside the role's members) and wait for the
/// run it starts to end.
pub(crate) async fn send_and_wait<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    fields: Option<&Map<String, Value>>,
    prompt: &str,
    extra: &Map<String, Value>,
) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let send_id = case.mint.next("send");
    let before = head(&session.route)
        .await?
        .last_run_state
        .map(|last| last.run_id);
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
    wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    Ok(())
}

/// A session of one run per prompt, answered by `turns`.
async fn session_with<S>(
    case: &Case<'_, S>,
    label: &str,
    turns: Vec<ScriptedTurn>,
    prompts: &[String],
) -> Result<Session<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case.open(label, Script { turns }).await?;
    require_declared(&session.route, groups::TAGS, ops::SESSION_RESOLVE).await?;
    for prompt in prompts {
        send_and_wait(case, &mut session, None, prompt, &Map::new()).await?;
    }
    Ok(session)
}

/// Every message has at least one tagged part, every tag is well formed,
/// and no tag or part key repeats. Returns each message's tags by `mid`.
fn check_tags(what: &str, messages: &[ReadMessage]) -> Result<BTreeMap<String, Vec<u64>>, String> {
    let mut seen = BTreeSet::new();
    let mut by_mid = BTreeMap::new();
    for message in messages {
        if message.tags.iter().all(|row| row.tag.is_none()) {
            return Err(format!(
                "{what}: the entry at ordinal {} carries no tagged part",
                message.ordinal
            ));
        }
        let mut parts = BTreeSet::new();
        let mut numbers = Vec::new();
        for row in &message.tags {
            if !parts.insert(row.part.as_str()) {
                return Err(format!(
                    "{what}: the entry at ordinal {} has two rows for part {:?}",
                    message.ordinal, row.part
                ));
            }
            let Some(tag) = &row.tag else {
                continue;
            };
            let number = tags::parse_tag(tag).ok_or_else(|| {
                format!(
                    "{what}: the entry at ordinal {} carries the malformed tag {:?}",
                    message.ordinal, tag
                )
            })?;
            if !seen.insert(number) {
                return Err(format!("{what}: the tag {tag} is used twice"));
            }
            numbers.push(number);
        }
        by_mid.insert(message.mid.clone(), numbers);
    }
    Ok(by_mid)
}

/// The one entry holding `marker`.
pub(crate) fn entry_holding<'m>(
    messages: &'m [ReadMessage],
    marker: &str,
) -> Result<&'m ReadMessage, String> {
    let found: Vec<&ReadMessage> = messages
        .iter()
        .filter(|message| message.message.to_string().contains(marker))
        .collect();
    match found.as_slice() {
        [one] => Ok(one),
        other => Err(format!(
            "{} entries hold {marker}, expected exactly one",
            other.len()
        )),
    }
}

async fn resolve_raw<R: RunnerRoute>(route: &R, request: &ResolveRequest) -> Result<Reply, String> {
    call(
        route,
        ops::SESSION_RESOLVE,
        serde_json::to_value(request).expect("serializes"),
    )
    .await
}

async fn resolve<R: RunnerRoute>(
    route: &R,
    request: &ResolveRequest,
) -> Result<ResolveReply, String> {
    decode(
        "session.resolve",
        expect_ok(
            &format!("session.resolve of {}", request.tag),
            resolve_raw(route, request).await?,
        )?,
    )
}

/// Resolve every tag of `entry` and return the answer whose content is
/// exactly `content`, checked against the entry it came from.
async fn resolve_part<R: RunnerRoute>(
    route: &R,
    entry: &ReadMessage,
    content: &str,
) -> Result<ResolveReply, String> {
    let mut answered = Vec::new();
    for row in &entry.tags {
        let Some(tag) = &row.tag else {
            continue;
        };
        let reply = resolve(route, &ResolveRequest::new(tag)).await?;
        if &reply.tag != tag
            || reply.ordinal != entry.ordinal
            || reply.mid != entry.mid
            || reply.part != row.part
        {
            return Err(format!(
                "session.resolve of {} answered {reply:?}, which does not name the entry's part \
                 {:?} at ordinal {} ({})",
                tag, row.part, entry.ordinal, entry.mid
            ));
        }
        if reply.content == content {
            return Ok(reply);
        }
        answered.push(reply.content);
    }
    Err(format!(
        "no tag of the entry at ordinal {} resolves to exactly {content:?}; its parts resolve to \
         {answered:?}",
        entry.ordinal
    ))
}

async fn on_every_entry<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (answer, answer_marker) = case.mint.text("tagged-answer");
    let (last, last_marker) = case.mint.text("tagged-final");
    let call = case.mint.call("tagged-call");
    let prompt = case.mint.next("tagged-prompt");
    let session = session_with(
        case,
        "tags-entries",
        vec![
            ScriptedTurn::new(vec![answer, ScriptedPart::ToolCall(call)]),
            ScriptedTurn::new(vec![last]),
        ],
        std::slice::from_ref(&prompt),
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    let tagged = check_tags("the transcript", &messages)?;
    for marker in [&prompt, &answer_marker, &last_marker] {
        entry_holding(&messages, marker)?;
    }
    let (_, again) = read_all(&session.route).await?;
    let key = |messages: &[ReadMessage]| {
        messages
            .iter()
            .map(|m| (m.mid.clone(), m.tags.clone()))
            .collect::<Vec<_>>()
    };
    if key(&again) != key(&messages) || check_tags("a second read", &again)? != tagged {
        return Err("a second read of the same transcript answered other tags".into());
    }
    Ok(())
}

async fn never_reused<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (first, _) = case.mint.text("first-answer");
    let (second, _) = case.mint.text("second-answer");
    let prompt = case.mint.next("first-prompt");
    let mut session = session_with(
        case,
        "tags-reuse",
        vec![
            ScriptedTurn::new(vec![first]),
            ScriptedTurn::new(vec![second]),
        ],
        std::slice::from_ref(&prompt),
    )
    .await?;
    let (_, before) = read_all(&session.route).await?;
    let earlier = check_tags("the first run's transcript", &before)?;
    let highest = earlier.values().flatten().max().copied().unwrap_or(0);
    let prompt = case.mint.next("second-prompt");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (_, after) = read_all(&session.route).await?;
    let later = check_tags("the transcript after a second run", &after)?;
    for (mid, numbers) in &earlier {
        if later.get(mid) != Some(numbers) {
            return Err(format!(
                "message {mid} carried tags {numbers:?} after the first run and {:?} after the \
                 second",
                later.get(mid)
            ));
        }
    }
    for (mid, numbers) in later.iter().filter(|(mid, _)| !earlier.contains_key(*mid)) {
        if let Some(low) = numbers.iter().find(|n| **n <= highest) {
            return Err(format!(
                "message {mid}, written by the second run, carries tag {low}, at or below the \
                 first run's highest tag {highest}: tags are assigned in increasing order and \
                 never reused"
            ));
        }
    }
    Ok(())
}

async fn whether_rendered<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for render in [true, false] {
        let (answer, _) = case.mint.text("render-answer");
        let mut session = case
            .open(
                if render {
                    "tags-rendered"
                } else {
                    "tags-hidden"
                },
                Script {
                    turns: vec![ScriptedTurn::new(vec![answer])],
                },
            )
            .await?;
        require_declared(&session.route, groups::TAGS, ops::SESSION_RESOLVE).await?;
        let mut members = Map::new();
        if render {
            members.insert(tags::PLAN_RENDER_TAGS.into(), json!(true));
        }
        let fields = case
            .subject
            .send_fields_with_plan(&session.name, true, &members)
            .ok_or("the subject builds no plan carrying render_tags, which this case requires")?;
        let prompt = case.mint.next("render-prompt");
        send_and_wait(case, &mut session, Some(&fields), &prompt, &Map::new()).await?;
        let (_, messages) = read_all(&session.route).await?;
        let what = if render {
            "a session whose plan renders tags"
        } else {
            "a session whose plan leaves render_tags absent"
        };
        check_tags(what, &messages)?;
    }
    Ok(())
}

async fn resolve_content<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (answer, answer_marker) = case.mint.text("resolved-answer");
    // A prompt ending in non-ASCII text, so a range counted in bytes or in
    // UTF-16 code units answers other text than one counted in characters.
    let prompt = format!("{} héllo wörld", case.mint.next("resolved-prompt"));
    let session = session_with(
        case,
        "tags-resolve",
        vec![ScriptedTurn::new(vec![answer])],
        std::slice::from_ref(&prompt),
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    let entry = entry_holding(&messages, &prompt)?;
    let whole = resolve_part(&session.route, entry, &prompt).await?;
    let length = prompt.chars().count() as u64;
    let range = CharRange::new(length - 5, length);
    let ranged = resolve(
        &session.route,
        &ResolveRequest::new(&whole.tag).with_range(range),
    )
    .await?;
    if ranged.content != "wörld" || ranged.range != Some(range) {
        return Err(format!(
            "session.resolve of {} with characters [{}, {}) answered {:?} with range {:?}, not \
             \"wörld\" with the range echoed",
            whole.tag, range.start, range.end, ranged.content, ranged.range
        ));
    }
    let answer_entry = entry_holding(&messages, &answer_marker)?;
    resolve_part(&session.route, answer_entry, &answer_marker).await?;
    Ok(())
}

/// A well-formed tag far above the highest tag any entry of the session
/// carries, so the session never assigned it.
fn unassigned(messages: &[ReadMessage]) -> String {
    let highest = messages
        .iter()
        .flat_map(|m| &m.tags)
        .filter_map(|row| row.tag.as_deref().and_then(tags::parse_tag))
        .max()
        .unwrap_or(0);
    tags::format_tag(highest + 1000).expect("positive")
}

async fn resolve_unknown<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (answer, _) = case.mint.text("unknown-answer");
    let prompt = case.mint.next("unknown-prompt");
    let session = session_with(
        case,
        "tags-unknown",
        vec![ScriptedTurn::new(vec![answer])],
        std::slice::from_ref(&prompt),
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    let tag = unassigned(&messages);
    let error = expect_error(
        "session.resolve of a tag the session never assigned",
        resolve_raw(&session.route, &ResolveRequest::new(&tag)).await?,
    )?;
    expect_code(
        "session.resolve of a tag the session never assigned",
        &error,
        errors::UNKNOWN_TAG,
    )?;
    for malformed in ["§0§", "12", "§012§"] {
        let what = format!("session.resolve of the malformed tag {malformed:?}");
        let error = expect_error(
            &what,
            resolve_raw(&session.route, &ResolveRequest::new(malformed)).await?,
        )?;
        expect_field(&what, &error, errors::INVALID_PARAMS, TAG_FIELD)?;
    }
    Ok(())
}

async fn resolve_range_refused<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (answer, _) = case.mint.text("range-answer");
    let prompt = case.mint.next("range-prompt");
    let session = session_with(
        case,
        "tags-range",
        vec![ScriptedTurn::new(vec![answer])],
        std::slice::from_ref(&prompt),
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    let whole = resolve_part(&session.route, entry_holding(&messages, &prompt)?, &prompt).await?;
    let length = prompt.chars().count() as u64;
    for (start, end) in [(0, length + 1), (3, 2)] {
        let what = format!(
            "session.resolve of {} with characters [{start}, {end})",
            whole.tag
        );
        let request = ResolveRequest::new(&whole.tag).with_range(CharRange::new(start, end));
        let error = expect_error(&what, resolve_raw(&session.route, &request).await?)?;
        expect_field(&what, &error, errors::INVALID_PARAMS, RANGE_FIELD)?;
    }
    let empty = resolve(
        &session.route,
        &ResolveRequest::new(&whole.tag).with_range(CharRange::new(length, length)),
    )
    .await?;
    if !empty.content.is_empty() {
        return Err(format!(
            "an empty range at the end of {} answered {:?}, not \"\"",
            whole.tag, empty.content
        ));
    }
    Ok(())
}

async fn model_written<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let text = format!("{} as §1§ says", case.mint.next("model-tag"));
    let mut call = case.mint.call("model-tag-call");
    call.arguments["quote"] = json!("see §1§ and §2§");
    let (last, _) = case.mint.text("model-tag-final");
    let mut session = case
        .open(
            "tags-model-written",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![
                        ScriptedPart::Text(text.clone()),
                        ScriptedPart::ToolCall(call.clone()),
                    ]),
                    ScriptedTurn::new(vec![last]),
                ],
            },
        )
        .await?;
    require_declared(&session.route, groups::TAGS, ops::SESSION_RESOLVE).await?;
    let prompt = case.mint.next("model-tag-prompt");
    let ended = send_and_wait(case, &mut session, None, &prompt, &Map::new()).await;
    // Checked before the run's end: a runner that rewrote the arguments sent
    // the tool something the script does not answer.
    let invoked = case.subject.tool_invocations(&call.arguments).await;
    if invoked != 1 {
        return Err(format!(
            "the scripted tool received the model's arguments {} exactly {invoked} times, not \
             once: arguments holding a model-written §N§ must be dispatched as written",
            call.arguments
        ));
    }
    ended?;
    let (_, messages) = read_all(&session.route).await?;
    if holding(&messages, &text) != 1 {
        return Err(format!(
            "{} entries hold the model's text {text:?} as written, expected one: a model-written \
             §N§ is never rewritten in storage",
            holding(&messages, &text)
        ));
    }
    resolve_part(&session.route, entry_holding(&messages, &text)?, &text).await?;
    Ok(())
}
