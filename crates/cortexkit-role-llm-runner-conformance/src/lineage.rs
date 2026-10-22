//! The lineage cases: a compaction view with a cut starts a new lineage
//! (`cortexkit-role-llm-runner` CONTRACT §4.5), and the reads, Setup, views
//! and notices follow it.
//!
//! Every case builds the same fixture: a compaction session with two
//! exchanges (ordinals 0 to 3), a view that folds ordinals 0 and 1 and cuts
//! through ordinal 1, and a third exchange whose first step applies the view.
//! The new lineage then starts at ordinal 2. The fixture needs the adapter's
//! view-publishing hook; without it every case is inapplicable, by name.

use std::collections::BTreeMap;

use cortexkit_role_compaction_provider::notice::{AdvanceNotice, Event};
use cortexkit_role_llm_runner::{
    content_digest::content_digest,
    errors,
    read::{ReadMessage, ReadRequest},
};

use crate::{
    cases::{Case, Ending, Session},
    compaction::{self, CompactionMessage, CompactionSetup, ScriptedRange, ScriptedView},
    drive::{decode, expect_code, head, read, read_all, read_refusal},
    subject::{LlmRunnerSubject, Script},
    RunnerRoute,
};

/// The ordinal the cut lineage starts at: the cut covers ordinals 0 and 1.
pub(crate) const CUT_FIRST_ORDINAL: u64 = 2;

/// A session whose view with a cut has applied.
pub(crate) struct CutSession<R> {
    pub session: Session<R>,
    /// The lineage before the cut.
    pub old_lineage: String,
    /// Every message `session.read` served on the old lineage, read before
    /// the cut.
    pub before: Vec<ReadMessage>,
}

/// What a fixture step could not do: an adapter without the hook makes the
/// case inapplicable, anything else fails it.
pub(crate) enum Fixture<T> {
    Ready(T),
    Inapplicable(String),
}

/// Send `prompt` into a compaction session, release its one held model
/// answer and wait for the run to end.
pub(crate) async fn exchange<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    prompt: &str,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let reply = compaction::send(case, session, prompt).await?;
    compaction::release_model(case.subject, &session.name).await?;
    compaction::result(case, session, &reply).await?;
    Ok(())
}

/// A compaction session with two exchanges on its first lineage, ordinals 0
/// to 3, read before any cut.
pub(crate) struct Written<R> {
    pub session: Session<R>,
    pub old_lineage: String,
    pub before: Vec<ReadMessage>,
}

/// Two exchanges on a fresh compaction session, with `turns` scripted
/// answers in all (at least three: the third answers the exchange that
/// applies the cut).
pub(crate) async fn written<S>(
    case: &Case<'_, S>,
    label: &str,
    turns: usize,
) -> Result<Written<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    written_with(case, label, turns, false).await
}

/// [`written`], on a session whose first send's plan renders tags when
/// `render_tags` is set.
pub(crate) async fn written_with<S>(
    case: &Case<'_, S>,
    label: &str,
    turns: usize,
    render_tags: bool,
) -> Result<Written<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let script = Script {
        turns: (0..turns.max(3))
            .map(|n| compaction::text(&format!("{label}-answer-{n}")))
            .collect(),
    };
    let mut session = compaction::install(
        case,
        label,
        CompactionSetup::Ready(compaction::initial(&case.mint.next(label))),
        script,
    )
    .await?;
    let fields = if render_tags {
        Some(crate::tags::rendering_fields(case.subject, &session.name)?)
    } else {
        None
    };
    match fields {
        Some(fields) => {
            let prompt = format!("{label}-prompt-0");
            let id = case.mint.next("send");
            let params = crate::tags::params(
                case,
                &session,
                Some(&fields),
                &prompt,
                &id,
                &serde_json::Map::new(),
            );
            let reply: cortexkit_role_llm_runner::send::SendReply = decode(
                "the first send",
                crate::drive::expect_ok(
                    "the first send",
                    crate::drive::call(
                        &session.route,
                        cortexkit_role_llm_runner::ops::SESSION_SEND,
                        params,
                    )
                    .await?,
                )?,
            )?;
            session.first = Some((id, prompt));
            compaction::release_model(case.subject, &session.name).await?;
            compaction::result(case, &session, &reply).await?;
        }
        None => exchange(case, &mut session, &format!("{label}-prompt-0")).await?,
    }
    exchange(case, &mut session, &format!("{label}-prompt-1")).await?;
    let (_, before) = read_all(&session.route).await?;
    if before.len() != 4 || before.iter().map(|m| m.ordinal).ne(0..4) {
        return Err(format!(
            "the cut fixture needs ordinals 0 to 3 after two exchanges, got {:?}",
            before.iter().map(|m| m.ordinal).collect::<Vec<_>>()
        ));
    }
    let old_lineage = head(&session.route)
        .await?
        .lineage_id
        .ok_or("a written session has no lineage")?;
    Ok(Written {
        session,
        old_lineage,
        before,
    })
}

