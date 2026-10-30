//! Lineages, the read bound, views, notices, `session.history` and the cost
//! rule in the durable-log fake (`cortexkit-role-llm-runner` CONTRACT §4.5,
//! §4.6, §14.2; compaction-provider §5, §6, §7).
//!
//! The fake keeps every message of the session's ancestry in `messages`; a
//! cut only moves `first_ordinal`, so reads refuse what lies below it and
//! history still serves it. Seeding is the one exception: seeded history
//! behind a cut goes to `archive`, which a step never touches.

use super::*;
use cortexkit_role_compaction_provider::{
    lineage::MessageRef,
    notice::{AdvanceNotice, Event, NoticeDescent, ViewReady},
    view::{
        dispose_fetched, CompactionView, TranscriptMessage, ViewCut, ViewDisposition, ViewRange,
    },
};
use cortexkit_role_llm_runner::{
    content_digest::content_digest,
    history::{HistoryEntry, HistoryMode, HistoryPage, HistoryRequest},
    read::{EntrySource, ModelEntry},
    session_ref::session_ref,
};
use cortexkit_role_llm_runner_conformance::{ScriptedRange, ScriptedView};

/// Deliberate breaks of the lineage, read-bound, view-start, history and
/// cost rules; each makes the fake violate one rule a case must catch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineageDefect {
    #[default]
    None,
    /// A view with a cut applies, but the session keeps its lineage.
    CutKeepsLineage,
    /// After a cut, reads naming the lineage the cut left are still served.
    OldLineageServedAfterCut,
    /// Reads below `first_ordinal` are served instead of refused.
    BelowFirstOrdinalServed,
    /// Reads serve ordinals counted from the lineage's start, so a cut
    /// renumbers every retained message.
    CutRenumbersOrdinals,
    /// Setup runs once per session: a new lineage keeps the old answer.
    SetupOncePerSession,
    /// A view is accepted wherever its ranges start.
    ViewFromZeroAccepted,
    /// The first notice of a cut lineage carries no `descends_from`.
    CutNoticeWithoutDescent,
    /// An `older` cursor skips a page.
    HistoryOlderSkipsPage,
    /// History mints its own ids instead of serving the messages' `mid`s.
    HistoryMintsOwnIds,
    /// History serves window entries from the host's copy, whose bytes
    /// differ from what `session.read` serves.
    HistoryWindowFromHost,
    /// History serves tags on entries below `first_ordinal`, copied from the
    /// lineage that showed them.
    HistoryTagsBelowFirst,
    /// History drops the ordinals of entries below the latest cut.
    HistoryDropsOrdinalsBelowCut,
    /// A history cursor minted on another lineage is refused
    /// `lineage_changed`, so cursors do not survive cuts.
    HistoryCursorLineageChanged,
    /// History serves the branch a revert left.
    HistoryServesRevertedBranch,
    /// A cursor a revert invalidated is answered with an empty page instead
    /// of `cursor_expired`.
    HistoryExpiredAsEmptyPage,
    /// History serves the host prefix after the numbered entries.
    HistoryPrefixAfterNumbered,
    /// `before_mid` answers an empty page with no `older` cursor.
    HistoryBeforeMidEmpty,
    /// Every step walks the whole seeded history.
    CostWalksHistory,
    /// `session.head.cut` names the cut lineage's first retained message
    /// instead of the message the cut folds last.
    HeadCutNamesFirstRetained,
    /// Notices carry no `first_ordinal`.
    NoticeWithoutFirstOrdinal,
    /// A revert below the latest cut keeps that cut's first ordinal.
    RevertKeepsTheCut,
    /// A revert below the latest cut, to a cut the runner has no lineage
    /// at, invents a descent from the lineage it left.
    RevertInventsDescent,
    /// The history version rises at a cut.
    HistoryVersionBumpsAtCut,
    /// The history version stays the same at a revert.
    HistoryVersionStaticAtRevert,
    /// `from_ordinal` starts one entry after the ordinal it names.
    HistoryOrdinalSeekOffByOne,
    /// An unknown ordinal is answered with an empty page.
    HistoryUnknownOrdinalEmpty,
    /// `before_ordinal` at the lowest ordinal answers an empty page with no
    /// `older`, though entries without an ordinal come before it.
    HistoryBeforeLowestOrdinalEmpty,
    /// A checkpoint on a Setup with descent is applied instead of failing
    /// the Setup.
    CheckpointOnDescentApplied,
    /// Reads are refused `transient` while a cut's Setup is pending too.
    ReadsHeldDuringCutSetup,
    /// After a cut, `session.read` numbers the window's parts afresh from 1.
    CutRestartsNumbering,
}

