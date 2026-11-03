//! The `host_history` cases: `session.history` pages through the session's
//! whole surviving history by opaque cursors (`cortexkit-role-llm-runner`
//! CONTRACT §4.6).
//!
//! Pages are read with a limit of two entries, so every walk crosses
//! several pages and every cursor is exercised.

use std::collections::BTreeSet;

use cortexkit_role_llm_runner::{
    errors,
    history::{HistoryEntry, HistoryPage, HistoryRequest},
    ops,
    read::{fields, ReadMessage},
};
use subc_protocol::ErrorBody;

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};

/// Entries per history page in every case.
enum Fixture<T> {
    Ready(T),
    Inapplicable(String),
}

const PAGE: u64 = 2;

/// How many pages a walk follows before it gives up.
const MAX_PAGES: usize = 1_000;

/// Host entries the host-prefix cases seed before the runner's first
/// numbered message.
const HOST_PREFIX: u64 = 3;

type Check = Result<(), String>;

async fn history_reply<R: RunnerRoute>(
    route: &R,
    request: &HistoryRequest,
) -> Result<crate::route::Reply, String> {
    call(
        route,
        ops::SESSION_HISTORY,
        serde_json::to_value(request).expect("a history request serializes"),
    )
    .await
}

/// One `session.history` page, checked against its page-local envelope rules.
async fn history<R: RunnerRoute>(
    route: &R,
    request: &HistoryRequest,
) -> Result<HistoryPage, String> {
    let body = expect_ok("session.history", history_reply(route, request).await?)?;
    let page: HistoryPage = decode("session.history", body)?;
    page.check()
        .map_err(|problem| format!("a session.history page breaks its own rules: {problem:?}"))?;
    Ok(page)
}

async fn history_refusal<R: RunnerRoute>(
    route: &R,
    what: &str,
    request: &HistoryRequest,
) -> Result<ErrorBody, String> {
    expect_error(what, history_reply(route, request).await?)
}

/// Every entry, from the newest page back through `older`, oldest first.
/// Checks that no `mid` repeats across pages, that a page with `older` is
/// never empty, and that the newest page carries no `newer`.
pub(crate) async fn walk_older<R: RunnerRoute>(route: &R) -> Result<Vec<HistoryEntry>, String> {
    let mut page = history(route, &HistoryRequest::newest().with_limit(PAGE)).await?;
    if page.newer.is_some() {
        return Err("the newest history page carries a newer cursor".into());
    }
    let mut pages = vec![page.messages.clone()];
    for _ in 0..MAX_PAGES {
        let Some(older) = page.older.clone() else {
            let entries: Vec<HistoryEntry> = pages.into_iter().rev().flatten().collect();
            unique("following older", &entries)?;
            return Ok(entries);
        };
        page = history(route, &HistoryRequest::cursor(older).with_limit(PAGE)).await?;
        if page.messages.is_empty() {
            return Err("an older cursor answered an empty page".into());
        }
        pages.push(page.messages.clone());
    }
    Err(format!("history did not end within {MAX_PAGES} pages"))
}

/// Every entry from `from_mid` forward through `newer`.
async fn walk_newer<R: RunnerRoute>(
    route: &R,
    from_mid: &str,
) -> Result<Vec<HistoryEntry>, String> {
    let mut page = history(route, &HistoryRequest::from_mid(from_mid).with_limit(PAGE)).await?;
    let mut entries = page.messages.clone();
    for _ in 0..MAX_PAGES {
        let Some(newer) = page.newer.clone() else {
            unique("following newer", &entries)?;
            return Ok(entries);
        };
        page = history(route, &HistoryRequest::cursor(newer).with_limit(PAGE)).await?;
        if page.messages.is_empty() {
            return Err("a newer cursor answered an empty page".into());
        }
        entries.extend(page.messages.iter().cloned());
    }
    Err(format!("history did not end within {MAX_PAGES} pages"))
}