/// Publish a view over `written` that folds ordinals 0 and 1 and cuts
/// through ordinal 1, then run a third exchange, whose first step applies
/// it. The provider answers the cut lineage's Setup with a head inserted at
/// the lineage's first ordinal.
pub(crate) async fn apply_cut<S>(
    case: &Case<'_, S>,
    written: Written<S::Route>,
    label: &str,
) -> Result<Fixture<CutSession<S::Route>>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let Written {
        mut session,
        old_lineage,
        before,
    } = written;
    let view = ScriptedView {
        lineage_id: old_lineage.clone(),
        covers_through: 3,
        covers_through_mid: before[3].mid.clone(),
        ranges: vec![
            ScriptedRange::Replace {
                from: 0,
                to: CUT_FIRST_ORDINAL,
                replacement: vec![format!("{label}-folded")],
            },
            ScriptedRange::Keep {
                from: CUT_FIRST_ORDINAL,
                to: 4,
            },
        ],
        cut: Some(CUT_FIRST_ORDINAL - 1),
    };
    if let Err(e) = case
        .subject
        .publish_compaction_view(&session.name, view)
        .await
    {
        return Ok(Fixture::Inapplicable(format!(
            "the adapter cannot publish a compaction view: {e}"
        )));
    }
    // The cut lineage's Setup answers a head at the lineage's first ordinal.
    case.subject
        .install_compaction_setup(
            &session.name,
            CompactionSetup::Ready(CompactionMessage {
                compaction_id: case.mint.next("cut-head"),
                version: 1,
                from: CUT_FIRST_ORDINAL,
                to: CUT_FIRST_ORDINAL,
                replacement: vec![format!("{label}-cut-head")],
            }),
        )
        .await
        .map_err(|e| e.to_string())?;
    exchange(case, &mut session, &format!("{label}-prompt-2")).await?;
    Ok(Fixture::Ready(CutSession {
        session,
        old_lineage,
        before,
    }))
}

/// [`written`], then [`apply_cut`].
pub(crate) async fn cut_session<S>(
    case: &Case<'_, S>,
    label: &str,
    turns: usize,
) -> Result<Fixture<CutSession<S::Route>>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let written = written(case, label, turns).await?;
    apply_cut(case, written, label).await
}

/// Every notice the runner published for `session`, decoded.
pub(crate) async fn notices<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
) -> Result<Vec<AdvanceNotice>, String> {
    compaction::observation(subject, session)
        .await?
        .notices
        .iter()
        .map(|headers: &BTreeMap<String, String>| {
            AdvanceNotice::from_headers(headers)
                .map_err(|e| format!("the runner published a malformed notice {headers:?}: {e:?}"))
        })
        .collect()
}

/// Whether a notice refused view `version` `ranges_start`.
fn refused_by(seen: &[AdvanceNotice], version: u64) -> bool {
    seen.iter().any(|notice| {
        notice.events.iter().any(|event| {
            matches!(event, Event::ViewRefused { view_version, reason }
                if *view_version == version
                    && reason == cortexkit_role_compaction_provider::notice::refusal_reasons::RANGES_START)
        })
    })
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "checkpoint_on_descending_setup_refused" => return checkpoint_on_descent(case).await,
        "cut_setup_leaves_reads_alone" => return reads_during_cut_setup(case).await,
        "cut_numbering_continues_from_the_old_lineage" => return numbering_continues(case).await,
        "revert_below_the_cut_restarts_at_the_previous_cut" => {
            return revert_to_previous_cut(case).await
        }
        "revert_below_a_cut_before_adoption_has_no_descent" => {
            return revert_to_host_cut(case).await
        }
        _ => {}
    }
    let label = case.mint.next("cut");
    let turns = if name == "view_ranges_start_at_first_ordinal" || name == "setup_once_per_lineage"
    {
        6
    } else {
        3
    };
    let fixture = match cut_session(case, &label, turns).await? {
        Fixture::Ready(fixture) => fixture,
        Fixture::Inapplicable(reason) => return Ok(Ending::Inapplicable(reason)),
    };
    match name {
        "cut_starts_a_new_lineage" => new_lineage(&fixture).await,
        "old_lineage_reads_refused_after_a_cut" => old_lineage_refused(&fixture).await,
        "read_below_first_ordinal_refused" => below_first_ordinal(&fixture).await,
        "ordinals_never_renumber_across_a_cut" => no_renumbering(&fixture).await,
        "setup_once_per_lineage" => setup_once(case, fixture, &label).await,
        "view_ranges_start_at_first_ordinal" => ranges_start(case, fixture, &label).await,
        "cut_lineage_notice_descends_from" => descends_from(case, &fixture).await,
        "head_names_the_cut" => head_names_cut(&fixture).await,
        "notice_carries_first_ordinal" => notice_first_ordinal(case, &fixture).await,
        other => Err(format!("unknown lineage case {other}")),
    }
    .map(|()| Ending::Passed)
}

