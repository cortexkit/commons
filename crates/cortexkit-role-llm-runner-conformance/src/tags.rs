//! `tags` cases: runner-rendered part numbers and `session.resolve`, run only
//! when the subject declares `tags`.
//!
//! A tag number is only the name a part shows the model in one lineage,
//! computed from the transcript (llm-runner 0.4.0 §9.6). The cases read
//! numbers from `session.read` entries, `session.resolve` answers and, where
//! the adapter records them, the texts each model request carried; never
//! from a message body, whose schema is the runner's own. Numbering applies
//! only to sessions whose plan renders tags, so every case but the one that
//! checks exactly that opens its sessions with `render_tags: true`.

use std::collections::{BTreeMap, BTreeSet};

use cortexkit_role_llm_runner::{
    capabilities as groups,
    describe::RoleDescribe,
    errors::{self, ForkUnsupportedDetail},
    ops,
    read::ReadMessage,
    send::SendReply,
    tags::{self, CharRange, ResolveReply, ResolveRequest, RANGE_FIELD, TAG_FIELD},
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all,
        run_result, wait_run_end,
    },
    route::{Reply, RunnerRoute},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

type Check = Result<(), String>;

fn passed(check: Check) -> Result<Ending, String> {
    check.map(|()| Ending::Passed)
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "tags_on_every_read_entry" => passed(on_every_entry(case).await),
        "tags_numbers_stable_within_a_lineage" => passed(stable_within_lineage(case).await),
        "tags_only_on_tag_rendering_sessions" => passed(only_where_rendered(case).await),
        "tags_resolve_returns_exposed_content" => passed(resolve_content(case).await),
        "tags_resolve_unknown_tag_refused" => passed(resolve_unknown(case).await),
        "tags_resolve_range_refused" => passed(resolve_range_refused(case).await),
        "tags_model_written_tags_cleaned_on_exposure" => cleaned_on_exposure(case).await,
        "tags_numbered_in_rendered_order" => rendered_order(case).await,
        "tags_renumbering_waits_for_the_step_boundary" => passed(step_boundary(case).await),
        "tags_count_only_parts_rendered_with_a_number" => counted_parts(case).await,
        "tags_setup_checkpoint_anchors_at_its_cut" => passed(setup_checkpoint(case).await),
        "tags_fork_of_a_managed_session_refused" => fork_refused(case).await,
        "tags_lost_marker_key_keeps_numbers" => lost_marker_key(case).await,
        "tags_host_store_cleaned_at_the_save_seam" => save_seam(case).await,
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

/// The first send's fields for a session whose plan renders tags.
pub(crate) fn rendering_fields<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
) -> Result<Map<String, Value>, String> {
    let mut members = Map::new();
    members.insert(tags::PLAN_RENDER_TAGS.into(), json!(true));
    subject
        .send_fields_with_plan(session, true, &members)
        .ok_or_else(|| {
            "the subject builds no plan carrying render_tags (send_fields_with_plan), which the \
             tags cases require: numbering applies only to sessions whose plan renders tags"
                .into()
        })
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
async fn send<S>(
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

/// A session whose plan renders tags, of one run per prompt, answered by
/// `turns`.
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
    let fields = rendering_fields(case.subject, &session.name)?;
    for prompt in prompts {
        send_and_wait(case, &mut session, Some(&fields), prompt, &Map::new()).await?;
    }
    Ok(session)
}

/// Every number is well formed and used once, every part key is used once
/// in its message, and the numbers together are consecutive: within one
/// lineage of a session that never compacted there are no gaps. Returns
/// each message's numbers by `mid`.
fn check_tags(what: &str, messages: &[ReadMessage]) -> Result<BTreeMap<String, Vec<u64>>, String> {
    let mut seen = BTreeSet::new();
    let mut by_mid = BTreeMap::new();
    for message in messages {
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
                return Err(format!(
                    "{what}: the tag {tag} is used twice in one lineage"
                ));
            }
            numbers.push(number);
        }
        by_mid.insert(message.mid.clone(), numbers);
    }
    if let (Some(low), Some(high)) = (seen.first(), seen.last()) {
        if high - low + 1 != seen.len() as u64 {
            return Err(format!(
                "{what}: the numbers {seen:?} are not consecutive: within a lineage the counted \
                 parts are numbered one after another"
            ));
        }
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

/// The numbers the parts of `entry` carry.
fn numbers_of(entry: &ReadMessage) -> Vec<u64> {
    entry
        .tags
        .iter()
        .filter_map(|row| row.tag.as_deref().and_then(tags::parse_tag))
        .collect()
}

/// The one number the entry holding `marker` carries.
fn number_of(messages: &[ReadMessage], marker: &str, what: &str) -> Result<u64, String> {
    let entry = entry_holding(messages, marker)?;
    match numbers_of(entry).as_slice() {
        [one] => Ok(*one),
        other => Err(format!(
            "{what} at ordinal {} carries the numbers {other:?}, expected exactly one",
            entry.ordinal
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
                 {:?} at ordinal {} ({}) that shows it in the current lineage",
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
        number_of(&messages, marker, "a text part")?;
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

/// Two runs: within one lineage every part keeps its number and the
/// numbers stay consecutive; a part's number changes only with a lineage
/// change.
async fn stable_within_lineage<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (first, _) = case.mint.text("first-answer");
    let (second, _) = case.mint.text("second-answer");
    let prompt = case.mint.next("first-prompt");
    let mut session = session_with(
        case,
        "tags-stable",
        vec![
            ScriptedTurn::new(vec![first]),
            ScriptedTurn::new(vec![second]),
        ],
        std::slice::from_ref(&prompt),
    )
    .await?;
    let (pages, before) = read_all(&session.route).await?;
    let lineage = pages.last().map(|page| page.lineage_id.clone());
    let earlier = check_tags("the first run's transcript", &before)?;
    let prompt = case.mint.next("second-prompt");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (pages, after) = read_all(&session.route).await?;
    let later = check_tags("the transcript after a second run", &after)?;
    if pages.last().map(|page| page.lineage_id.clone()) != lineage {
        // A lineage change may renumber; within the new lineage the
        // numbers are checked above.
        return Ok(());
    }
    for (mid, numbers) in &earlier {
        if later.get(mid) != Some(numbers) {
            return Err(format!(
                "message {mid} carried tags {numbers:?} after the first run and {:?} after the \
                 second, under the same lineage: a part's number changes only with a lineage \
                 change",
                later.get(mid)
            ));
        }
    }
    Ok(())
}

/// A session whose plan renders tags numbers its parts; one whose plan
/// leaves `render_tags` absent numbers none, and resolves none.
async fn only_where_rendered<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for render in [true, false] {
        let (answer, answer_marker) = case.mint.text("render-answer");
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
        if render {
            check_tags("a session whose plan renders tags", &messages)?;
            number_of(&messages, &prompt, "the prompt")?;
            number_of(&messages, &answer_marker, "the answer")?;
            continue;
        }
        if let Some(entry) = messages.iter().find(|m| !numbers_of(m).is_empty()) {
            return Err(format!(
                "a session whose plan leaves render_tags absent numbers the entry at ordinal {} \
                 ({:?}): numbering applies only to sessions whose plan renders tags",
                entry.ordinal, entry.tags
            ));
        }
        if messages.iter().any(|m| m.tags.is_empty()) {
            return Err(
                "a session whose plan leaves render_tags absent reads an entry without part \
                 rows: every exposed part keeps its row and key, numbered or not"
                    .into(),
            );
        }
        let what = "session.resolve of §1§ on a session whose plan renders no tags";
        let error = expect_error(
            what,
            resolve_raw(&session.route, &ResolveRequest::new("§1§")).await?,
        )?;
        expect_code(what, &error, errors::UNKNOWN_TAG)?;
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
/// carries, so no part of the current lineage carries it.
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
    let what = "session.resolve of a tag no part of the current lineage carries";
    let error = expect_error(
        what,
        resolve_raw(&session.route, &ResolveRequest::new(&tag)).await?,
    )?;
    expect_code(what, &error, errors::UNKNOWN_TAG)?;
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
    if empty.content.as_str() != Some("") {
        return Err(format!(
            "an empty range at the end of {} answered {:?}, not \"\"",
            whole.tag, empty.content
        ));
    }
    Ok(())
}

/// What the model's text reads as on every exposure: `session.read`,
/// `session.resolve` and `run.result`'s final text.
async fn exposed_as<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    run_id: &str,
    written: &str,
    expected: &str,
    numbered: bool,
    what: &str,
) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (_, messages) = read_all(&session.route).await?;
    let marker = written
        .split_whitespace()
        .find(|word| word.starts_with("cf-"))
        .expect("the written text holds a marker");
    let entry = entry_holding(&messages, marker)?;
    if !entry.message.to_string().contains(expected)
        || (written != expected && entry.message.to_string().contains(written))
    {
        return Err(format!(
            "{what}: session.read shows the model's text {written:?} as {}, not as {expected:?}",
            entry.message
        ));
    }
    match (numbers_of(entry).is_empty(), numbered) {
        (true, true) => return Err(format!("{what}: the model's text carries no number")),
        // Whether an untagged session numbers nothing is the business of
        // tags_only_on_tag_rendering_sessions.
        (false, false) => {}
        (false, true) => resolve_part(&session.route, entry, expected)
            .await
            .map(drop)
            .map_err(|e| format!("{what}: {e}"))?,
        (true, false) => {}
    }
    if case.declared.contains(&Capability::RunOps) {
        let result = run_result(&session.route, run_id).await?;
        let text = result
            .final_message
            .as_ref()
            .map(|message| message.text.as_str());
        if text != Some(expected) {
            return Err(format!(
                "{what}: run.result's final text is {text:?}, not {expected:?}"
            ));
        }
    }
    Ok(())
}

/// The model writes tag notation itself. A tag-rendering run's text is
/// cleaned on every exposure by standalone's rules, an untagged run's is
/// exposed byte for byte, a tool call's arguments reach the tool as written,
/// and, where the runner declares `structured_output`, a structured-output
/// run's final text stays raw.
async fn cleaned_on_exposure<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for render in [true, false] {
        let written = format!("§3§ {} as §1§ says, see §5.1", case.mint.next("model-tag"));
        let expected = if render {
            tags::clean_model_text(&written)
        } else {
            written.clone()
        };
        let mut call = case.mint.call("model-tag-call");
        call.arguments["quote"] = json!("see §1§ and §2§");
        let mut session = case
            .open(
                if render {
                    "tags-cleaned"
                } else {
                    "tags-verbatim"
                },
                Script {
                    turns: vec![
                        ScriptedTurn::new(vec![ScriptedPart::ToolCall(call.clone())]),
                        ScriptedTurn::new(vec![ScriptedPart::Text(written.clone())]),
                    ],
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
        let prompt = case.mint.next("model-tag-prompt");
        let ended = send_and_wait(case, &mut session, Some(&fields), &prompt, &Map::new()).await;
        // Checked before the run's end: a runner that rewrote the arguments
        // sent the tool something the script does not answer.
        let invoked = case.subject.tool_invocations(&call.arguments).await;
        if invoked != 1 {
            return Err(format!(
                "the scripted tool received the model's arguments {} exactly {invoked} times, \
                 not once: a tool call's arguments are never cleaned",
                call.arguments
            ));
        }
        let run_id = ended?;
        let what = if render {
            "a tag-rendering run's text is cleaned on every exposure"
        } else {
            "an untagged run's text is exposed byte for byte"
        };
        exposed_as(case, &session, &run_id, &written, &expected, render, what).await?;
    }
    if !case.declared.contains(&Capability::StructuredOutput) {
        return Ok(Ending::PassedWithInapplicableRule(
            "a structured-output run's final text stays raw: the runner does not declare \
             structured_output"
                .into(),
        ));
    }
    structured_output_raw(case).await?;
    Ok(Ending::Passed)
}

/// A `final_text` structured-output run on a tag-rendering session: its
/// final text, JSON holding tag notation, is never cleaned.
async fn structured_output_raw<S>(case: &Case<'_, S>) -> Result<(String, String, String), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let marker = case.mint.next("structured-tag");
    let written = format!("{{\"answer\":\"§12§ {marker} see §5.1\"}}");
    let session = case
        .open(
            "tags-structured",
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(written.clone())])],
            },
        )
        .await?;
    case.subject
        .set_structured_output_support(&session.name, Some(true))
        .await
        .map_err(|e| format!("setting structured-output support: {e}"))?;
    let mut fields = case
        .subject
        .send_fields_with_tools(&session.name, true, &[])
        .ok_or("the subject supplies no tool-free send fields, which structured_output requires")?;
    fields
        .get_mut("plan")
        .and_then(Value::as_object_mut)
        .ok_or("the subject's tool-free send fields carry no plan object to render tags in")?
        .insert(tags::PLAN_RENDER_TAGS.into(), json!(true));
    let structured = cortexkit_role_llm_runner::structured_output::StructuredOutput::new(
        cortexkit_role_llm_runner::structured_output::Mode::FinalText,
        json!({"type": "object", "properties": {"answer": {"type": "string"}}, "required": ["answer"]}),
    );
    let mut extra = Map::new();
    extra.insert(
        "structured_output".into(),
        serde_json::to_value(structured).expect("serializes"),
    );
    let mut session = session;
    let prompt = case.mint.next("structured-prompt");
    let run_id = send_and_wait(case, &mut session, Some(&fields), &prompt, &extra).await?;
    let result = run_result(&session.route, &run_id).await?;
    let text = result.final_message.as_ref().map(|m| m.text.as_str());
    if text != Some(written.as_str()) {
        return Err(format!(
            "a structured-output run's final text reads {text:?} on run.result, not the raw \
             {written:?}: a structured-output run's final text is never cleaned"
        ));
    }
    let (_, messages) = read_all(&session.route).await?;
    let entry = entry_holding(&messages, &marker)?;
    if !entry.message.to_string().contains("§12§") {
        return Err(format!(
            "session.read shows the structured-output run's final text as {}, cleaned: it is \
             never cleaned",
            entry.message
        ));
    }
    Ok((session.name, marker, written))
}