/// The scripted provider's hold on a session's next Setup call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupHold {
    /// The next Setup call will be held.
    Armed,
    /// A Setup call is held, unanswered.
    Held,
    /// The held call may be answered.
    Released,
}

/// Whether the session's Setup call is held now: an armed hold becomes held
/// at the call, and a released one is spent by it.
pub(super) fn setup_held(world: &World, session: &str) -> bool {
    let mut holds = world.setup_holds.lock().unwrap();
    match holds.get(session).copied() {
        Some(SetupHold::Armed) | Some(SetupHold::Held) => {
            holds.insert(session.into(), SetupHold::Held);
            true
        }
        Some(SetupHold::Released) => {
            holds.remove(session);
            false
        }
        None => false,
    }
}

/// Whether reads of `session` are refused `transient` now: while its Setup
/// is held, on a runner that holds reads, for a tag-rendering session's
/// Setup without descent, never for a cut's.
pub(super) fn reads_held(module: &Module, session: &str) -> bool {
    if !module.world.holds_reads_during_setup
        || module.world.setup_holds.lock().unwrap().get(session) != Some(&SetupHold::Held)
    {
        return false;
    }
    let sessions = module.sessions.lock().unwrap();
    let Some(sess) = sessions.get(session) else {
        return false;
    };
    let descends = sess.pending_descent.is_some();
    (!descends && sess.render_tags)
        || (descends && module.world.defects.lineage == LineageDefect::ReadsHeldDuringCutSetup)
}

/// The read entries as served: with the defect, a cut lineage's window
/// numbered afresh from 1.
pub(super) fn served_numbers(
    module: &Module,
    sess: &Sess,
    window: &[&Msg],
    mut messages: Vec<ReadMessage>,
) -> Vec<ReadMessage> {
    use cortexkit_role_llm_runner::tags::{format_tag, parse_tag};
    if module.world.defects.lineage != LineageDefect::CutRestartsNumbering
        || sess.first_ordinal == 0
    {
        return messages;
    }
    let lowest = window
        .iter()
        .flat_map(|m| module.read_message(sess, m).tags)
        .filter_map(|row| row.tag.as_deref().and_then(parse_tag))
        .min();
    let Some(offset) = lowest.map(|n| n - 1) else {
        return messages;
    };
    for message in &mut messages {
        for row in &mut message.tags {
            if let Some(n) = row.tag.as_deref().and_then(parse_tag) {
                row.tag = format_tag(n - offset);
            }
        }
    }
    messages
}

/// A cut in the session's ancestry: its first retained ordinal, the message
/// it folds last (by `mid`, content digest and creation time), and the
/// lineage the runner was on when it made the cut, absent for a cut the
/// host made before the runner adopted the session.
#[derive(Clone, Debug)]
pub struct CutRec {
    first: u64,
    mid: String,
    digest: String,
    written_at: u64,
    left_lineage: Option<String>,
}

/// `session.head.cut` for the current lineage.
pub(super) fn head_cut(
    sess: &Sess,
    defects: &Defects,
) -> Option<cortexkit_role_llm_runner::read::HeadCut> {
    let cut = sess.cuts.last()?;
    if defects.lineage == LineageDefect::HeadCutNamesFirstRetained {
        if let Some(first) = sess.messages.iter().find(|m| m.ordinal == cut.first) {
            return Some(cortexkit_role_llm_runner::read::HeadCut::new(
                &first.mid,
                content_digest(&first.body),
                first.written_at,
            ));
        }
    }
    Some(cortexkit_role_llm_runner::read::HeadCut::new(
        &cut.mid,
        &cut.digest,
        cut.written_at,
    ))
}

/// At a session's start, a cut the host made before adoption becomes the
/// first lineage's cut: its first ordinal is 0, and the runner had no
/// lineage at it.
pub(super) fn adopt_host_cut(sess: &mut Sess, world: &World) {
    if !world.host_cuts.lock().unwrap().contains(&sess.name) {
        return;
    }
    let prefixes = world.host_prefixes.lock().unwrap();
    if let Some(last) = prefixes.get(&sess.name).and_then(|entries| entries.last()) {
        sess.cuts.push(CutRec {
            first: 0,
            mid: last.mid.clone(),
            digest: content_digest(&last.body),
            written_at: last.written_at,
            left_lineage: None,
        });
    }
}