fn unique(what: &str, entries: &[HistoryEntry]) -> Check {
    let mut seen = BTreeSet::new();
    for entry in entries {
        if !seen.insert(entry.mid.as_str()) {
            return Err(format!("{what}: mid {} is served twice", entry.mid));
        }
    }
    Ok(())
}

/// `entries` end with exactly the messages of `window`, in order, and
/// any entries preceding that selection have lower ordinals. Both read
/// operations can reach those older active-path entries.
fn ends_with_window(what: &str, entries: &[HistoryEntry], window: &[ReadMessage]) -> Check {
    let window_mids: Vec<&str> = window.iter().map(|m| m.mid.as_str()).collect();
    let served = mids(entries);
    let start = window.first().map_or(u64::MAX, |m| m.ordinal);
    let split = served.len().checked_sub(window_mids.len());
    match split {
        Some(split)
            if served[split..] == window_mids[..]
                && entries[..split]
                    .iter()
                    .all(|e| e.ordinal < start) => {}
        _ => {
            return Err(format!(
                "{what}, history served {served:?}; it must end with the session's window                  {window_mids:?}, with only older history before it"
            ))
        }
    }
    Ok(())
}

fn mids(entries: &[HistoryEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.mid.as_str()).collect()
}

/// A plain session with `host` seeded host entries before its first send
/// and `exchanges` prompt-and-answer exchanges, or why the adapter cannot
/// build it.
async fn written<S>(
    case: &Case<'_, S>,
    label: &str,
    host: u64,
    exchanges: usize,
) -> Result<Fixture<Session<S::Route>>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let script = Script {
        turns: (0..exchanges)
            .map(|n| ScriptedTurn::new(vec![ScriptedPart::Text(format!("{label}-answer-{n}"))]))
            .collect(),
    };
    let mut session = case.open(label, script).await?;
    if host > 0 {
        if let Err(e) = case
            .subject
            .seed_host_history(case.handle.inner(), &session.name, host, false)
            .await
        {
            return Ok(Fixture::Inapplicable(format!(
                "the adapter cannot give a session host history before the runner's first \
                 numbered message: {e}"
            )));
        }
    }
    for n in 0..exchanges {
        case.run_prompt(&mut session, &format!("{label}-prompt-{n}"))
            .await?;
    }
    Ok(Fixture::Ready(session))
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let label = case.mint.next("history");
    let outcome = match name {
        "history_pages_back_to_the_first_message" => pages_back(case, &label).await?,
        "history_mid_matches_read" => mid_matches_read(case, &label).await?,
        "history_never_serves_a_reverted_branch" => never_reverted(case, &label).await?,
        "history_cursor_expired_after_a_revert_past_it" => cursor_expired(case, &label).await?,
        "history_boundary_order" => boundary_order(case, &label).await?,
        "history_before_mid_reaches_host_prefix" => before_mid(case, &label).await?,
        "history_version_rises_at_a_revert" => version_at_revert(case, &label).await?,
        "history_from_ordinal_and_before_ordinal" => ordinal_seeks(case, &label).await?,
        "history_unknown_ordinal_refused" => unknown_ordinal(case, &label).await?,
        "history_before_ordinal_reaches_host_prefix" => before_lowest(case, &label).await?,
        other => return Err(format!("unknown history case {other}")),
    };
    Ok(match outcome {
        Fixture::Ready(()) => Ending::Passed,
        Fixture::Inapplicable(reason) => Ending::Inapplicable(reason),
    })
}

/// From the newest page, `older` reaches the first message, in order, with
/// no gap or duplicate; `newer` from the first message gives exactly the
/// same sequence; a second walk serves the same bytes.
async fn pages_back<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, 0, 3).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let route = &session.route;
    let (_, read) = read_all(route).await?;
    let back = walk_older(route).await?;
    ends_with_window("following older from the newest page", &back, &read)?;
    let forward = walk_newer(route, &back[0].mid).await?;
    if forward != back {
        return Err(format!(
            "following newer from the first message served {:?}, not the {:?} older served",
            mids(&forward),
            mids(&back)
        ));
    }
    if walk_older(route).await? != back {
        return Err("a second walk back served other entries or other bytes".into());
    }

    Ok(Fixture::Ready(()))
}