async fn new_lineage<R: RunnerRoute>(fixture: &CutSession<R>) -> Result<(), String> {
    let now = head(&fixture.session.route).await?;
    if now.lineage_id.as_deref() == Some(fixture.old_lineage.as_str()) {
        return Err(format!(
            "a view with a cut applied, but the session kept lineage {}",
            fixture.old_lineage
        ));
    }
    if now.first_ordinal != Some(CUT_FIRST_ORDINAL) {
        return Err(format!(
            "after a cut through ordinal {}, first_ordinal is {:?}, not the cut's first retained \
             ordinal {CUT_FIRST_ORDINAL}",
            CUT_FIRST_ORDINAL - 1,
            now.first_ordinal
        ));
    }
    Ok(())
}

async fn old_lineage_refused<R: RunnerRoute>(fixture: &CutSession<R>) -> Result<(), String> {
    let route = &fixture.session.route;
    let range = ReadRequest::range(CUT_FIRST_ORDINAL).with_lineage_id(&fixture.old_lineage);
    let error = read_refusal(route, "a range read naming the lineage a cut left", &range).await?;
    expect_code(
        "a range read naming the lineage a cut left",
        &error,
        errors::LINEAGE_CHANGED,
    )?;
    let after = ReadRequest::after(&fixture.before[2].mid, &fixture.old_lineage);
    let error = read_refusal(
        route,
        "an after_mid read naming the lineage a cut left",
        &after,
    )
    .await?;
    expect_code(
        "an after_mid read naming the lineage a cut left",
        &error,
        errors::LINEAGE_CHANGED,
    )
}

/// A `before_lineage_start` refusal carrying `first_ordinal`.
fn expect_before_start(what: &str, error: &subc_protocol::ErrorBody) -> Result<(), String> {
    expect_code(what, error, errors::BEFORE_LINEAGE_START)?;
    let detail: errors::BeforeLineageStartDetail = decode(
        what,
        error.detail.clone().unwrap_or(serde_json::Value::Null),
    )?;
    if detail.first_ordinal != CUT_FIRST_ORDINAL {
        return Err(format!(
            "{what} names first_ordinal {}, not {CUT_FIRST_ORDINAL}",
            detail.first_ordinal
        ));
    }
    Ok(())
}

async fn below_first_ordinal<R: RunnerRoute>(fixture: &CutSession<R>) -> Result<(), String> {
    let route = &fixture.session.route;
    for from in 0..CUT_FIRST_ORDINAL {
        let what = format!("a range read from ordinal {from}, below first_ordinal");
        let error = read_refusal(route, &what, &ReadRequest::range(from)).await?;
        expect_before_start(&what, &error)?;
    }
    let lineage = head(route)
        .await?
        .lineage_id
        .ok_or("the cut session has no lineage")?;
    let what = "an after_mid read naming a message below first_ordinal";
    let error = read_refusal(
        route,
        what,
        &ReadRequest::after(&fixture.before[0].mid, &lineage),
    )
    .await?;
    expect_before_start(what, &error)?;
    let tail = read(route, &ReadRequest::tail()).await?;
    if let Some(message) = tail.messages.iter().find(|m| m.ordinal < CUT_FIRST_ORDINAL) {
        return Err(format!(
            "the tail served ordinal {}, below first_ordinal {CUT_FIRST_ORDINAL}",
            message.ordinal
        ));
    }
    Ok(())
}

async fn no_renumbering<R: RunnerRoute>(fixture: &CutSession<R>) -> Result<(), String> {
    let (_, after) = read_all(&fixture.session.route).await?;
    for kept in fixture
        .before
        .iter()
        .filter(|m| m.ordinal >= CUT_FIRST_ORDINAL)
    {
        match after.iter().find(|m| m.mid == kept.mid) {
            Some(now) if now.ordinal == kept.ordinal => {}
            Some(now) => {
                return Err(format!(
                    "message {} had ordinal {} before the cut and {} after it",
                    kept.mid, kept.ordinal, now.ordinal
                ))
            }
            None => {
                return Err(format!(
                    "message {} at ordinal {}, which the cut retained, is gone from the new \
                     lineage",
                    kept.mid, kept.ordinal
                ))
            }
        }
    }
    let ordinals: Vec<u64> = after.iter().map(|m| m.ordinal).collect();
    let expected: Vec<u64> = (CUT_FIRST_ORDINAL..6).collect();
    if ordinals != expected {
        return Err(format!(
            "the new lineage reads ordinals {ordinals:?}, expected {expected:?}: the exchange \
             after the cut continues the session-wide numbering"
        ));
    }
    Ok(())
}

async fn setup_once<S>(
    case: &Case<'_, S>,
    mut fixture: CutSession<S::Route>,
    label: &str,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    exchange(case, &mut fixture.session, &format!("{label}-prompt-3")).await?;
    let calls = compaction::observation(case.subject, &fixture.session.name)
        .await?
        .calls;
    if calls.len() != 2 {
        return Err(format!(
            "expected exactly one Setup on each of the two lineages, observed {} Setup calls",
            calls.len()
        ));
    }
    let new_lineage = head(&fixture.session.route).await?.lineage_id;
    if let Some(lineage) = &calls[1].lineage_id {
        if Some(lineage) != new_lineage.as_ref() {
            return Err(format!(
                "the second Setup names lineage {lineage}, not the cut lineage {new_lineage:?}"
            ));
        }
    }
    if calls[0]
        .lineage_id
        .as_deref()
        .is_some_and(|l| l != fixture.old_lineage)
    {
        return Err(format!(
            "the first Setup names lineage {:?}, not {}",
            calls[0].lineage_id, fixture.old_lineage
        ));
    }
    Ok(())
}