/// One message of the host's own history from before the runner numbered
/// anything.
#[derive(Clone, Debug)]
pub struct HostEntry {
    mid: String,
    written_at: u64,
    body: Value,
}

/// `count` host messages for `session`, oldest first, alternating a person's
/// prompt and the host's own reply.
pub fn host_entries(session: &str, count: u64) -> Vec<HostEntry> {
    (0..count)
        .map(|n| HostEntry {
            mid: format!("host-{session}-{n}"),
            written_at: 1_780_000_000_000 + n * 1_000,
            body: if super::display::profiled(session) {
                json!({"role":if n.is_multiple_of(2) {"user"} else {"assistant"}, "content":[{"type":"text","text":format!("host message {n}")}]})
            } else if n.is_multiple_of(2) {
                json!({ "role": "user", "text": format!("host prompt {n}") })
            } else {
                json!({ "role": "assistant", "parts": [{ "type": "text", "text": format!("host reply {n}") }] })
            },
        })
        .collect()
}

/// The view the runner renders now, as its record holds it.
#[derive(Clone, Debug)]
pub struct Applied {
    pub version: u64,
    /// `(from, to, replacement)`: a keep range has no replacement.
    pub ranges: Vec<(u64, u64, Option<Vec<String>>)>,
    pub covers_through: u64,
}

/// The `compaction_id` a model page names while a view renders.
pub fn view_compaction_id(version: u64) -> String {
    format!("view-{version}")
}

/// The session reference the scripted provider keys its state by.
fn reference(session: &str) -> String {
    session_ref(&cortexkit_role_llm_runner::SessionIdentity {
        runner: FAKE_RUNNER,
        harness: FAKE_HARNESS,
        session,
    })
}

/// The canonical model input under an applied view.
pub(super) fn render_view(sess: &Sess, applied: &Applied) -> Vec<ModelEntry> {
    let message = |m: &Msg| {
        ModelEntry::new(
            EntrySource::Message {
                ordinal: m.ordinal,
                mid: m.mid.clone(),
            },
            m.body.clone(),
        )
    };
    let mut entries = Vec::new();
    for (from, to, replacement) in &applied.ranges {
        match replacement {
            Some(texts) => entries.extend(texts.iter().map(|text| {
                ModelEntry::new(
                    EntrySource::Replacement {
                        compaction_id: view_compaction_id(applied.version),
                        version: applied.version,
                        from_ordinal: *from,
                        to_ordinal: *to,
                    },
                    json!({ "role": "system", "text": text }),
                )
            })),
            None => entries.extend(
                sess.messages
                    .iter()
                    .filter(|m| m.ordinal >= *from && m.ordinal < *to)
                    .map(message),
            ),
        }
    }
    entries.extend(
        sess.messages
            .iter()
            .filter(|m| m.ordinal > applied.covers_through)
            .map(message),
    );
    entries
}

/// Whether a read naming `lineage` is served.
pub(super) fn lineage_served(sess: &Sess, lineage: &str, defects: &Defects) -> bool {
    lineage == sess.lineage
        || (defects.lineage == LineageDefect::OldLineageServedAfterCut
            && sess.previous_lineages.iter().any(|l| l == lineage))
}

/// The `before_lineage_start` refusal of a range or after-message read
/// naming a position below the lineage's first ordinal.
pub(super) fn read_bound(sess: &Sess, mode: &ReadMode<'_>, defects: &Defects) -> Option<Reply> {
    if defects.lineage == LineageDefect::BelowFirstOrdinalServed {
        return None;
    }
    let below = match mode {
        ReadMode::Range { from_ordinal } => *from_ordinal < sess.first_ordinal,
        ReadMode::After { mid, .. } => sess
            .messages
            .iter()
            .find(|m| m.mid == *mid)
            .is_some_and(|m| m.ordinal < sess.first_ordinal),
        ReadMode::Tail => false,
    };
    below.then(|| {
        refuse(
            errors::BEFORE_LINEAGE_START,
            Some(json!({ "first_ordinal": sess.first_ordinal })),
        )
    })
}

/// Whether `session.read` serves `message`: only the current lineage's.
pub(super) fn in_window(sess: &Sess, message: &Msg, defects: &Defects) -> bool {
    message.ordinal >= sess.first_ordinal
        || defects.lineage == LineageDefect::BelowFirstOrdinalServed
}

