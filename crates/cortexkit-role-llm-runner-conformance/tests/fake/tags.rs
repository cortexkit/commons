//! Tag numbering, exposure cleaning and rendering for the reference fake, by
//! llm-runner 0.4.0's rules (CONTRACT §9.6), on the role crate's shared
//! functions, so a fault in those functions shows in the suite's own tests.
//!
//! The fake renders its history in storage order with one exception, the
//! one real runners have: a user message (a steer) stored while a tool call
//! was running renders after that call's result, where a provider requires
//! a result to follow its call. So a steer into a held call is stored before
//! the result and rendered after it, and its number changes once the result
//! is stored; the fake applies that renumbering at the next step boundary,
//! on a new lineage.

use std::collections::BTreeMap;

use cortexkit_role_llm_runner::tags::{
    self, Anchor, Marker, MarkerCheckpoint, PartKind, RenderedPart, SetupCheckpoint, TextWriter,
};
use serde_json::Value;

use super::{Defects, Msg, Sess};

/// The member a scripted tool result carries to stand for an image-only
/// result: the fake renders it as an image and no text.
pub const IMAGE_RESULT: &str = "fake_image";

/// A part of a message, by message id and part key.
pub type Key = (String, String);

/// The writing run's frozen mode decides how `m`'s text is exposed,
/// independent of the session's current plan.
fn writer(_sess: &Sess, m: &Msg) -> TextWriter {
    m.writer
}

/// The model's text of `m` as every exposure shows it.
pub fn exposed(sess: &Sess, m: &Msg, text: &str, defects: &Defects) -> String {
    if defects.tags_exposed_raw {
        return text.to_owned();
    }
    tags::exposed_model_text(writer(sess, m), text).into_owned()
}

/// Every part of `m`, with its key and kind.
pub fn kinds(sess: &Sess, m: &Msg, defects: &Defects) -> Vec<(String, PartKind)> {
    // A host compaction pair: the marker message and its summary.
    if m.marker_cut.is_some() {
        return vec![("0".to_owned(), PartKind::CompactionMarker)];
    }
    if m.body.get("summary").is_some() {
        return vec![("0".to_owned(), PartKind::HistoryHead)];
    }
    let role = m.body["role"].as_str().unwrap_or_default();
    if let Some(parts) = m.body["parts"].as_array() {
        return parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                let kind = match part["type"].as_str().unwrap_or_default() {
                    // An embedded layout stores a call's result in the call's
                    // own tool part: the part then counts as the result.
                    "tool_call"
                        if part.get("result").is_some()
                            && !defects.tags_embedded_result_unnumbered =>
                    {
                        result_kind(&part["result"], defects)
                    }
                    "tool_call" => PartKind::ToolCall,
                    "reasoning" if defects.tags_count_reasoning => PartKind::AssistantText,
                    "reasoning" => PartKind::Reasoning,
                    _ => {
                        let text = part["text"].as_str().unwrap_or_default();
                        if exposed(sess, m, text, defects).is_empty() {
                            PartKind::EmptyText
                        } else {
                            PartKind::AssistantText
                        }
                    }
                };
                (index.to_string(), kind)
            })
            .collect();
    }
    let kind = match role {
        "system" => PartKind::TurnSystem,
        "tool" if defects.tags_tool_result_entry_unnumbered => PartKind::ToolCall,
        "tool" => result_kind(&m.body["content"], defects),
        _ if m.body["text"].as_str().is_some_and(str::is_empty) => PartKind::EmptyText,
        _ => PartKind::UserText,
    };
    vec![("0".to_owned(), kind)]
}

fn result_blocks(result: &Value) -> Vec<tags::ToolResultBlock<'_>> {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|block| match block["type"].as_str().unwrap_or_default() {
            "text" => tags::ToolResultBlock::Text(block["text"].as_str().unwrap_or_default()),
            "image" => tags::ToolResultBlock::Image,
            _ => tags::ToolResultBlock::Media,
        })
        .collect()
}

