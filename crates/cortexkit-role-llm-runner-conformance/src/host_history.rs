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
    read::{fields, ReadMessage, ReadView},
};
use subc_protocol::ErrorBody;

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all},
    lineage::{self, Fixture, CUT_FIRST_ORDINAL},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};

/// Entries per history page in every case.
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

/// One `session.history` page, checked on its own against the current
/// lineage's `first_ordinal`.
async fn history<R: RunnerRoute>(
    route: &R,
    request: &HistoryRequest,
) -> Result<HistoryPage, String> {
    let body = expect_ok("session.history", history_reply(route, request).await?)?;
    let page: HistoryPage = decode("session.history", body)?;
    let first = head(route).await?.first_ordinal;
    page.check(first)
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
/// everything before them is history `session.read` does not serve: host
/// entries without an ordinal, or entries below the window.
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
                    .all(|e| e.ordinal.is_none_or(|ordinal| ordinal < start)) => {}
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
        "history_no_tags_below_first_ordinal" => no_tags_below(case, &label).await?,
        "history_ordinal_kept_below_the_cut" => ordinal_kept(case, &label).await?,
        "history_cursor_survives_a_cut" => cursor_survives_cut(case, &label).await?,
        "history_never_serves_a_reverted_branch" => never_reverted(case, &label).await?,
        "history_cursor_expired_after_a_revert_past_it" => cursor_expired(case, &label).await?,
        "history_boundary_order" => boundary_order(case, &label).await?,
        "history_before_mid_continues_below_the_window" => before_mid(case, &label).await?,
        "history_version_unchanged_by_a_cut" => version_at_cut(case, &label).await?,
        "history_version_rises_at_a_revert" => version_at_revert(case, &label).await?,
        "history_from_ordinal_and_before_ordinal" => ordinal_seeks(case, &label).await?,
        "history_unknown_ordinal_refused" => unknown_ordinal(case, &label).await?,
        "history_before_lowest_ordinal_reaches_unnumbered_entries" => {
            before_lowest(case, &label).await?
        }
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
    let error = history_refusal(
        route,
        "a model-view history request",
        &HistoryRequest::newest().with_view(ReadView::Model),
    )
    .await?;
    expect_field(
        "a model-view history request",
        &error,
        errors::INVALID_PARAMS,
        fields::VIEW,
    )?;
    Ok(Fixture::Ready(()))
}

/// Every message both ops serve has the same `mid`, ordinal, bytes and tags
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
/// and tags.
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

/// After a cut, entries below `first_ordinal` carry no tags, and window
/// entries carry exactly what `session.read` serves.
async fn no_tags_below<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let fixture = match lineage::cut_session(case, label, 3).await? {
        Fixture::Ready(fixture) => fixture,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    if fixture.before.iter().all(|m| m.tags.is_empty()) {
        return Err(
            "session.read served no tag rows before the cut, though compaction \
                    requires the tags group"
                .into(),
        );
    }
    let route = &fixture.session.route;
    let entries = walk_older(route).await?;
    for entry in &entries {
        let below = entry
            .ordinal
            .is_none_or(|ordinal| ordinal < CUT_FIRST_ORDINAL);
        if below && !entry.tags.is_empty() {
            return Err(format!(
                "history entry {} at ordinal {:?}, below first_ordinal {CUT_FIRST_ORDINAL}, \
                 carries tags {:?}",
                entry.mid, entry.ordinal, entry.tags
            ));
        }
    }
    let (_, read) = read_all(route).await?;
    compare_window(&read, &entries)?;
    Ok(Fixture::Ready(()))
}

/// An entry read with an ordinal before a cut carries the same ordinal from
/// history after it.
async fn ordinal_kept<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let fixture = match lineage::cut_session(case, label, 3).await? {
        Fixture::Ready(fixture) => fixture,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let entries = walk_older(&fixture.session.route).await?;
    for before in &fixture.before {
        match entries.iter().find(|entry| entry.mid == before.mid) {
            Some(entry) if entry.ordinal == Some(before.ordinal) => {}
            Some(entry) => {
                return Err(format!(
                    "message {} was read at ordinal {} before the cut; history serves it with \
                     ordinal {:?}",
                    before.mid, before.ordinal, entry.ordinal
                ))
            }
            None => {
                return Err(format!(
                    "message {} at ordinal {}, read before the cut, is missing from history",
                    before.mid, before.ordinal
                ))
            }
        }
    }
    Ok(Fixture::Ready(()))
}