/// Every message both ops serve has the same `mid`, ordinal, bytes and served part identities
/// in both.
async fn mid_matches_read<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, 0, 2).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let (_, read) = read_all(&session.route).await?;
    let entries = walk_older(&session.route).await?;
    compare_window(&read, &entries)?;
    Ok(Fixture::Ready(()))
}

/// Each read entry is served by history with the same `mid`, ordinal, bytes
/// and served part identities.
fn compare_window(read: &[ReadMessage], entries: &[HistoryEntry]) -> Check {
    for message in read {
        match entries.iter().find(|entry| entry.mid == message.mid) {
            Some(entry) if entry.matches_read(message) => {}
            Some(entry) => {
                return Err(format!(
                    "message {} at ordinal {} is served differently by session.history \
                     ({entry:?}) and session.read ({message:?})",
                    message.mid, message.ordinal
                ))
            }
            None => {
                return Err(format!(
                    "session.read serves mid {} at ordinal {}, which session.history never \
                     serves under that mid",
                    message.mid, message.ordinal
                ))
            }
        }
    }
    Ok(())
}

/// Revert the message `mid` and everything after it, or why the adapter
/// cannot.
async fn revert<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    mid: &str,
) -> Result<Option<String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match case
        .subject
        .revert_session(case.handle.inner(), &session.name, mid)
        .await
    {
        Ok(()) => Ok(None),
        Err(e) => Ok(Some(format!("the adapter cannot revert a session: {e}"))),
    }
}

/// After a revert, history serves the surviving prefix and what followed
/// it, never the branch the revert left.
async fn never_reverted<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = match written(case, label, 0, 3).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let (_, before) = read_all(&session.route).await?;
    let left: Vec<String> = before[4..].iter().map(|m| m.mid.clone()).collect();
    if let Some(reason) = revert(
        case,
        &session,
        &before
            .get(4)
            .ok_or("the history fixture omitted a scripted entry")?
            .mid,
    )
    .await?
    {
        return Ok(Fixture::Inapplicable(reason));
    }
    case.run_prompt(&mut session, &format!("{label}-after-revert"))
        .await?;
    let (_, now) = read_all(&session.route).await?;
    let entries = walk_older(&session.route).await?;
    if let Some(entry) = entries.iter().find(|e| left.contains(&e.mid)) {
        return Err(format!(
            "history serves {}, a message the revert left",
            entry.mid
        ));
    }
    ends_with_window("after a revert", &entries, &now)?;
    Ok(Fixture::Ready(()))
}

/// A revert that removes a cursor's entry refuses the cursor
/// `cursor_expired`, never an empty page.
async fn cursor_expired<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, 0, 3).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let route = &session.route;
    let (_, before) = read_all(route).await?;
    let newest = history(route, &HistoryRequest::newest().with_limit(PAGE)).await?;
    let first_older = newest
        .older
        .ok_or("a six-message session has one page only")?;
    let middle = history(
        route,
        &HistoryRequest::cursor(&first_older).with_limit(PAGE),
    )
    .await?;
    let second_older = middle
        .older
        .ok_or("a six-message session has two pages only")?;
    // Remove the middle page's oldest entry, which minted `second_older`,
    // and everything after it.
    let oldest_of_middle = middle
        .messages
        .first()
        .ok_or("the middle history page is empty")?;
    if oldest_of_middle.mid != before[2].mid {
        return Err(format!(
            "the middle two-entry page starts at {}, not at the third message {}",
            oldest_of_middle.mid, before[2].mid
        ));
    }
    if let Some(reason) = revert(
        case,
        &session,
        &before
            .get(2)
            .ok_or("the history fixture omitted a scripted entry")?
            .mid,
    )
    .await?
    {
        return Ok(Fixture::Inapplicable(reason));
    }
    for (what, cursor) in [
        ("a cursor minted next to a reverted entry", &second_older),
        ("a cursor minted on a page the revert removed", &first_older),
    ] {
        let reply = history_reply(route, &HistoryRequest::cursor(cursor).with_limit(PAGE)).await?;
        let error = expect_error(what, reply)?;
        expect_code(what, &error, errors::CURSOR_EXPIRED)?;
    }
    Ok(Fixture::Ready(()))
}

