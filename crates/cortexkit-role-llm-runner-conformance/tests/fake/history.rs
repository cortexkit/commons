//! Full active-path history and native marker metadata in the durable-log fake.
use super::*;
use cortexkit_role_llm_runner::{
    content_digest::content_digest,
    history::{HistoryEntry, HistoryMode, HistoryPage, HistoryRequest},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineageDefect {
    #[default]
    None,
    /// An `older` cursor skips a page.
    HistoryOlderSkipsPage,
    /// History mints its own ids instead of serving the messages' `mid`s.
    HistoryMintsOwnIds,
    /// History serves window entries from the host's copy, whose bytes
    /// differ from what `session.read` serves.
    HistoryWindowFromHost,
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
    /// The history version stays the same at a revert.
    HistoryVersionStaticAtRevert,
    /// `from_ordinal` starts one entry after the ordinal it names.
    HistoryOrdinalSeekOffByOne,
    /// An unknown ordinal is answered with an empty page.
    HistoryUnknownOrdinalEmpty,
    /// `before_ordinal` at the lowest ordinal answers an empty page with no
    /// `older`, though entries without an ordinal come before it.
    HistoryBeforeLowestOrdinalEmpty,
}

#[derive(Clone, Debug)]
pub struct CutRec {
    first: u64,
    mid: String,
    digest: String,
    written_at: u64,
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

pub(super) fn adopt_host_cut(sess: &mut Sess, world: &World) {
    let prefixes = world.host_prefixes.lock().unwrap();
    if let Some(entries) = prefixes.get(&sess.name) {
        for entry in entries {
            let ordinal = sess.push(entry.body.clone(), "", vec![]);
            let message = sess.at_mut(ordinal);
            message.mid = entry.mid.clone();
            message.written_at = entry.written_at;
        }
        if world.host_cuts.lock().unwrap().contains(&sess.name) {
            if let Some(last) = sess.messages.last() {
                sess.first_ordinal = last.ordinal + 1;
                sess.cuts.push(CutRec {
                    first: sess.first_ordinal,
                    mid: last.mid.clone(),
                    digest: content_digest(&last.body),
                    written_at: last.written_at,
                });
            }
        }
    }
}

pub(super) fn served_ordinal(_sess: &Sess, entry: ReadMessage, _defects: &Defects) -> ReadMessage {
    entry
}
pub(super) fn step_cost(sess: &Sess, defects: &Defects) {
    if defects.lineage == LineageDefect::CostWalksHistory {
        std::thread::sleep(Duration::from_micros(sess.archive.len() as u64));
    }
}
pub(super) fn apply(sess: &mut Sess, kind: &str, record: &Value, defects: &Defects) {
    match kind {
        "revert" => {
            let Some(at) = sess.messages.iter().position(|m| m.mid == record["mid"]) else {
                return;
            };
            sess.left.extend(sess.messages.split_off(at));
            if defects.lineage != LineageDefect::HistoryVersionStaticAtRevert {
                sess.history_version += 1;
            }
            sess.lineage = format!("{}-rewrite", sess.lineage);
            sess.cuts
                .retain(|cut| sess.messages.iter().any(|m| m.mid == cut.mid));
            sess.first_ordinal = sess.cuts.last().map_or(1, |cut| cut.first);
        }
        "host_compact" => {
            if let Some(last) = sess.messages.last() {
                sess.first_ordinal = last.ordinal + 1;
                sess.cuts.push(CutRec {
                    first: sess.first_ordinal,
                    mid: last.mid.clone(),
                    digest: content_digest(&last.body),
                    written_at: last.written_at,
                });
            }
        }
        "seed" => {
            let count = record["count"].as_u64().unwrap_or_default();
            for _ in 0..count {
                let ordinal = sess.push(json!({"role":"user","text":"seeded entry"}), "", vec![]);
                sess.at_mut(ordinal).written_at = record["at"].as_u64().unwrap_or(1);
            }
            let window = record["window"].as_u64().unwrap_or_default() as usize;
            let split = sess.messages.len().saturating_sub(window);
            let folded = split
                .checked_sub(1)
                .and_then(|index| sess.messages.get(index))
                .map(|message| {
                    (
                        message.mid.clone(),
                        content_digest(&message.body),
                        message.written_at,
                    )
                });
            let tail = sess.messages.split_off(split);
            sess.archive = Arc::new(std::mem::replace(&mut sess.messages, tail));
            sess.base = sess.messages.first().map_or(count + 1, |m| m.ordinal);
            sess.first_ordinal = sess.base;
            if let Some((mid, digest, written_at)) = folded {
                sess.cuts.push(CutRec {
                    first: sess.base,
                    mid,
                    digest,
                    written_at,
                });
            }
        }
        _ => {}
    }
}

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
    fn history_entries(&self, _session: &str, sess: &Sess) -> Vec<HistoryEntry> {
        let mut entries: Vec<_> = sess
            .archive
            .iter()
            .chain(&sess.messages)
            .map(|message| HistoryEntry::from_read(self.read_message(sess, message)).unwrap())
            .collect();
        match self.world.defects.lineage {
            LineageDefect::HistoryMintsOwnIds => {
                for entry in &mut entries {
                    entry.mid = format!("other-{}", entry.mid);
                }
            }
            LineageDefect::HistoryWindowFromHost => {
                for entry in &mut entries {
                    entry.message["host-title"] = json!("changed");
                }
            }
            LineageDefect::HistoryServesRevertedBranch => {
                entries.extend(sess.left.iter().map(|message| {
                    HistoryEntry::from_read(self.read_message(sess, message)).unwrap()
                }))
            }
            LineageDefect::HistoryPrefixAfterNumbered if !entries.is_empty() => {
                entries.rotate_left(1)
            }
            _ => {}
        }
        entries
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
            return respond(HistoryPage::new(1, "log-1", vec![]));
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
        let at_ordinal = |ordinal: u64| entries.iter().position(|entry| entry.ordinal == ordinal);
        let empty = || {
            respond(
                HistoryPage::new(
                    sess.history_version,
                    sess.incarnation.clone().unwrap_or_else(|| "log-1".into()),
                    Vec::new(),
                )
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
                Some(at) if defect == LineageDefect::HistoryBeforeLowestOrdinalEmpty && at > 0 => {
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
                    return refuse(errors::HISTORY_CHANGED, None);
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
        let mut page = HistoryPage::new(
            sess.history_version,
            sess.incarnation.clone().unwrap_or_else(|| "log-1".into()),
            entries[start..end].to_vec(),
        )
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