/// The leading `§N§` of a rendered text, if it carries one.
fn leading_number(text: &str) -> Option<u64> {
    let rest = text.strip_prefix(tags::SIGIL)?;
    let end = rest.find(tags::SIGIL)?;
    tags::parse_tag(&text[..end + 2 * tags::SIGIL.len_utf8()])
}

/// The whole transcript and the lineage its last page names.
async fn lineage_read<R: RunnerRoute>(route: &R) -> Result<(String, Vec<ReadMessage>), String> {
    let (pages, messages) = read_all(route).await?;
    let lineage = pages
        .last()
        .and_then(|page| page.lineage_id.clone())
        .unwrap_or_default();
    Ok((lineage, messages))
}

/// A steer sent while the run's tool call is held: one that renders the
/// steer after the call's result stores it before the result. Answers the
/// session, the reads from before and after the steer while the call was
/// held, and the markers of the prompt, the step's text, the result and the
/// steer.
struct HeldSteer<R> {
    session: Session<R>,
    before: (String, Vec<ReadMessage>),
    during: (String, Vec<ReadMessage>),
    after: (String, Vec<ReadMessage>),
    markers: [String; 4],
    arguments: Value,
}

async fn held_steer<S>(case: &Case<'_, S>, label: &str) -> Result<HeldSteer<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (step, step_marker) = case.mint.text("held-step");
    let (last, _) = case.mint.text("held-final");
    let mut held = case.mint.call("held-call");
    held.hold = true;
    let arguments = held.arguments.clone();
    let result_marker = crate::drive::result_marker(&held);
    let mut session = case
        .open(
            label,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![step, ScriptedPart::ToolCall(held)]),
                    ScriptedTurn::new(vec![last]),
                ],
            },
        )
        .await?;
    require_declared(&session.route, groups::TAGS, ops::SESSION_RESOLVE).await?;
    let fields = rendering_fields(case.subject, &session.name)?;
    let prompt = case.mint.next("held-prompt");
    let reply = send(case, &mut session, Some(&fields), &prompt, &Map::new()).await?;
    case.subject
        .await_tool_call(&arguments)
        .await
        .map_err(|e| format!("waiting for the held call: {e}"))?;
    let before = lineage_read(&session.route).await?;
    let steer = case.mint.next("held-steer");
    let mut extra = Map::new();
    extra.insert("delivery".into(), json!(groups::STEER));
    send(case, &mut session, None, &steer, &extra).await?;
    let during = lineage_read(&session.route).await?;
    case.subject
        .release_tool_call(&arguments)
        .await
        .map_err(|e| format!("releasing the held call: {e}"))?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    let after = lineage_read(&session.route).await?;
    Ok(HeldSteer {
        session,
        before,
        during,
        after,
        markers: [prompt, step_marker, result_marker, steer],
        arguments,
    })
}