async fn ranges_start<S>(
    case: &Case<'_, S>,
    mut fixture: CutSession<S::Route>,
    label: &str,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let route = &fixture.session.route;
    let lineage = head(route)
        .await?
        .lineage_id
        .ok_or("the cut session has no lineage")?;
    let (_, now) = read_all(route).await?;
    let last = now.last().ok_or("the cut lineage reads empty")?.clone();
    let view = |from: u64| ScriptedView {
        lineage_id: lineage.clone(),
        covers_through: last.ordinal,
        covers_through_mid: last.mid.clone(),
        ranges: vec![
            ScriptedRange::Replace {
                from,
                to: CUT_FIRST_ORDINAL,
                replacement: vec![format!("{label}-refolded")],
            },
            ScriptedRange::Keep {
                from: CUT_FIRST_ORDINAL,
                to: last.ordinal + 1,
            },
        ],
        cut: None,
    };
    let from_zero = case
        .subject
        .publish_compaction_view(&fixture.session.name, view(0))
        .await
        .map_err(|e| e.to_string())?;
    exchange(case, &mut fixture.session, &format!("{label}-prompt-3")).await?;
    let seen = notices(case.subject, &fixture.session.name).await?;
    if !refused_by(&seen, from_zero) {
        return Err(format!(
            "a view tiling from 0 on a lineage starting at {CUT_FIRST_ORDINAL} was not refused \
             view_refused:{from_zero}:ranges_start"
        ));
    }
    if seen
        .iter()
        .any(|notice| notice.applied_view_version == Some(from_zero))
    {
        return Err(format!(
            "the runner applied view {from_zero}, which tiles from 0 on a lineage starting at \
             {CUT_FIRST_ORDINAL}"
        ));
    }
    // A view tiling from the first ordinal but leading with a keep range
    // carries nothing for the history before the cut: refused as well.
    let (_, now) = read_all(&fixture.session.route).await?;
    let last = now.last().ok_or("the cut lineage reads empty")?.clone();
    let headless = case
        .subject
        .publish_compaction_view(
            &fixture.session.name,
            ScriptedView {
                lineage_id: lineage.clone(),
                covers_through: last.ordinal,
                covers_through_mid: last.mid.clone(),
                ranges: vec![ScriptedRange::Keep {
                    from: CUT_FIRST_ORDINAL,
                    to: last.ordinal + 1,
                }],
                cut: None,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    exchange(
        case,
        &mut fixture.session,
        &format!("{label}-prompt-headless"),
    )
    .await?;
    let seen = notices(case.subject, &fixture.session.name).await?;
    if !refused_by(&seen, headless)
        || seen
            .iter()
            .any(|n| n.applied_view_version == Some(headless))
    {
        return Err(format!(
            "a view on a lineage starting at {CUT_FIRST_ORDINAL} whose first range is a keep \
             range, with no head for the history before the cut, was not refused \
             view_refused:{headless}:ranges_start"
        ));
    }
    // The same coverage tiled from the lineage's first ordinal applies, so
    // the refusals above were about how the ranges start.
    let (_, now) = read_all(&fixture.session.route).await?;
    let last = now.last().ok_or("the cut lineage reads empty")?.clone();
    let tiled = ScriptedView {
        lineage_id: lineage.clone(),
        covers_through: last.ordinal,
        covers_through_mid: last.mid.clone(),
        ranges: vec![
            ScriptedRange::Replace {
                from: CUT_FIRST_ORDINAL,
                to: CUT_FIRST_ORDINAL,
                replacement: vec![format!("{label}-head")],
            },
            ScriptedRange::Keep {
                from: CUT_FIRST_ORDINAL,
                to: last.ordinal + 1,
            },
        ],
        cut: None,
    };
    let tiled = case
        .subject
        .publish_compaction_view(&fixture.session.name, tiled)
        .await
        .map_err(|e| e.to_string())?;
    exchange(case, &mut fixture.session, &format!("{label}-prompt-4")).await?;
    let seen = notices(case.subject, &fixture.session.name).await?;
    if !seen
        .iter()
        .any(|notice| notice.applied_view_version == Some(tiled))
    {
        return Err(format!(
            "a view tiling from the lineage's first ordinal {CUT_FIRST_ORDINAL} did not apply \
             (no notice names applied_view_version {tiled})"
        ));
    }
    Ok(())
}

async fn descends_from<S>(case: &Case<'_, S>, fixture: &CutSession<S::Route>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let lineage = head(&fixture.session.route)
        .await?
        .lineage_id
        .ok_or("the cut session has no lineage")?;
    let seen = notices(case.subject, &fixture.session.name).await?;
    let first = seen
        .iter()
        .find(|notice| notice.lineage_id == lineage)
        .ok_or(format!(
            "the runner published no notice for lineage {lineage}"
        ))?;
    let cut = &fixture.before[(CUT_FIRST_ORDINAL - 1) as usize];
    let digest = cut
        .digest
        .clone()
        .unwrap_or_else(|| content_digest(&cut.message));
    match &first.descends_from {
        Some(descent)
            if descent.lineage_id == fixture.old_lineage
                && descent.through_ordinal == CUT_FIRST_ORDINAL - 1
                && descent.through_digest == digest => {}
        other => {
            return Err(format!(
                "the first notice of the cut lineage must descend from {}:{}:{digest}, the old \
                 lineage at the cut; it carries {other:?}",
                fixture.old_lineage,
                CUT_FIRST_ORDINAL - 1
            ))
        }
    }
    Ok(())
}

/// The content digest a read entry's message carries, as `descends_from`
/// and `session.head.cut` name it.
fn digest_of(message: &ReadMessage) -> String {
    message
        .digest
        .clone()
        .unwrap_or_else(|| content_digest(&message.message))
}

async fn head_names_cut<R: RunnerRoute>(fixture: &CutSession<R>) -> Result<(), String> {
    let now = head(&fixture.session.route).await?;
    let folded = &fixture.before[(CUT_FIRST_ORDINAL - 1) as usize];
    let cut = now
        .cut
        .ok_or("session.head names no cut on a lineage a cut started")?;
    if cut.mid != folded.mid || cut.digest != digest_of(folded) {
        return Err(format!(
            "session.head names the cut as {cut:?}; the cut folds {} last, with digest {}",
            folded.mid,
            digest_of(folded)
        ));
    }
    if folded.written_at.is_some_and(|at| at != cut.written_at) {
        return Err(format!(
            "session.head names the cut written at {}, but session.read served {:?} for it",
            cut.written_at, folded.written_at
        ));
    }
    Ok(())
}

async fn notice_first_ordinal<S>(
    case: &Case<'_, S>,
    fixture: &CutSession<S::Route>,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let lineage = head(&fixture.session.route)
        .await?
        .lineage_id
        .ok_or("the cut session has no lineage")?;
    let seen = notices(case.subject, &fixture.session.name).await?;
    let on_cut: Vec<&AdvanceNotice> = seen.iter().filter(|n| n.lineage_id == lineage).collect();
    if on_cut.is_empty() {
        return Err(format!(
            "the runner published no notice for lineage {lineage}"
        ));
    }
    if let Some(notice) = on_cut
        .iter()
        .find(|n| n.first_ordinal != Some(CUT_FIRST_ORDINAL))
    {
        return Err(format!(
            "notice {} on the cut lineage carries first_ordinal {:?}, not {CUT_FIRST_ORDINAL}",
            notice.seq, notice.first_ordinal
        ));
    }
    Ok(())
}

/// Publish a view over the current lineage that keeps its head (a replace
/// range from `first` folding through `through`), cuts through `through`,
/// and covers `covers`; answer the next lineage's Setup with a head at
/// `through + 1`; and run an exchange, whose first step applies the view.
#[allow(clippy::too_many_arguments)]
async fn cut_at<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    lineage: &str,
    first: u64,
    through: u64,
    covers: &ReadMessage,
    label: &str,
) -> Result<Fixture<()>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let ranges = vec![
        ScriptedRange::Replace {
            from: first,
            to: through + 1,
            replacement: vec![format!("{label}-folded-{through}")],
        },
        ScriptedRange::Keep {
            from: through + 1,
            to: covers.ordinal + 1,
        },
    ];
    let view = ScriptedView {
        lineage_id: lineage.to_owned(),
        covers_through: covers.ordinal,
        covers_through_mid: covers.mid.clone(),
        ranges,
        cut: Some(through),
    };
    if let Err(e) = case
        .subject
        .publish_compaction_view(&session.name, view)
        .await
    {
        return Ok(Fixture::Inapplicable(format!(
            "the adapter cannot publish a compaction view: {e}"
        )));
    }
    install_head_at(case, session, through + 1).await?;
    exchange(case, session, &format!("{label}-after-cut-{through}")).await?;
    Ok(Fixture::Ready(()))
}