fn result_kind(result: &Value, defects: &Defects) -> PartKind {
    if result.get("fake_numbering").is_some() {
        if defects.tags_count_structured_result && result.get("content").is_none() {
            return PartKind::ToolResultText;
        }
        tags::tool_result_kind(&result_blocks(result))
    } else if result.get(IMAGE_RESULT).is_some() {
        PartKind::ToolResultImageOnly
    } else {
        PartKind::ToolResultText
    }
}

fn render_result(
    result: &Value,
    kind: PartKind,
    number: Option<u64>,
    defects: &Defects,
) -> Vec<String> {
    let blocks = result_blocks(result);
    let placement = if defects.tags_number_empty_block {
        blocks
            .iter()
            .position(|block| matches!(block, tags::ToolResultBlock::Text(_)))
    } else {
        tags::numbered_block_index(&blocks)
    };
    let mut texts = Vec::new();
    if placement.is_none() && tags::counts(kind) {
        let kind = if defects.tags_pdf_image_label && kind == PartKind::ToolResultMediaOnly {
            PartKind::ToolResultImageOnly
        } else {
            kind
        };
        texts.push(tags::render_part(kind, number, ""));
    }
    for (index, block) in blocks.iter().enumerate() {
        match block {
            tags::ToolResultBlock::Text(text) => {
                let number = number.filter(|_| {
                    Some(index) == placement || (defects.tags_number_every_text && !text.is_empty())
                });
                texts.push(tags::render_part(PartKind::ToolResultText, number, text));
            }
            _ => texts.push(result["content"][index].to_string()),
        }
    }
    texts
}

/// Indices of `messages` in the order the fake renders them: storage order,
/// except that a user message stored between a step's tool calls and the
/// last of their results renders right after that result.
pub fn rendered_order(messages: &[Msg]) -> Vec<usize> {
    let stored_results: Vec<&str> = messages
        .iter()
        .filter_map(|m| m.body["call_key"].as_str())
        .collect();
    let mut order = Vec::with_capacity(messages.len());
    let mut open: Vec<String> = Vec::new();
    let mut deferred = Vec::new();
    for (index, m) in messages.iter().enumerate() {
        let role = m.body["role"].as_str().unwrap_or_default();
        if role == "assistant" {
            open = m
                .calls
                .iter()
                .filter(|key| stored_results.contains(&key.as_str()))
                .cloned()
                .collect();
            order.push(index);
        } else if let Some(key) = m.body["call_key"].as_str() {
            open.retain(|open| open != key);
            order.push(index);
            if open.is_empty() {
                order.append(&mut deferred);
            }
        } else if !open.is_empty() && role == "user" {
            deferred.push(index);
        } else {
            order.push(index);
        }
    }
    order.append(&mut deferred);
    order
}