/// Numbers run in the order the runner renders the history, never in
/// storage order: the last model request's texts carry exactly the numbers
/// `session.read` reports, increasing in request order.
async fn rendered_order<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let held = held_steer(case, "tags-rendered-order").await?;
    let (_, messages) = &held.after;
    let requests = case
        .subject
        .model_requests(&held.session.name)
        .await
        .map_err(|e| format!("observing the model requests: {e}"))?;
    let last = requests
        .last()
        .ok_or("the scripted model received no request")?;
    let mut seen = Vec::new();
    for marker in &held.markers {
        let entry = entry_holding(messages, marker)?;
        let read = number_of(messages, marker, "a counted part")?;
        let (index, text) = last
            .rendered
            .iter()
            .enumerate()
            .find(|(_, text)| text.contains(marker.as_str()))
            .ok_or_else(|| {
                format!("the last model request carried no text holding {marker}: {last:?}")
            })?;
        let shown = leading_number(text).ok_or_else(|| {
            format!("the model saw {text:?} without a leading number, though it counts")
        })?;
        if shown != read {
            return Err(format!(
                "the model saw {text:?} numbered {shown}, but session.read numbers it {read}: \
                 a read reports the number the request shows"
            ));
        }
        seen.push((index, entry.ordinal, shown));
    }
    seen.sort();
    if seen.windows(2).any(|pair| pair[0].2 >= pair[1].2) {
        return Err(format!(
            "in the order the model saw them, the parts are numbered {:?}: numbers run in \
             rendered order, never in storage order",
            seen.iter().map(|(_, _, n)| *n).collect::<Vec<_>>()
        ));
    }
    if seen.windows(2).all(|pair| pair[0].1 < pair[1].1) {
        return Ok(Ending::Inapplicable(
            "the runner rendered the steer where it stored it, so storage order and rendered \
             order did not differ"
                .into(),
        ));
    }
    Ok(Ending::Passed)
}