/// Answer the next Setup with a head inserted at `first`.
async fn install_head_at<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    first: u64,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    case.subject
        .install_compaction_setup(
            &session.name,
            CompactionSetup::Ready(CompactionMessage {
                compaction_id: case.mint.next("head"),
                version: 1,
                from: first,
                to: first,
                replacement: vec![format!("head-at-{first}")],
            }),
        )
        .await
        .map_err(|e| e.to_string())
}

/// Revert `mid` and everything after it, or why the adapter cannot.
async fn revert<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    mid: &str,
) -> Result<Option<String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    Ok(case
        .subject
        .revert_session(case.handle.inner(), &session.name, mid)
        .await
        .err()
        .map(|e| format!("the adapter cannot revert a session: {e}")))
}

/// After a revert and one exchange on the new lineage: the lineage changed,
/// it starts at `first` with `cut` named on `session.head` (by `mid`), Setup
/// ran on it (more than `setups` calls, the last naming it where observed),
/// and its first notice carries `descent` (`None`: no `descends_from`).
#[allow(clippy::too_many_arguments)]
async fn check_restart<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    setups: usize,
    left: &str,
    first: u64,
    cut: Option<&str>,
    descent: Option<(&str, u64, &str)>,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let now = head(&session.route).await?;
    let lineage = now.lineage_id.clone().ok_or("the session has no lineage")?;
    if lineage == left {
        return Err(format!("a revert below the latest cut kept lineage {left}"));
    }
    if now.first_ordinal != Some(first) {
        return Err(format!(
            "after a revert below the latest cut, first_ordinal is {:?}, not {first}, where the \
             newest cut at or below the revert point leaves it",
            now.first_ordinal
        ));
    }
    if now.cut.as_ref().map(|c| c.mid.as_str()) != cut {
        return Err(format!(
            "after a revert below the latest cut, session.head names the cut {:?}, expected {cut:?}",
            now.cut
        ));
    }
    let calls = compaction::observation(case.subject, &session.name)
        .await?
        .calls;
    if calls.len() <= setups
        || calls
            .last()
            .and_then(|c| c.lineage_id.as_deref())
            .is_some_and(|l| l != lineage)
    {
        return Err(format!(
            "Setup did not run again for the lineage {lineage} a revert started"
        ));
    }
    let seen = notices(case.subject, &session.name).await?;
    let notice = seen
        .iter()
        .find(|n| n.lineage_id == lineage)
        .ok_or(format!(
            "the runner published no notice for lineage {lineage}"
        ))?;
    let declared = notice.descends_from.as_ref().map(|d| {
        (
            d.lineage_id.as_str(),
            d.through_ordinal,
            d.through_digest.as_str(),
        )
    });
    if declared != descent {
        return Err(format!(
            "the first notice of the lineage a revert started descends from {declared:?}, \
             expected {descent:?}"
        ));
    }
    Ok(())
}