/// A cursor minted before a cut continues after it, with no overlap or gap.
async fn cursor_survives_cut<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let written = lineage::written(case, label, 3).await?;
    let newest = history(
        &written.session.route,
        &HistoryRequest::newest().with_limit(PAGE),
    )
    .await?;
    let older = newest
        .older
        .clone()
        .ok_or("a four-message session's newest two-entry page has no older cursor")?;
    let expected: Vec<String> = written.before[..2].iter().map(|m| m.mid.clone()).collect();
    if mids(&newest.messages)
        != written.before[2..]
            .iter()
            .map(|m| m.mid.as_str())
            .collect::<Vec<_>>()
    {
        return Err(format!(
            "the newest history page serves {:?}, not the two newest messages",
            mids(&newest.messages)
        ));
    }
    let fixture = match lineage::apply_cut(case, written, label).await? {
        Fixture::Ready(fixture) => fixture,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let reply = history_reply(
        &fixture.session.route,
        &HistoryRequest::cursor(older).with_limit(PAGE),
    )
    .await?;
    let page: HistoryPage = decode(
        "session.history after a cut",
        expect_ok("a cursor minted before a cut, used after it", reply)?,
    )?;
    if mids(&page.messages) != expected.iter().map(String::as_str).collect::<Vec<_>>() {
        return Err(format!(
            "a cursor minted before a cut served {:?} after it, not the {expected:?} directly \
             before the page that minted it",
            mids(&page.messages)
        ));
    }
    Ok(Fixture::Ready(()))
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
    if let Some(reason) = revert(case, &session, &before[4].mid).await? {
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
    if let Some(reason) = revert(case, &session, &before[2].mid).await? {
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

/// With host history before the runner's first numbered message, the
/// newest unnumbered entry comes directly before the lowest ordinal.
async fn boundary_order<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = match written(case, label, HOST_PREFIX, 2).await? {
        Fixture::Ready(session) => session,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let entries = walk_older(&session.route).await?;
    let unnumbered = entries.iter().filter(|e| e.ordinal.is_none()).count();
    if unnumbered as u64 != HOST_PREFIX {
        return Err(format!(
            "history serves {unnumbered} entries without an ordinal; the host held \
             {HOST_PREFIX} before the runner's first numbered message"
        ));
    }
    let lowest = entries
        .iter()
        .filter_map(|e| e.ordinal)
        .min()
        .ok_or("history serves no numbered entry")?;
    let at = entries
        .iter()
        .position(|e| e.ordinal == Some(lowest))
        .expect("the lowest ordinal is served");
    if at != unnumbered || entries[..at].iter().any(|e| e.ordinal.is_some()) {
        return Err(format!(
            "the entry with the lowest ordinal {lowest} is at position {at}, not directly after \
             the {unnumbered} entries without an ordinal: {:?}",
            entries.iter().map(|e| e.ordinal).collect::<Vec<_>>()
        ));
    }
    Ok(Fixture::Ready(()))
}

/// `before_mid` of the window's first message continues into the host
/// history before it, and paging `older` ends at the first entry.
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
    let (_, window) = read_all(route).await?;
    let first = window.first().ok_or("the window is empty")?;
    if first.ordinal != 0 || head(route).await?.first_ordinal != Some(0) {
        return Err(format!(
            "an adopted session's window should start at ordinal 0; it starts at {}",
            first.ordinal
        ));
    }
    let mut page = history(
        route,
        &HistoryRequest::before_mid(&first.mid).with_limit(PAGE),
    )
    .await?;
    let mut entries = page.messages.clone();
    if entries.is_empty() || page.older.is_none() {
        return Err(format!(
            "before_mid of the window's first message answered {} entries and older {:?}, \
             though the host holds {HOST_PREFIX} entries before it",
            entries.len(),
            page.older
        ));
    }
    for _ in 0..MAX_PAGES {
        let Some(older) = page.older.clone() else {
            break;
        };
        page = history(route, &HistoryRequest::cursor(older).with_limit(PAGE)).await?;
        let mut older_entries = page.messages.clone();
        older_entries.extend(entries);
        entries = older_entries;
    }
    if entries.len() as u64 != HOST_PREFIX || entries.iter().any(|e| e.ordinal.is_some()) {
        return Err(format!(
            "below the window, history served {:?}; the host held {HOST_PREFIX} entries \
             without ordinals",
            entries
                .iter()
                .map(|e| (&e.mid, e.ordinal))
                .collect::<Vec<_>>()
        ));
    }
    unique("following older below the window", &entries)?;
    Ok(Fixture::Ready(()))
}

/// The history version `session.head` states and the newest history page
/// carries; they must agree.
async fn versions<R: RunnerRoute>(route: &R) -> Result<u64, String> {
    let on_head = head(route)
        .await?
        .history_version
        .ok_or("session.head carries no history_version")?;
    let on_page = history(route, &HistoryRequest::newest().with_limit(PAGE))
        .await?
        .history_version
        .ok_or("a session.history page carries no history_version")?;
    if on_head != on_page {
        return Err(format!(
            "session.head states history_version {on_head}, a history page {on_page}"
        ));
    }
    Ok(on_head)
}

/// A cut, and the exchange after it, leave the history version as it was.
async fn version_at_cut<S>(case: &Case<'_, S>, label: &str) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let written = lineage::written(case, label, 3).await?;
    let before = versions(&written.session.route).await?;
    let fixture = match lineage::apply_cut(case, written, label).await? {
        Fixture::Ready(fixture) => fixture,
        Fixture::Inapplicable(reason) => return Ok(Fixture::Inapplicable(reason)),
    };
    let after = versions(&fixture.session.route).await?;
    if after != before {
        return Err(format!(
            "a cut moved history_version from {before} to {after}; only a rewrite of the \
             visible history moves it"
        ));
    }
    Ok(Fixture::Ready(()))
}

/// A revert raises the history version.
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
    if let Some(reason) = revert(case, &session, &read[2].mid).await? {
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

fn ordinals(entries: &[HistoryEntry]) -> Vec<Option<u64>> {
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
    let from = history(route, &HistoryRequest::from_ordinal(2).with_limit(PAGE)).await?;
    if ordinals(&from.messages) != [Some(2), Some(3)] {
        return Err(format!(
            "from_ordinal 2 served ordinals {:?}, not [2, 3]",
            ordinals(&from.messages)
        ));
    }
    let newer = from
        .newer
        .ok_or("from_ordinal 2 of six messages has no newer cursor")?;
    let next = history(route, &HistoryRequest::cursor(newer).with_limit(PAGE)).await?;
    if ordinals(&next.messages) != [Some(4), Some(5)] {
        return Err(format!(
            "the page after from_ordinal 2 served {:?}, not [4, 5]",
            ordinals(&next.messages)
        ));
    }
    let before = history(route, &HistoryRequest::before_ordinal(4).with_limit(PAGE)).await?;
    if ordinals(&before.messages) != [Some(2), Some(3)] {
        return Err(format!(
            "before_ordinal 4 served ordinals {:?}, not [2, 3]",
            ordinals(&before.messages)
        ));
    }
    let older = before.older.ok_or("before_ordinal 4 has no older cursor")?;
    let prior = history(route, &HistoryRequest::cursor(older).with_limit(PAGE)).await?;
    if ordinals(&prior.messages) != [Some(0), Some(1)] {
        return Err(format!(
            "the page before before_ordinal 4 served {:?}, not [0, 1]",
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
    if revert(case, &session, &read[2].mid).await?.is_none() {
        let what = "an ordinal a revert removed";
        let error = history_refusal(route, what, &HistoryRequest::from_ordinal(3)).await?;
        expect_code(what, &error, errors::UNKNOWN_ORDINAL)?;
    }
    Ok(Fixture::Ready(()))
}

/// `before_ordinal` at the lowest ordinal returns the newest host-history
/// entries, which have no ordinal.
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
    let page = history(route, &HistoryRequest::before_ordinal(0).with_limit(PAGE)).await?;
    if page.messages.len() as u64 != PAGE.min(HOST_PREFIX)
        || page.messages.iter().any(|e| e.ordinal.is_some())
        || page.older.is_none()
    {
        return Err(format!(
            "before_ordinal 0 served {:?} with older {:?}; the host holds {HOST_PREFIX} \
             entries without an ordinal before it",
            ordinals(&page.messages),
            page.older
        ));
    }
    let all = walk_older(route).await?;
    let host: Vec<&str> = all
        .iter()
        .filter(|e| e.ordinal.is_none())
        .map(|e| e.mid.as_str())
        .collect();
    let newest_host = host[host.len().saturating_sub(PAGE as usize)..].to_vec();
    if mids(&page.messages) != newest_host {
        return Err(format!(
            "before_ordinal 0 served {:?}, not the newest entries without an ordinal {newest_host:?}",
            mids(&page.messages)
        ));
    }
    Ok(Fixture::Ready(()))
}