/// A renumbering the runner detects during a step takes effect at the next
/// step boundary: while the step's tool call is held, a read after a steer
/// answers the lineage and numbers of the read before it, the call runs
/// once, and afterwards a part a read exposed carries another number only
/// on a new lineage.
async fn step_boundary<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let held = held_steer(case, "tags-step-boundary").await?;
    let (lineage, before) = &held.before;
    let (during_lineage, during) = &held.during;
    if during_lineage != lineage {
        return Err(format!(
            "the session moved from lineage {lineage} to {during_lineage} while the step's tool \
             call was still held: a renumbering takes effect at the next step boundary, never \
             before the step's tool calls have run"
        ));
    }
    let numbers = |messages: &[ReadMessage]| -> BTreeMap<(String, String), Option<String>> {
        messages
            .iter()
            .flat_map(|m| {
                m.tags
                    .iter()
                    .map(move |row| ((m.mid.clone(), row.part.clone()), row.tag.clone()))
            })
            .collect()
    };
    let shown = numbers(during);
    for (key, tag) in numbers(before) {
        if shown.get(&key) != Some(&tag) {
            return Err(format!(
                "part {key:?} read {tag:?} before the steer and {:?} after it, both while the \
                 step's tool call was held, under lineage {lineage}",
                shown.get(&key)
            ));
        }
    }
    check_tags("a read while the tool call was held", during)?;
    let invoked = case.subject.tool_invocations(&held.arguments).await;
    if invoked != 1 {
        return Err(format!("the held call ran {invoked} times, not once"));
    }
    let (after_lineage, after) = &held.after;
    check_tags("the transcript after the run", after)?;
    if after_lineage == lineage {
        let now = numbers(after);
        for (key, tag) in &shown {
            if now.get(key) != Some(tag) {
                return Err(format!(
                    "part {key:?} was exposed as {tag:?} and now reads {:?} under the same \
                     lineage {lineage}: renumbering a part a read has exposed starts a new \
                     lineage",
                    now.get(key)
                ));
            }
        }
    }
    Ok(())
}