/// Two cuts, then a revert between them restarts at the first cut,
/// descending from the lineage the runner was on at it; then a revert below
/// every cut restarts at the first message with no cut and no descent.
async fn revert_to_previous_cut<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let label = case.mint.next("revert-cut");
    let written = written(case, &label, 8).await?;
    let first_lineage = written.old_lineage.clone();
    let before = written.before.clone();
    let folded = before[(CUT_FIRST_ORDINAL - 1) as usize].clone();
    let fixture = match apply_cut(case, written, &label).await? {
        Fixture::Ready(fixture) => fixture,
        Fixture::Inapplicable(reason) => return Ok(Ending::Inapplicable(reason)),
    };
    let mut session = fixture.session;
    // A second cut, through ordinal 3, on the lineage the first cut started.
    let second = head(&session.route).await?.lineage_id.ok_or("no lineage")?;
    let (_, now) = read_all(&session.route).await?;
    let covers = now.last().ok_or("the cut lineage reads empty")?.clone();
    if let Fixture::Inapplicable(reason) = cut_at(
        case,
        &mut session,
        &second,
        CUT_FIRST_ORDINAL,
        3,
        &covers,
        &label,
    )
    .await?
    {
        return Ok(Ending::Inapplicable(reason));
    }
    if head(&session.route).await?.first_ordinal != Some(4) {
        return Err("the second cut did not start a lineage at ordinal 4".into());
    }
    // Revert from ordinal 3, the message the second cut folds last: that
    // cut is undone, and the first cut, whose message (ordinal 1) survives,
    // is the newest at or below the revert point.
    let third = head(&session.route).await?.lineage_id.ok_or("no lineage")?;
    let setups = compaction::observation(case.subject, &session.name)
        .await?
        .calls
        .len();
    install_head_at(case, &session, CUT_FIRST_ORDINAL).await?;
    if let Some(reason) = revert(case, &session, &before[3].mid).await? {
        return Ok(Ending::Inapplicable(reason));
    }
    exchange(case, &mut session, &format!("{label}-after-revert")).await?;
    let digest = digest_of(&folded);
    check_restart(
        case,
        &session,
        setups,
        &third,
        CUT_FIRST_ORDINAL,
        Some(&folded.mid),
        Some((&first_lineage, CUT_FIRST_ORDINAL - 1, &digest)),
    )
    .await?;
    // Below every cut: revert from ordinal 1, the first cut's message.
    let left = head(&session.route).await?.lineage_id.ok_or("no lineage")?;
    let setups = compaction::observation(case.subject, &session.name)
        .await?
        .calls
        .len();
    install_head_at(case, &session, 0).await?;
    if let Some(reason) = revert(case, &session, &folded.mid).await? {
        return Ok(Ending::Inapplicable(reason));
    }
    exchange(case, &mut session, &format!("{label}-below-every-cut")).await?;
    check_restart(case, &session, setups, &left, 0, None, None).await?;
    Ok(Ending::Passed)
}