/// The numbering a fresh computation gives: empty unless the session's plan
/// renders tags; otherwise the Setup checkpoint at its cut, when the runner
/// honours one and it is there, or the session start.
pub fn fresh(sess: &Sess, defects: &Defects, honour_checkpoint: bool) -> BTreeMap<Key, u64> {
    if !sess.render_tags && !defects.tags_numbered_without_render_tags {
        return BTreeMap::new();
    }
    // The anchor, by the shared fallback chain: the newest host marker
    // carrying a key, then older ones, then the Setup checkpoint at its cut,
    // then the session start.
    let markers: Vec<Marker> = sess
        .messages
        .iter()
        .rev()
        .filter_map(|m| {
            let cut = m.marker_cut.as_ref()?;
            let mut marker = Marker::new(&m.mid, true).with_tail_start_id(cut);
            if let Some(n) = m.marker_key {
                marker = marker.with_checkpoint(MarkerCheckpoint::new(n, &sess.name));
            }
            Some(marker)
        })
        .collect();
    let setup = sess
        .compaction
        .checkpoint
        .as_ref()
        .filter(|_| honour_checkpoint)
        .map(|(n, cut)| SetupCheckpoint::new(*n, cut));
    let at = |mid: &str| sess.messages.iter().position(|m| m.mid == mid);
    let (checkpoint, cut) = match tags::anchor(&sess.name, &markers, setup.as_ref(), |mid| {
        at(mid).is_some()
    }) {
        Ok(Anchor::Marker { checkpoint, cut }) | Ok(Anchor::Setup { checkpoint, cut }) => {
            (checkpoint, at(&cut).unwrap_or(0))
        }
        Ok(Anchor::SessionStart { checkpoint, .. }) => (checkpoint, 0),
        _ => (0, 0),
    };
    // The defect counts only the parts rendered now: it skips the parts a
    // newer marker summarized when the anchor fell back past it.
    let newest_cut = markers
        .first()
        .and_then(|marker| at(marker.cut()))
        .unwrap_or(0);
    let cut = if defects.tags_count_only_rendered_parts {
        cut.max(newest_cut)
    } else {
        cut
    };
    // Parts are handed to the shared numbering in storage order, each with
    // its rendered position, as a runner holding its transcript would.
    let mut position = vec![0; sess.messages.len()];
    for (rendered_at, index) in rendered_order(&sess.messages).into_iter().enumerate() {
        position[index] = rendered_at;
    }
    let mut keys = Vec::new();
    let mut parts = Vec::new();
    let mut runs = Vec::new();
    for (index, m) in sess.messages.iter().enumerate().skip(cut) {
        // The defect numbers by where a part is stored instead.
        let rendered_at = if defects.tags_numbered_in_storage_order {
            index
        } else {
            position[index]
        };
        for (part, kind) in kinds(sess, m, defects) {
            keys.push((m.mid.clone(), part));
            parts.push(RenderedPart::new(kind, rendered_at as u64));
            runs.push(m.run_id.clone());
        }
    }
    if defects.tags_restart_each_run {
        // Numbers each run's parts, in rendered order, from the checkpoint
        // again.
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by_key(|index| parts[*index].rendered_at);
        let mut numbers = BTreeMap::new();
        let mut next = checkpoint;
        let mut last_run = None;
        for index in order {
            if last_run != Some(&runs[index]) {
                next = checkpoint;
                last_run = Some(&runs[index]);
            }
            if tags::counts(parts[index].kind) {
                next += 1;
                numbers.insert(keys[index].clone(), next);
            }
        }
        return numbers;
    }
    keys.into_iter()
        .zip(
            // The fake's checkpoints are small, so numbering never nears
            // u64::MAX; a real runner refuses the checkpoint instead.
            tags::try_number_parts(checkpoint, &parts).expect("fake checkpoints never overflow"),
        )
        .filter_map(|(key, number)| Some((key, number?)))
        .collect()
}

/// The numbering a read answers now: the numbering in effect for the
/// current lineage, for the parts that still exist.
pub fn current(sess: &Sess) -> BTreeMap<Key, u64> {
    sess.numbering
        .iter()
        .filter(|((mid, _), _)| sess.messages.iter().any(|m| &m.mid == mid))
        .map(|(key, number)| (key.clone(), *number))
        .collect()
}

/// After a part is written: it shows the number a fresh computation gives
/// it now, and every part already numbered keeps its number until the next
/// step boundary. Writes and reads never interleave in the fake, so a
/// collision a later write could cause is settled before any read.
pub fn note_write(sess: &mut Sess, defects: &Defects, honour_checkpoint: bool) {
    for (key, number) in fresh(sess, defects, honour_checkpoint) {
        sess.numbering.entry(key).or_insert(number);
    }
}

/// At a step boundary: take up a fresh computation. When it renumbers a
/// part already numbered, the session moves to a new lineage first.
pub fn settle(sess: &mut Sess, defects: &Defects, honour_checkpoint: bool) {
    let fresh = fresh(sess, defects, honour_checkpoint);
    let renumbered = sess
        .numbering
        .iter()
        .any(|(key, number)| fresh.get(key).is_some_and(|now| now != number));
    if renumbered && !defects.tags_renumber_without_new_lineage {
        sess.lineage = format!("{}-t", sess.lineage);
        sess.history_version += 1;
    }
    // A part behind the anchor's cut keeps the number it showed.
    let mut merged = fresh;
    for (key, number) in std::mem::take(&mut sess.numbering) {
        merged.entry(key).or_insert(number);
    }
    sess.numbering = merged;
}