/// Only parts rendered with a number are counted: the model's text and an
/// image-only tool result count, reasoning and a tool call never do.
async fn counted_parts<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let reasoning = case.mint.next("counted-reasoning");
    let (answer, answer_marker) = case.mint.text("counted-answer");
    let (last, _) = case.mint.text("counted-final");
    let mut call = case.mint.call("counted-call");
    let image_marker = case.mint.next("counted-image");
    let image = case.subject.image_tool_result(&image_marker);
    if let Some(result) = &image {
        call.result = result.clone();
    }
    let prompt = case.mint.next("counted-prompt");
    let session = session_with(
        case,
        "tags-counted",
        vec![
            ScriptedTurn::new(vec![
                ScriptedPart::Reasoning(reasoning.clone()),
                answer,
                ScriptedPart::ToolCall(call),
            ]),
            ScriptedTurn::new(vec![last]),
        ],
        std::slice::from_ref(&prompt),
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    check_tags("the transcript", &messages)?;
    let entry = entry_holding(&messages, &answer_marker)?;
    let numbered = numbers_of(entry);
    if numbered.len() != 1 || entry.tags.len() < 3 {
        return Err(format!(
            "the step holding reasoning, text and a tool call reads rows {:?}: only its text \
             counts, reasoning and the tool call carry no number",
            entry.tags
        ));
    }
    if !entry.message.to_string().contains(&reasoning) {
        return Err("the step's reasoning is not in its entry".into());
    }
    let Some(_) = image else {
        return Ok(Ending::PassedWithInapplicableRule(
            "an image-only tool result counts: the adapter's tool provider cannot answer one \
             (image_tool_result)"
                .into(),
        ));
    };
    let number = number_of(&messages, &image_marker, "the image-only tool result")?;
    if case.declared.contains(&Capability::RenderedModelInput) {
        let requests = case
            .subject
            .model_requests(&session.name)
            .await
            .map_err(|e| format!("observing the model requests: {e}"))?;
        let label = tags::image_label(number);
        if !requests
            .last()
            .is_some_and(|request| request.rendered.iter().any(|text| text.contains(&label)))
        {
            return Err(format!(
                "the last model request carried no label {label:?} for the image-only tool \
                 result: it renders a short text label carrying its number"
            ));
        }
    }
    Ok(Ending::Passed)
}