/// A session adopted with host history ending in a cut: after the runner's
/// own cut, a revert below it restarts at the host's cut, which the runner
/// has no lineage at, so without `descends_from`.
async fn revert_to_host_cut<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let label = case.mint.next("host-cut");
    let script = Script {
        turns: (0..8)
            .map(|n| compaction::text(&format!("{label}-answer-{n}")))
            .collect(),
    };
    let mut session = compaction::install(
        case,
        &label,
        CompactionSetup::Ready(compaction::initial(&case.mint.next(&label))),
        script,
    )
    .await?;
    if let Err(e) = case
        .subject
        .seed_host_history(case.handle.inner(), &session.name, 2, true)
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot give a session a host cut before adoption: {e}"
        )));
    }
    exchange(case, &mut session, &format!("{label}-prompt-0")).await?;
    exchange(case, &mut session, &format!("{label}-prompt-1")).await?;
    let start = head(&session.route).await?;
    let host_cut = start
        .cut
        .clone()
        .ok_or("a session adopted after its host cut names no cut on session.head")?;
    if start.first_ordinal != Some(0) {
        return Err(format!(
            "the runner's first lineage starts at {:?}, not 0, though the host's cut predates \
             every ordinal",
            start.first_ordinal
        ));
    }
    let first_lineage = start.lineage_id.ok_or("no lineage")?;
    let (_, before) = read_all(&session.route).await?;
    if before.len() != 4 {
        return Err(format!("expected four messages, read {}", before.len()));
    }
    if let Fixture::Inapplicable(reason) =
        cut_at(case, &mut session, &first_lineage, 0, 1, &before[3], &label).await?
    {
        return Ok(Ending::Inapplicable(reason));
    }
    // Revert from ordinal 1, the runner's own cut's message: the host's cut
    // is the newest left, and the runner has no lineage at it.
    let left = head(&session.route).await?.lineage_id.ok_or("no lineage")?;
    let setups = compaction::observation(case.subject, &session.name)
        .await?
        .calls
        .len();
    install_head_at(case, &session, 0).await?;
    if let Some(reason) = revert(case, &session, &before[1].mid).await? {
        return Ok(Ending::Inapplicable(reason));
    }
    exchange(case, &mut session, &format!("{label}-after-revert")).await?;
    check_restart(case, &session, setups, &left, 0, Some(&host_cut.mid), None).await?;
    Ok(Ending::Passed)
}

/// The view [`apply_cut`] publishes over a written session: it folds
/// ordinals 0 and 1 and cuts through ordinal 1.
fn cut_view(written: &Written<impl RunnerRoute>, label: &str) -> ScriptedView {
    ScriptedView {
        lineage_id: written.old_lineage.clone(),
        covers_through: 3,
        covers_through_mid: written.before[3].mid.clone(),
        ranges: vec![
            ScriptedRange::Replace {
                from: 0,
                to: CUT_FIRST_ORDINAL,
                replacement: vec![format!("{label}-folded")],
            },
            ScriptedRange::Keep {
                from: CUT_FIRST_ORDINAL,
                to: 4,
            },
        ],
        cut: Some(CUT_FIRST_ORDINAL - 1),
    }
}