/// The seeded host prefix is part of the same one-based active-path count.
async fn boundary_order<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, HOST_PREFIX, 2).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let (_, read) = read_all(&session.route).await?;
    let entries = walk_older(&session.route).await?;
    if entries.len() < HOST_PREFIX as usize
        || mids(&entries)
            != read
                .iter()
                .map(|entry| entry.mid.as_str())
                .collect::<Vec<_>>()
    {
        return Err("the seeded host prefix is not in the same order as full-path reads".into());
    }
    let expected: Vec<u64> = (1..=entries.len() as u64).collect();
    if ordinals(&entries) != expected {
        return Err(format!(
            "host history must count its prefix from 1 without gaps: {:?}",
            ordinals(&entries)
        ));
    }
    Ok(Fixture::Ready(()))
}

/// Backward paging from the first post-seed entry reaches the whole counted host prefix.
async fn before_mid<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, HOST_PREFIX, 1).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let route = &session.route;
    let (_, all) = read_all(route).await?;
    let first_new = all
        .get(HOST_PREFIX as usize)
        .ok_or("the fixture omitted a post-seed entry")?;
    let mut page = history(
        route,
        &HistoryRequest::before_mid(&first_new.mid).with_limit(PAGE),
    )
    .await?;
    let mut entries = page.messages.clone();
    if entries.is_empty() || page.older.is_none() {
        return Err("before_mid omitted the host prefix or its older cursor".into());
    }
    for _ in 0..MAX_PAGES {
        let Some(older) = page.older.clone() else {
            break;
        };
        page = history(route, &HistoryRequest::cursor(older).with_limit(PAGE)).await?;
        let mut prefix = page.messages.clone();
        prefix.extend(entries);
        entries = prefix;
    }
    let expected = all[..HOST_PREFIX as usize]
        .iter()
        .map(|entry| entry.mid.as_str())
        .collect::<Vec<_>>();
    if mids(&entries) != expected || ordinals(&entries) != (1..=HOST_PREFIX).collect::<Vec<_>>() {
        return Err("backward paging did not reach the complete one-based host prefix".into());
    }
    unique("following older into the host prefix", &entries)?;
    Ok(Fixture::Ready(()))
}

/// The history_version `session.head` states and the newest history page
/// carries; they must agree.
async fn versions<R: RunnerRoute>(route: &R) -> Result<u64, String> {
    let on_head = head(route).await?.history_version;
    let on_page = history(route, &HistoryRequest::newest().with_limit(PAGE))
        .await?
        .history_version;
    if on_head != on_page {
        return Err(format!(
            "session.head states history_version {on_head}, a history page {on_page}"
        ));
    }
    Ok(on_head)
}

/// A revert raises the history_version.
async fn version_at_revert<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, 0, 2).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let before = versions(&session.route).await?;
    let (_, read) = read_all(&session.route).await?;
    if let Some(reason) = revert(
        case,
        &session,
        &read
            .get(2)
            .ok_or("the history fixture omitted a scripted entry")?
            .mid,
    )
    .await?
    {
        return Ok(Fixture::Inapplicable(reason));
    }
    let after = versions(&session.route).await?;
    if after <= before {
        return Err(format!(
            "a revert left history_version at {after} (it was {before}); it rises at every \
             rewrite of the visible history"
        ));
    }
    Ok(Fixture::Ready(()))
}

fn ordinals(entries: &[HistoryEntry]) -> Vec<u64> {
    entries.iter().map(|e| e.ordinal).collect()
}