/// A compaction provider's Setup checkpoint at its cut: with no marker
/// carrying a checkpoint, the cut message, the session's first, is
/// numbered right after the checkpoint.
async fn setup_checkpoint<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    const CHECKPOINT: u64 = 500;
    let (answer, answer_marker) = case.mint.text("checkpoint-answer");
    let mut session = case
        .open(
            "tags-setup-checkpoint",
            Script {
                turns: vec![ScriptedTurn::new(vec![answer])],
            },
        )
        .await?;
    require_declared(&session.route, groups::TAG_CHECKPOINT, ops::SESSION_RESOLVE).await?;
    let id = case.mint.next("checkpoint-head");
    case.subject
        .install_compaction_setup(
            &session.name,
            crate::CompactionSetup::ReadyWithCheckpoint {
                initial: crate::CompactionMessage {
                    compaction_id: id.clone(),
                    version: 1,
                    from: 0,
                    to: 0,
                    replacement: vec![format!("{id}-head")],
                },
                tag_checkpoint: CHECKPOINT,
            },
        )
        .await
        .map_err(|e| format!("installing the scripted compaction provider: {e}"))?;
    let fields = rendering_fields(case.subject, &session.name)?;
    let prompt = case.mint.next("checkpoint-prompt");
    let reply = send(case, &mut session, Some(&fields), &prompt, &Map::new()).await?;
    case.subject
        .release_compaction_model(&session.name)
        .await
        .map_err(|e| format!("releasing the held model answer: {e}"))?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    let (_, messages) = read_all(&session.route).await?;
    for (marker, expected) in [(&prompt, CHECKPOINT + 1), (&answer_marker, CHECKPOINT + 2)] {
        let number = number_of(&messages, marker, "a counted part")?;
        if number != expected {
            return Err(format!(
                "the part holding {marker} is numbered {number}, not {expected}: with a Setup \
                 checkpoint of {CHECKPOINT} at the session's first message and no marker \
                 carrying one, numbering starts after the checkpoint"
            ));
        }
    }
    Ok(())
}