/// The entry as served: with the defect, its ordinal counted from the
/// lineage's start.
pub(super) fn served_ordinal(
    sess: &Sess,
    mut message: ReadMessage,
    defects: &Defects,
) -> ReadMessage {
    if defects.lineage == LineageDefect::CutRenumbersOrdinals {
        message.ordinal = message.ordinal.saturating_sub(sess.first_ordinal);
    }
    message
}

/// The work a step does before its model request, beyond the window: none,
/// unless the defect walks the seeded history.
pub(super) fn step_cost(sess: &Sess, defects: &Defects) {
    if defects.lineage == LineageDefect::CostWalksHistory {
        for message in sess.archive.iter() {
            std::hint::black_box(content_digest(&message.body));
        }
    }
}

/// Start a new lineage, remembering the one left.
fn leave_lineage(sess: &mut Sess, suffix: String) {
    sess.previous_lineages.push(sess.lineage.clone());
    sess.lineage = format!("{}-{suffix}", sess.lineage);
}

pub(super) fn apply(sess: &mut Sess, kind: &str, record: &Value, defects: &Defects) {
    let number = |key: &str| record[key].as_u64().unwrap_or_default();
    match kind {
        "view_applied" => {
            let ranges = record["ranges"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|range| {
                    let replacement = range["replacement"].as_array().map(|texts| {
                        texts
                            .iter()
                            .map(|t| t.as_str().unwrap_or_default().to_owned())
                            .collect()
                    });
                    (
                        range["from"].as_u64().unwrap_or_default(),
                        range["to"].as_u64().unwrap_or_default(),
                        replacement,
                    )
                })
                .collect();
            sess.compaction.disposed.insert(number("version"));
            sess.compaction.applied = Some(Applied {
                version: number("version"),
                ranges,
                covers_through: number("covers_through"),
            });
        }
        "view_disposed" => {
            sess.compaction.disposed.insert(number("version"));
            if let Some(event) = record["event"].as_str() {
                sess.notice_events.push(event.to_owned());
            }
        }
        "cut" => {
            if sess.render_tags {
                super::message_checkpoint::write_native_key(sess, record, defects);
            }
            let first = number("first_ordinal");
            let old = sess.lineage.clone();
            if defects.lineage != LineageDefect::CutKeepsLineage {
                leave_lineage(sess, format!("c{first}"));
            }
            sess.first_ordinal = first;
            if defects.lineage == LineageDefect::HistoryVersionBumpsAtCut {
                sess.history_version += 1;
            }
            sess.cuts.push(CutRec {
                first,
                mid: record["through_mid"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                digest: record["through_digest"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                written_at: number("through_written_at"),
                left_lineage: Some(old.clone()),
            });
            // The applied view belongs to the lineage the cut left; the new
            // lineage renders its own Setup answer until its first view.
            sess.compaction.applied = None;
            sess.pending_descent = Some((
                old,
                first - 1,
                record["through_digest"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            ));
        }
        "notice" => {
            sess.notice_events.clear();
            sess.pending_descent = None;
        }
        "revert" => {
            let mid = record["mid"].as_str().unwrap_or_default();
            let Some(at) = sess.messages.iter().position(|m| m.mid == mid) else {
                return;
            };
            let ordinal = sess.messages[at].ordinal;
            let removed = sess.messages.split_off(at);
            sess.left.extend(removed);
            let left = sess.lineage.clone();
            leave_lineage(sess, format!("v{ordinal}"));
            if defects.lineage != LineageDefect::HistoryVersionStaticAtRevert {
                sess.history_version += 1;
            }
            sess.numbering.clear();
            // The new lineage starts at the newest cut at or below the revert
            // point. When the revert undid a cut, the first notice descends
            // from the lineage the runner was on at the cut it restarts at,
            // when it has one.
            let before = sess.cuts.len();
            if defects.lineage != LineageDefect::RevertKeepsTheCut {
                sess.cuts.retain(|cut| cut.first <= ordinal);
            }
            let undid_a_cut = sess.cuts.len() < before;
            sess.first_ordinal = sess.cuts.last().map_or(0, |cut| cut.first);
            sess.compaction.applied = None;
            sess.pending_descent = None;
            if undid_a_cut {
                if let Some(cut) = sess.cuts.last() {
                    let lineage = match &cut.left_lineage {
                        Some(lineage) => Some(lineage.clone()),
                        None if defects.lineage == LineageDefect::RevertInventsDescent => {
                            Some(left)
                        }
                        None => None,
                    };
                    sess.pending_descent = lineage
                        .map(|lineage| (lineage, cut.first.saturating_sub(1), cut.digest.clone()));
                }
            }
        }
        "seed" => {
            let at = number("at");
            let window = number("window") as usize;
            let mut all = std::mem::take(&mut sess.messages);
            for _ in 0..number("count") {
                let ordinal = sess.base + all.len() as u64;
                let body = if ordinal.is_multiple_of(2) {
                    json!({ "role": "user", "text": format!("seeded prompt {ordinal}") })
                } else {
                    json!({ "role": "assistant", "parts": [{ "type": "text", "text": format!("seeded reply {ordinal}") }] })
                };
                all.push(Msg {
                    ordinal,
                    mid: format!("{}.s{ordinal}", sess.lineage),
                    host_body: body.clone(),
                    body,
                    writer: cortexkit_role_llm_runner::tags::TextWriter::UntaggedRun,
                    run_id: String::new(),
                    calls: Vec::new(),
                    left_numbers: Vec::new(),
                    marker_cut: None,
                    marker_key: None,
                    delivered_as: None,
                    written_at: at,
                    client_message_id: None,
                    sender: None,
                });
            }
            let split = all.len() - window.min(all.len());
            let tail = all.split_off(split);
            let folded = all
                .last()
                .map(|m| (m.mid.clone(), content_digest(&m.body), m.written_at));
            let mut archive = (*sess.archive).clone();
            archive.extend(all);
            sess.archive = Arc::new(archive);
            sess.base = tail.first().map_or(sess.base, |m| m.ordinal);
            sess.messages = tail;
            sess.first_ordinal = sess.base;
            let left = sess.lineage.clone();
            leave_lineage(sess, format!("s{}", sess.base));
            if let Some((mid, digest, written_at)) = folded {
                sess.cuts.push(CutRec {
                    first: sess.base,
                    mid,
                    digest,
                    written_at,
                    left_lineage: Some(left),
                });
            }
            sess.numbering.clear();
        }
        _ => {}
    }
}

/// The page cursor: the session, the lineage it was minted on, its
/// direction (`before` an entry, `after` one, or `at` one) and that entry's
/// `mid` and position in the history.
fn cursor_of(session: &str, lineage: &str, direction: &str, mid: &str, index: usize) -> String {
    format!("h1~{session}~{lineage}~{direction}~{mid}~{index}")
}

struct Cursor {
    session: String,
    lineage: String,
    direction: String,
    mid: String,
    index: usize,
}

fn parse_cursor(cursor: &str) -> Option<Cursor> {
    let parts: Vec<&str> = cursor.split('~').collect();
    let [version, session, lineage, direction, mid, index] = parts[..] else {
        return None;
    };
    if version != "h1" || !matches!(direction, "before" | "after" | "at") {
        return None;
    }
    Some(Cursor {
        session: session.to_owned(),
        lineage: lineage.to_owned(),
        direction: direction.to_owned(),
        mid: mid.to_owned(),
        index: index.parse().ok()?,
    })
}

const HISTORY_FIELDS: &[&str] = &[
    "cursor",
    "from_mid",
    "before_mid",
    "from_ordinal",
    "before_ordinal",
    "limit",
    "max_bytes",
    "media",
    "include_originals",
    "view",
];

impl Module {
    /// Dispose of every view the provider published that this lineage has
    /// not seen yet, at the step boundary, as `view::dispose_fetched` says.
    pub(super) fn dispose_views(&self, session: &str) -> Result<(), Killed> {
        let views: Vec<CompactionView> = self.world.compaction.lock().unwrap().sessions[session]
            .views
            .clone();
        for view in views {
            let sess = self.sessions.lock().unwrap()[session].clone();
            if sess.compaction.disposed.contains(&view.view_version) {
                continue;
            }
            let first = if self.world.defects.lineage == LineageDefect::ViewFromZeroAccepted {
                view.ranges_start().unwrap_or(sess.first_ordinal)
            } else {
                sess.first_ordinal
            };
            let held = |ordinal: u64| {
                sess.messages
                    .iter()
                    .find(|m| m.ordinal == ordinal)
                    .map(|m| {
                        let parts = media::parts(&m.body)
                            .into_iter()
                            .map(|(key, _)| key)
                            .collect();
                        TranscriptMessage::new(ordinal, &m.mid, parts, m.body.clone())
                    })
            };
            let disposition = dispose_fetched(
                &view,
                &ViewReady::for_view(&view),
                sess.compaction.applied.as_ref().map(|a| a.version),
                &sess.lineage,
                first,
                held,
            );
            let version = view.view_version;
            match disposition {
                ViewDisposition::Apply => {
                    let ranges: Vec<Value> = view
                        .ranges
                        .iter()
                        .map(|range| match range {
                            ViewRange::Replace { from, to, bytes } => {
                                let texts: Vec<Value> = serde_json::from_str::<Vec<Value>>(bytes)
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|m| m["text"].clone())
                                    .collect();
                                json!({ "from": from, "to": to, "replacement": texts })
                            }
                            other => json!({ "from": other.from(), "to": other.to() }),
                        })
                        .collect();
                    self.commit(
                        session,
                        json!({ "kind": "view_applied", "version": version, "ranges": ranges,
                                "covers_through": view.covers_through.ordinal }),
                    )?;
                    if let Some(cut) = &view.cut {
                        let digest = sess
                            .messages
                            .iter()
                            .find(|m| m.ordinal == cut.through_ordinal)
                            .map(|m| content_digest(&self.read_message(&sess, m).message))
                            .unwrap_or_default();
                        let through = sess
                            .messages
                            .iter()
                            .find(|m| m.ordinal == cut.through_ordinal);
                        self.commit(
                            session,
                            json!({ "kind": "cut", "first_ordinal": cut.through_ordinal + 1,
                                    "through_digest": digest,
                                    "through_mid": through.map(|m| m.mid.clone()),
                                    "through_written_at": through.map(|m| m.written_at) }),
                        )?;
                    }
                }
                ViewDisposition::Refuse(reason) => self.commit(
                    session,
                    json!({ "kind": "view_disposed", "version": version,
                            "event": format!("view_refused:{version}:{reason}") }),
                )?,
                _ => self.commit(
                    session,
                    json!({ "kind": "view_disposed", "version": version }),
                )?,
            }
        }
        Ok(())
    }

    /// Publish an advance notice for the step about to send its model
    /// request: the lineage, its newest message, the view applied, the
    /// events since the last notice and, first on a cut lineage, its
    /// descent.
    pub(super) fn publish_notice(&self, session: &str, sess: &Sess) -> Result<(), Killed> {
        let Some(last) = sess.messages.last() else {
            return Ok(());
        };
        let digest = content_digest(&self.read_message(sess, last).message);
        let mut providers = self.world.compaction.lock().unwrap();
        let provider = providers.sessions.get_mut(session).unwrap();
        let seq = provider.notices.len() as u64 + 1;
        #[allow(deprecated)]
        let reference = if self.world.defects.compaction == CompactionDefect::SessionRefFromFolder {
            cortexkit_role_llm_runner::session_ref_v1(FAKE_HARNESS, "/fake/project", session)
        } else {
            reference(session)
        };
        let mut notice = AdvanceNotice::new(
            reference,
            seq,
            &sess.lineage,
            last.ordinal,
            digest,
            1_000,
            self.session_model(session),
        );
        if let Some(applied) = &sess.compaction.applied {
            notice = notice.with_applied_view_version(applied.version);
        }
        if self.world.defects.lineage != LineageDefect::NoticeWithoutFirstOrdinal {
            notice = notice.with_first_ordinal(sess.first_ordinal);
        }
        let events: Vec<Event> = sess
            .notice_events
            .iter()
            .filter_map(|token| Event::parse(token).ok())
            .collect();
        if !events.is_empty() {
            notice = notice.with_events(events);
        }
        if let Some((lineage, through, digest)) = &sess.pending_descent {
            if self.world.defects.lineage != LineageDefect::CutNoticeWithoutDescent
                && *lineage != sess.lineage
            {
                notice = notice.with_descends_from(NoticeDescent::new(lineage, *through, digest));
            }
        }
        provider.notices.push(notice.to_headers());
        drop(providers);
        self.commit(session, json!({ "kind": "notice", "seq": seq }))
    }

    /// The session's whole surviving history, oldest first.
    fn history_entries(&self, session: &str, sess: &Sess) -> Vec<HistoryEntry> {
        let defect = self.world.defects.lineage;
        let host: Vec<HistoryEntry> = self
            .world
            .host_prefixes
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_else(|| host_entries(session, self.world.host_prefix_everywhere))
            .iter()
            .map(|entry| {
                let mut out = HistoryEntry::new(&entry.mid, entry.written_at, entry.body.clone());
                if let Some(role) = entry.body["role"].as_str() {
                    out = out.with_role(role);
                }
                out
            })
            .collect();
        let plain = |m: &Msg| {
            let mut entry =
                HistoryEntry::new(&m.mid, m.written_at, m.body.clone()).with_ordinal(m.ordinal);
            if let Some(role) = m.body["role"].as_str() {
                entry = entry.with_role(role);
            }
            entry
        };
        let mut numbered: Vec<HistoryEntry> = sess.archive.iter().map(plain).collect();
        for m in &sess.messages {
            let read = self.read_message(sess, m);
            let mut entry = HistoryEntry::from_read(read).expect("every read entry has written_at");
            let below = m.ordinal < sess.first_ordinal;
            if below && defect != LineageDefect::HistoryTagsBelowFirst {
                entry = entry.without_tags();
            }
            if below && defect == LineageDefect::HistoryDropsOrdinalsBelowCut {
                entry.ordinal = None;
            }
            if !below && defect == LineageDefect::HistoryWindowFromHost {
                entry.message["host_copy"] = json!(true);
            }
            numbered.push(entry);
        }
        if defect == LineageDefect::HistoryServesRevertedBranch {
            numbered.extend(sess.left.iter().map(plain));
            numbered.sort_by_key(|entry| entry.ordinal);
        }
        if defect == LineageDefect::HistoryMintsOwnIds {
            for entry in &mut numbered {
                entry.mid = format!("h-{}", entry.mid);
            }
        }
        if self.world.defects.display_pi_profile && profiled(session) {
            for entry in &mut numbered {
                entry.message = display::pi_body(&entry.mid, &entry.message);
            }
        }
        if self.world.defects.display_opencode_v2_profile && profiled(session) {
            for entry in &mut numbered {
                entry.message = display::opencode_v2_body(&entry.mid, &entry.message);
            }
        }
        if defect == LineageDefect::HistoryPrefixAfterNumbered {
            numbered.extend(host);
            numbered
        } else {
            let mut all = host;
            all.extend(numbered);
            all
        }
    }

    /// `session.history`.
    pub(super) fn history(&self, session: &str, params: &Value) -> Reply {
        let mut fields = HISTORY_FIELDS.to_vec();
        if self.world.serves(groups::HISTORY_DISPLAY) {
            fields.extend(["form", "limits"]);
        }
        if let Some(field) = unknown_field(params, &fields) {
            return invalid(&field);
        }
        let request: HistoryRequest = match serde_json::from_value(params.clone()) {
            Ok(request) => request,
            Err(e) if e.to_string().contains("media") => return invalid("media"),
            Err(e) if e.to_string().contains("form") => return invalid("form"),
            Err(e) if params.get("limits").is_some() && !e.to_string().contains("view") => {
                return invalid("limits")
            }
            Err(_) => return invalid("view"),
        };
        let mode = match request.mode() {
            Ok(mode) => mode,
            Err(field) => return invalid(field),
        };
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            return respond(HistoryPage::no_lineage());
        };
        let entries = self.history_entries(session, sess);
        let defect = self.world.defects.lineage;
        let limit = request
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(1, MAXIMUM_LIMIT) as usize;
        let cap = request
            .max_bytes
            .unwrap_or(DEFAULT_MAX_BYTES)
            .min(MAXIMUM_MAX_BYTES) as usize;
        let size = |entry: &HistoryEntry| entry.message.to_string().len();
        let backward = |end: usize| {
            let mut start = end;
            let mut bytes = 0;
            while start > 0 && end - start < limit {
                let next = size(&entries[start - 1]);
                if start < end && bytes + next > cap {
                    break;
                }
                bytes += next;
                start -= 1;
            }
            (start, end)
        };
        let forward = |start: usize| {
            let mut end = start;
            let mut bytes = 0;
            while end < entries.len() && end - start < limit {
                let next = size(&entries[end]);
                if end > start && bytes + next > cap {
                    break;
                }
                bytes += next;
                end += 1;
            }
            (start, end)
        };
        let position = |mid: &str| entries.iter().position(|entry| entry.mid == mid);
        let at_ordinal = |ordinal: u64| {
            entries
                .iter()
                .position(|entry| entry.ordinal == Some(ordinal))
        };
        let empty = || {
            respond(
                HistoryPage::new(&sess.lineage, Vec::new())
                    .with_history_version(sess.history_version),
            )
        };
        let unknown_ordinal = || {
            if defect == LineageDefect::HistoryUnknownOrdinalEmpty {
                empty()
            } else {
                refuse(errors::UNKNOWN_ORDINAL, None)
            }
        };
        let (start, end) = match mode {
            HistoryMode::Newest => backward(entries.len()),
            HistoryMode::FromMid(mid) => match position(mid) {
                Some(at) => forward(at),
                None => return refuse(errors::UNKNOWN_MID, None),
            },
            HistoryMode::BeforeMid(mid) => match position(mid) {
                Some(_) if defect == LineageDefect::HistoryBeforeMidEmpty => return empty(),
                Some(at) => backward(at),
                None => return refuse(errors::UNKNOWN_MID, None),
            },
            HistoryMode::FromOrdinal(ordinal) => match at_ordinal(ordinal) {
                Some(at) if defect == LineageDefect::HistoryOrdinalSeekOffByOne => forward(at + 1),
                Some(at) => forward(at),
                None => return unknown_ordinal(),
            },
            HistoryMode::BeforeOrdinal(ordinal) => match at_ordinal(ordinal) {
                Some(at)
                    if defect == LineageDefect::HistoryBeforeLowestOrdinalEmpty
                        && entries[..at].iter().all(|e| e.ordinal.is_none()) =>
                {
                    return empty()
                }
                Some(at) => backward(at),
                None => return unknown_ordinal(),
            },
            HistoryMode::Cursor(cursor) => {
                let Some(cursor) = parse_cursor(cursor) else {
                    return invalid("cursor");
                };
                if cursor.lineage != sess.lineage
                    && defect == LineageDefect::HistoryCursorLineageChanged
                {
                    return refuse(errors::LINEAGE_CHANGED, None);
                }
                let at = position(&cursor.mid);
                if cursor.session != session || at != Some(cursor.index) {
                    if defect == LineageDefect::HistoryExpiredAsEmptyPage {
                        return empty();
                    }
                    return refuse(errors::CURSOR_EXPIRED, None);
                }
                match cursor.direction.as_str() {
                    "before" => backward(cursor.index),
                    "after" => forward(cursor.index + 1),
                    _ => forward(cursor.index),
                }
            }
        };
        let mut page = HistoryPage::new(&sess.lineage, entries[start..end].to_vec())
            .with_history_version(sess.history_version);
        if start > 0 {
            let anchor = if defect == LineageDefect::HistoryOlderSkipsPage && start > limit {
                start - limit
            } else {
                start
            };
            page = page.with_older(cursor_of(
                session,
                &sess.lineage,
                "before",
                &entries[anchor].mid,
                anchor,
            ));
        }
        if end < entries.len() {
            page = page.with_newer(if end > start {
                cursor_of(
                    session,
                    &sess.lineage,
                    "after",
                    &entries[end - 1].mid,
                    end - 1,
                )
            } else {
                cursor_of(session, &sess.lineage, "at", &entries[end].mid, end)
            });
        }
        if request.form() == cortexkit_role_llm_runner::history::HistoryForm::Display {
            return self.display_reply(session, sess, &page, request.limits.as_ref());
        }
        respond(page)
    }
}

impl FakeSubject {
    /// Publish `view` from the session's scripted provider, as the next
    /// version.
    pub(super) fn publish_view(&self, session: &str, view: ScriptedView) -> u64 {
        publish(&self.world(), session, view)
    }
}

/// Publish `view` from `session`'s scripted provider as the next version,
/// and answer that version.
pub(super) fn publish(world: &World, session: &str, view: ScriptedView) -> u64 {
    {
        let mut providers = world.compaction.lock().unwrap();
        let provider = providers.sessions.get_mut(session).unwrap();
        let version = provider.views.last().map_or(1, |v| v.view_version + 1);
        let ranges = view
            .ranges
            .iter()
            .filter_map(|range| match range {
                ScriptedRange::Keep { from, to } => Some(ViewRange::keep(*from, *to)),
                ScriptedRange::Replace {
                    from,
                    to,
                    replacement,
                } => {
                    let messages: Vec<Value> = replacement
                        .iter()
                        .map(|text| json!({ "role": "system", "text": text }))
                        .collect();
                    Some(ViewRange::replace(
                        *from,
                        *to,
                        serde_json::to_string(&messages).unwrap(),
                    ))
                }
                _ => None,
            })
            .collect();
        let mut built = CompactionView::new(
            reference(session),
            version,
            &view.lineage_id,
            MessageRef {
                ordinal: view.covers_through,
                mid: view.covers_through_mid.clone(),
            },
            ranges,
        );
        if let Some(through) = view.cut {
            built = built.with_cut(ViewCut::new(format!("cut-{version}"), through));
        }
        provider.views.push(built);
        version
    }
}