/// A provider answering a cut lineage's Setup with `checkpoint` makes that
/// Setup fail: the runner never applies a checkpoint to a lineage that
/// descends from another.
async fn checkpoint_on_descent<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let label = case.mint.next("checkpoint-descent");
    let mut written = written(case, &label, 4).await?;
    if let Err(e) = case
        .subject
        .publish_compaction_view(&written.session.name, cut_view(&written, &label))
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot publish a compaction view: {e}"
        )));
    }
    case.subject
        .install_compaction_setup(
            &written.session.name,
            CompactionSetup::ReadyWithCheckpoint {
                initial: CompactionMessage {
                    compaction_id: case.mint.next("cut-head"),
                    version: 1,
                    from: CUT_FIRST_ORDINAL,
                    to: CUT_FIRST_ORDINAL,
                    replacement: vec![format!("{label}-cut-head")],
                },
                tag_checkpoint: 500,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let models = compaction::observation(case.subject, &written.session.name)
        .await?
        .model_inputs
        .len();
    let reply = compaction::send(case, &mut written.session, &format!("{label}-prompt-2")).await?;
    let run_id = match &reply.run_id {
        Some(id) => id.clone(),
        None => {
            head(&written.session.route)
                .await?
                .last_run_state
                .ok_or("the send has no observable run")?
                .run_id
        }
    };
    // The run must end without a model request; one that reaches the model
    // has applied the checkpoint.
    let mut ended = None;
    for _ in 0..crate::drive::MAX_POLLS {
        let sent = compaction::observation(case.subject, &written.session.name)
            .await?
            .model_inputs
            .len();
        if sent > models {
            compaction::release_model(case.subject, &written.session.name).await?;
            return Err(
                "a cut lineage's Setup answered with a checkpoint reached the model: the \
                        runner applied the checkpoint instead of failing the Setup \
                        checkpoint_on_descent"
                    .into(),
            );
        }
        let result = crate::drive::run_result(&written.session.route, &run_id).await?;
        if result.run_state().is_terminal() {
            ended = Some(result);
            break;
        }
        case.subject.pause().await;
    }
    let ended = ended.ok_or(format!("run {run_id} did not end"))?;
    let error = ended.error.clone().ok_or(format!(
        "a cut lineage's Setup answered with a checkpoint ended {:?}, not error",
        ended.state
    ))?;
    if ended.run_state() != cortexkit_role_llm_runner::run::RunState::Error
        || error.provider_code
            != cortexkit_role_llm_runner::errors::provider_codes::COMPACTION_UNAVAILABLE
        || error.reason.as_deref()
            != Some(cortexkit_role_compaction_provider::setup::CHECKPOINT_ON_DESCENT)
    {
        return Err(format!(
            "a cut lineage's Setup answered with a checkpoint must fail as \
             compaction_unavailable with reason checkpoint_on_descent; the run ended {:?} with \
             {error:?}",
            ended.state
        ));
    }
    let after = compaction::observation(case.subject, &written.session.name)
        .await?
        .model_inputs
        .len();
    if after != models {
        return Err("a failed Setup still sent a model request".into());
    }
    Ok(Ending::Passed)
}

/// While a cut lineage's Setup is held, reads of the new lineage are served.
async fn reads_during_cut_setup<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if !case.subject.holds_reads_during_setup() {
        return Ok(Ending::Inapplicable(
            "the runner never refuses reads while a Setup is pending, so a cut's Setup cannot \
             interrupt them"
                .into(),
        ));
    }
    let label = case.mint.next("cut-setup-reads");
    let mut written = written(case, &label, 4).await?;
    if let Err(e) = case
        .subject
        .publish_compaction_view(&written.session.name, cut_view(&written, &label))
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot publish a compaction view: {e}"
        )));
    }
    install_head_at(case, &written.session, CUT_FIRST_ORDINAL).await?;
    if let Err(e) = case
        .subject
        .hold_next_compaction_setup(&written.session.name)
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot hold a Setup call: {e}"
        )));
    }
    let reply = compaction::send(case, &mut written.session, &format!("{label}-prompt-2")).await?;
    case.subject
        .await_compaction_setup_held(&written.session.name)
        .await
        .map_err(|e| e.to_string())?;
    let route = &written.session.route;
    let reads = async {
        let now = head(route).await?;
        read(route, &ReadRequest::tail()).await?;
        if let Some(first) = now.first_ordinal {
            read(route, &ReadRequest::range(first)).await?;
        }
        Ok::<(), String>(())
    }
    .await;
    case.subject
        .release_compaction_setup(&written.session.name)
        .await
        .map_err(|e| e.to_string())?;
    compaction::release_model(case.subject, &written.session.name).await?;
    compaction::result(case, &written.session, &reply).await?;
    reads.map_err(|e| format!("while a cut lineage's Setup was pending, {e}"))?;
    Ok(Ending::Passed)
}

/// The numbers of the counted parts of `messages`, in order.
fn numbers(messages: &[ReadMessage]) -> Vec<(String, u64)> {
    messages
        .iter()
        .flat_map(|m| {
            m.tags.iter().filter_map(move |row| {
                row.tag
                    .as_deref()
                    .and_then(cortexkit_role_llm_runner::tags::parse_tag)
                    .map(|n| (format!("{}/{}", m.mid, row.part), n))
            })
        })
        .collect()
}

/// On a tag-rendering session a cut renumbers nothing: retained parts keep
/// their numbers, and the first part after the cut continues from the old
/// lineage's highest number.
async fn numbering_continues<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let label = case.mint.next("cut-numbers");
    if let Err(reason) = crate::tags::rendering_fields(case.subject, &label) {
        return Ok(Ending::Inapplicable(reason));
    }
    let written = written_with(case, &label, 4, true).await?;
    let old_numbers = numbers(&written.before);
    let highest = old_numbers
        .iter()
        .map(|(_, n)| *n)
        .max()
        .ok_or("a tag-rendering session showed no tag numbers before the cut")?;
    let retained: Vec<(String, u64)> = numbers(&written.before[CUT_FIRST_ORDINAL as usize..]);
    let fixture = match apply_cut(case, written, &label).await? {
        Fixture::Ready(fixture) => fixture,
        Fixture::Inapplicable(reason) => return Ok(Ending::Inapplicable(reason)),
    };
    let (_, now) = read_all(&fixture.session.route).await?;
    let new_numbers = numbers(&now);
    for (key, number) in &retained {
        match new_numbers.iter().find(|(k, _)| k == key) {
            Some((_, now)) if now == number => {}
            other => {
                return Err(format!(
                    "part {key} showed {number} before the cut and {other:?} after it: a cut \
                     renumbers nothing"
                ))
            }
        }
    }
    let first_new = new_numbers
        .iter()
        .find(|(key, _)| !retained.iter().any(|(k, _)| k == key))
        .map(|(_, n)| *n)
        .ok_or("no part written after the cut shows a number")?;
    if first_new != highest + 1 {
        return Err(format!(
            "the first part written after the cut shows {first_new}; numbering continues from \
             the old lineage's highest number {highest}, so it is {}",
            highest + 1
        ));
    }
    Ok(Ending::Passed)
}