/// A session forked from a managed session is refused `fork_unsupported`
/// on `session.send` and, where declared, `session.adopt`, writing nothing.
/// Inapplicable unless the adapter can create a fork the runner can know
/// about: the kit never requires detecting a fork that carries nothing of
/// the runner's.
async fn fork_refused<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (answer, _) = case.mint.text("fork-parent-answer");
    let (fork_answer, _) = case.mint.text("fork-answer");
    let mut parent = case
        .open(
            "tags-fork-parent",
            Script {
                turns: vec![ScriptedTurn::new(vec![answer])],
            },
        )
        .await?;
    let id = case.mint.next("fork-head");
    case.subject
        .install_compaction_setup(
            &parent.name,
            crate::CompactionSetup::Ready(crate::CompactionMessage {
                compaction_id: id.clone(),
                version: 1,
                from: 0,
                to: 0,
                replacement: vec![format!("{id}-head")],
            }),
        )
        .await
        .map_err(|e| format!("installing the scripted compaction provider: {e}"))?;
    let fields = rendering_fields(case.subject, &parent.name)?;
    let prompt = case.mint.next("fork-parent-prompt");
    let reply = send(case, &mut parent, Some(&fields), &prompt, &Map::new()).await?;
    case.subject
        .release_compaction_model(&parent.name)
        .await
        .map_err(|e| format!("releasing the held model answer: {e}"))?;
    wait_run_end(case.subject, &parent.route, case.declared, None, &reply).await?;
    let fork = case
        .open(
            "tags-fork",
            Script {
                turns: vec![ScriptedTurn::new(vec![fork_answer])],
            },
        )
        .await?;
    if let Err(error) = case
        .subject
        .fork_session(case.handle.inner(), &parent.name, &fork.name)
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot create a fork of a managed session the runner can detect: \
             {error}"
        )));
    }
    let before = head(&fork.route).await?;
    let fields = case.subject.send_fields(&fork.name, true);
    let send_id = case.mint.next("send");
    let what = "session.send into a fork of a managed session";
    let error = expect_error(
        what,
        call(
            &fork.route,
            ops::SESSION_SEND,
            params(
                case,
                &fork,
                Some(&fields),
                &case.mint.next("fork-prompt"),
                &send_id,
                &Map::new(),
            ),
        )
        .await?,
    )?;
    expect_code(what, &error, errors::FORK_UNSUPPORTED)?;
    if let Some(detail) = &error.detail {
        decode::<ForkUnsupportedDetail>("the fork_unsupported detail", detail.clone())?;
    }
    if case.declared.contains(&Capability::Adopt) {
        if let Some(plan) = case.subject.host_plan(&fork.name, false) {
            let what = "session.adopt of a fork of a managed session";
            let request = json!({ "send_id": case.mint.next("adopt"), "plan": plan });
            let error = expect_error(what, call(&fork.route, ops::SESSION_ADOPT, request).await?)?;
            expect_code(what, &error, errors::FORK_UNSUPPORTED)?;
        }
    }
    let after = head(&fork.route).await?;
    if after.last_ordinal != before.last_ordinal || after.lineage_id != before.lineage_id {
        return Err(format!(
            "the refused fork's transcript moved from {before:?} to {after:?}: a refusal \
             writes nothing"
        ));
    }
    Ok(Ending::Passed)
}

/// The numbers a read shows, by message id and part key.
fn shown_numbers(messages: &[ReadMessage]) -> BTreeMap<(String, String), String> {
    messages
        .iter()
        .flat_map(|m| {
            m.tags
                .iter()
                .filter_map(move |row| Some(((m.mid.clone(), row.part.clone()), row.tag.clone()?)))
        })
        .collect()
}