/// The checkpoint a host compaction at `cut` writes on its marker: the
/// counted parts before the cut, which is the number the cut's first
/// counted part shows, less one.
pub fn checkpoint_before(sess: &Sess, cut: &str) -> u64 {
    let numbers = current(sess);
    let Some(at) = sess.messages.iter().position(|m| m.mid == cut) else {
        return 0;
    };
    sess.messages[at..]
        .iter()
        .find_map(|m| {
            numbers
                .iter()
                .filter(|((mid, _), _)| mid == &m.mid)
                .map(|(_, n)| *n)
                .min()
        })
        .map_or(0, |n| n.saturating_sub(1))
}

/// The host's save seam cleans model text by the writing run's frozen mode,
/// leaving the runner's raw record and non-text parts unchanged.
pub fn save_in_host(m: &mut Msg, defects: &Defects) {
    if defects.host_seam_not_cleaning {
        return;
    }
    if let Some(parts) = m.host_body["parts"].as_array_mut() {
        for part in parts {
            if part["type"] == "text" {
                if let Some(text) = part["text"].as_str() {
                    part["text"] =
                        Value::String(tags::exposed_model_text(m.writer, text).into_owned());
                }
            }
        }
    }
}

/// Read the host's saved copy directly, without running exposure cleaning.
pub fn host_stored(sess: &Sess, marker: &str, _defects: &Defects) -> Option<String> {
    sess.messages.iter().find_map(|m| {
        let parts = m.host_body["parts"].as_array()?;
        let text = parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .find(|text| text.contains(marker))?;
        Some(text.to_owned())
    })
}

/// The texts a model request carries, in request order, as the model
/// receives them.
pub fn rendered_texts(sess: &Sess, defects: &Defects, honour_checkpoint: bool) -> Vec<String> {
    let numbers = fresh(sess, defects, honour_checkpoint);
    let mut texts = Vec::new();
    for index in rendered_order(&sess.messages) {
        let m = &sess.messages[index];
        for (part, kind) in kinds(sess, m, defects) {
            let number = numbers.get(&(m.mid.clone(), part.clone())).copied();
            let result = if m.body["role"] == "tool" {
                &m.body["content"]
            } else {
                &m.body["parts"][part.parse::<usize>().unwrap_or_default()]["result"]
            };
            if result.get("fake_numbering").is_some() {
                texts.extend(render_result(result, kind, number, defects));
                continue;
            }
            let text = match m.body["parts"].as_array() {
                Some(parts) => {
                    let part = &parts[part.parse::<usize>().unwrap_or_default()];
                    match kind {
                        PartKind::ToolCall => continue,
                        PartKind::ToolResultText
                        | PartKind::ToolResultImageOnly
                        | PartKind::ToolResultMediaOnly => part["result"].to_string(),
                        PartKind::Reasoning => part["text"].as_str().unwrap_or_default().to_owned(),
                        _ => exposed(sess, m, part["text"].as_str().unwrap_or_default(), defects),
                    }
                }
                None => match m.body["text"].as_str() {
                    Some(text) => text.to_owned(),
                    None => m.body["content"].to_string(),
                },
            };
            texts.push(tags::render_part(kind, number, &text));
        }
    }
    texts
}

/// The tag rows of `m` in a read.
pub fn rows(
    sess: &Sess,
    m: &Msg,
    defects: &Defects,
    numbers: &BTreeMap<Key, u64>,
) -> Vec<tags::PartTag> {
    kinds(sess, m, defects)
        .into_iter()
        .map(
            |(part, _)| match numbers.get(&(m.mid.clone(), part.clone())) {
                Some(n) => tags::PartTag::tagged(&part, tags::format_tag(*n).unwrap()),
                None => tags::PartTag::untagged(part),
            },
        )
        .collect()
}

/// The part of the current lineage `number` names, if any.
pub fn find(sess: &Sess, number: u64) -> Option<(&Msg, String)> {
    let numbers = current(sess);
    let (mid, part) = numbers
        .iter()
        .find(|(_, n)| **n == number)
        .map(|(key, _)| key.clone())?;
    sess.messages
        .iter()
        .find(|m| m.mid == mid)
        .map(|m| (m, part))
}

/// The value of a scripted result standing for an image-only tool result.
pub fn image_result(marker: &str) -> Value {
    serde_json::json!({ IMAGE_RESULT: marker })
}