/// `from_ordinal` reads from its ordinal, inclusive, and `before_ordinal`
/// the page ending directly before its ordinal, exclusive; the returned
/// cursors continue paging. A request naming both is refused
/// `invalid_params` naming `before_ordinal`.
async fn ordinal_seeks<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, 0, 3).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let route = &session.route;
    let from = history(route, &HistoryRequest::from_ordinal(3).with_limit(PAGE)).await?;
    if ordinals(&from.messages) != [3, 4] {
        return Err(format!(
            "from_ordinal 3 served ordinals {:?}, not [3, 4]",
            ordinals(&from.messages)
        ));
    }
    let newer = from
        .newer
        .ok_or("from_ordinal 3 of six messages has no newer cursor")?;
    let next = history(route, &HistoryRequest::cursor(newer).with_limit(PAGE)).await?;
    if ordinals(&next.messages) != [5, 6] {
        return Err(format!(
            "the page after from_ordinal 3 served {:?}, not [5, 6]",
            ordinals(&next.messages)
        ));
    }
    let before = history(route, &HistoryRequest::before_ordinal(5).with_limit(PAGE)).await?;
    if ordinals(&before.messages) != [3, 4] {
        return Err(format!(
            "before_ordinal 5 served ordinals {:?}, not [3, 4]",
            ordinals(&before.messages)
        ));
    }
    let older = before.older.ok_or("before_ordinal 5 has no older cursor")?;
    let prior = history(route, &HistoryRequest::cursor(older).with_limit(PAGE)).await?;
    if ordinals(&prior.messages) != [1, 2] {
        return Err(format!(
            "the page before before_ordinal 5 served {:?}, not [1, 2]",
            ordinals(&prior.messages)
        ));
    }
    let mut both = HistoryRequest::from_ordinal(1);
    both.before_ordinal = Some(3);
    let error = history_refusal(route, "from_ordinal with before_ordinal", &both).await?;
    expect_field(
        "from_ordinal with before_ordinal",
        &error,
        errors::INVALID_PARAMS,
        fields::BEFORE_ORDINAL,
    )?;
    Ok(Fixture::Ready(()))
}

/// An ordinal never assigned, or one a revert removed, is refused
/// `unknown_ordinal`.
async fn unknown_ordinal<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, 0, 2).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let route = &session.route;
    let (_, read) = read_all(route).await?;
    let never = read.last().ok_or("an empty session")?.ordinal + 100;
    for request in [
        HistoryRequest::from_ordinal(never),
        HistoryRequest::before_ordinal(never),
    ] {
        let what = format!("an ordinal never assigned, in {request:?}");
        let error = history_refusal(route, &what, &request).await?;
        expect_code(&what, &error, errors::UNKNOWN_ORDINAL)?;
    }
    if revert(
        case,
        &session,
        &read
            .get(2)
            .ok_or("the history fixture omitted a scripted entry")?
            .mid,
    )
    .await?
    .is_none()
    {
        let what = "an ordinal a revert removed";
        let error = history_refusal(route, what, &HistoryRequest::from_ordinal(3)).await?;
        expect_code(what, &error, errors::UNKNOWN_ORDINAL)?;
    }
    Ok(Fixture::Ready(()))
}

/// An ordinal seek before the first post-seed entry pages through counted host history.
async fn before_lowest<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, HOST_PREFIX, 1).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let route = &session.route;
    let (_, all) = read_all(route).await?;
    let first_new = all
        .get(HOST_PREFIX as usize)
        .ok_or("the fixture omitted a post-seed entry")?;
    let page = history(
        route,
        &HistoryRequest::before_ordinal(first_new.ordinal).with_limit(PAGE),
    )
    .await?;
    let start = (HOST_PREFIX as usize).saturating_sub(PAGE as usize);
    let expected = all[start..HOST_PREFIX as usize]
        .iter()
        .map(|entry| entry.mid.as_str())
        .collect::<Vec<_>>();
    if mids(&page.messages) != expected || page.older.is_none() {
        return Err(
            "before_ordinal did not serve the newest counted host prefix with an older cursor"
                .into(),
        );
    }
    let first = history(route, &HistoryRequest::before_ordinal(1).with_limit(PAGE)).await?;
    if !first.messages.is_empty() || first.older.is_some() {
        return Err("before the first ordinal must be empty with no older cursor".into());
    }
    Ok(Fixture::Ready(()))
}