/// A newer compaction marker that lost its checkpoint key falls back to the
/// older keyed marker and renumbers nothing: after the next step boundary
/// the lineage is the same and every part keeps the number it showed.
/// Inapplicable unless the adapter can compact the session in its host and
/// drop a marker's key, as OpenCode's part-update route does.
async fn lost_marker_key<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let turns = (0..4)
        .map(|_| ScriptedTurn::new(vec![case.mint.text("lost-key-answer").0]))
        .collect();
    let mut session = case.open("tags-lost-key", Script { turns }).await?;
    require_declared(&session.route, groups::TAGS, ops::SESSION_RESOLVE).await?;
    let fields = rendering_fields(case.subject, &session.name)?;
    for round in 0..2 {
        let prompt = case.mint.next("lost-key-prompt");
        let first = (round == 0).then_some(&fields);
        send_and_wait(case, &mut session, first, &prompt, &Map::new()).await?;
        if let Err(error) = case
            .subject
            .compact_in_host(case.handle.inner(), &session.name)
            .await
        {
            return Ok(Ending::Inapplicable(format!(
                "the adapter cannot compact a session in its host: {error}"
            )));
        }
    }
    let prompt = case.mint.next("lost-key-prompt");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (lineage, before) = lineage_read(&session.route).await?;
    check_tags("the transcript with every key present", &before)?;
    if let Err(error) = case
        .subject
        .drop_marker_key(case.handle.inner(), &session.name)
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot drop a compaction marker's checkpoint key: {error}"
        )));
    }
    let prompt = case.mint.next("lost-key-prompt");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (after_lineage, after) = lineage_read(&session.route).await?;
    if after_lineage != lineage {
        return Err(format!(
            "after the newest marker lost its key the session moved from lineage {lineage} to \
             {after_lineage}: the fallback counts every part from the older cut and renumbers \
             nothing"
        ));
    }
    check_tags("the transcript after the key was lost", &after)?;
    let now = shown_numbers(&after);
    for (key, tag) in shown_numbers(&before) {
        if now.get(&key) != Some(&tag) {
            return Err(format!(
                "part {key:?} showed {tag} with every key present and {:?} after the newest \
                 marker lost its key: the fallback counts every part from the older cut, the \
                 ones the newer marker summarized included",
                now.get(&key)
            ));
        }
    }
    Ok(Ending::Passed)
}

/// An in-process host cleans at its own save seam: the host's own store
/// holds a tag-rendering run's reply cleaned, and a structured-output
/// reply raw. Inapplicable unless the adapter can read the host's store.
async fn save_seam<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let marker = case.mint.next("seam-tag");
    let written = format!("§3§ {marker} as §1§ says");
    let session = session_with(
        case,
        "tags-save-seam",
        vec![ScriptedTurn::new(vec![ScriptedPart::Text(written.clone())])],
        &[case.mint.next("seam-prompt")],
    )
    .await?;
    let stored = match case
        .subject
        .host_stored_text(case.handle.inner(), &session.name, &marker)
        .await
    {
        Ok(stored) => stored,
        Err(error) => {
            return Ok(Ending::Inapplicable(format!(
                "the adapter cannot read the host's own store: {error}"
            )))
        }
    };
    let expected = format!("{marker} as  says");
    if stored.as_deref() != Some(expected.as_str()) {
        return Err(format!(
            "the host's own store holds the tag-rendering run's reply as {stored:?}, not \
             {expected:?}: an in-process host cleans at its save seam"
        ));
    }
    let marker = case.mint.next("seam-untagged");
    let written = format!("§3§ {marker} as §1§ says");
    let mut untagged = case
        .open(
            "seam-untagged",
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(written.clone())])],
            },
        )
        .await?;
    let fields = case
        .subject
        .send_fields_with_plan(&untagged.name, true, &Map::new())
        .ok_or("no untagged plan")?;
    send_and_wait(
        case,
        &mut untagged,
        Some(&fields),
        "untagged prompt",
        &Map::new(),
    )
    .await?;
    let stored = case
        .subject
        .host_stored_text(case.handle.inner(), &untagged.name, &marker)
        .await
        .map_err(|e| e.to_string())?;
    if stored.as_deref() != Some(written.as_str()) {
        return Err("the save seam cleaned an untagged writing run".into());
    }
    if !case.declared.contains(&Capability::StructuredOutput) {
        return Ok(Ending::PassedWithInapplicableRule(
            "a structured-output reply stays raw in the host's store: the runner does not \
             declare structured_output"
                .into(),
        ));
    }
    let (name, marker, written) = structured_output_raw(case).await?;
    let stored = case
        .subject
        .host_stored_text(case.handle.inner(), &name, &marker)
        .await
        .map_err(|e| format!("reading the host's store: {e}"))?;
    if stored.as_deref() != Some(written.as_str()) {
        return Err(format!(
            "the host's own store holds the structured-output reply as {stored:?}, not the raw \
             {written:?}: the save seam never cleans a structured-output reply"
        ));
    }
    Ok(Ending::Passed)
}
