//! Applying a view: the render plan a runner renders from, and what the
//! model sees in the neutral part model.
//!
//! The provider computes every byte it puts in a view (placeholders such as
//! `[dropped §12§]`, cut-down edit arguments, the summaries and memories
//! inserted at the start of a head's request). The runner
//! adds nothing to them, not even a tag prefix; every other numbered text is
//! rendered with its prefix ([`RenderedMessage::wire_text`]).
//!
//! No edit changes a tag number: the runner counts from the transcript, not
//! from what it renders. The only way a view could mislead the model's count
//! is a number written into its own bytes, so [`plan`] refuses an inserted
//! message that carries a tag prefix, and a replaced field that carries one
//! the part neither owns nor already contained.

use std::collections::{BTreeSet, HashMap, HashSet};

use cortexkit_role_compaction::part::{
    Delivered, FileMedia, OutputFormat, Part, PartBody, ToolStatus,
};
use cortexkit_role_compaction::read::{Entry, NoticeOrigin, Role};
use cortexkit_role_compaction::view::{
    Edit, EditOp, InsertAt, InsertedImage, InsertedMessage, View, ViewRefusal,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tags::tag_notation_in;

/// One message as the model sees it after a view is applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderedMessage {
    /// The transcript message it comes from, or the inserted message's id.
    pub source: Source,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<NoticeOrigin>,
    /// The parts that remain, with replaced fields applied. A replaced part
    /// keeps its id and tag and gets a new checksum.
    pub parts: Vec<Part>,
    /// The parts whose text (a text part's text or a tool result's output)
    /// a view replaced. Those bytes are final: no tag prefix goes in front.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub final_text: BTreeSet<String>,
}

impl RenderedMessage {
    /// The text the model sees for a text part or a finished tool result:
    /// the tag prefix ([`crate::tags::tag_prefix`]) then the stored text, or
    /// a view's replaced bytes as they are. `None` for other parts and for a
    /// tool still running.
    pub fn wire_text(&self, part: &Part) -> Option<String> {
        let text = match &part.body {
            PartBody::Text { text } => text,
            PartBody::Tool(tool) => match crate::tags::empty_result_label(tool) {
                Some(label) if !self.final_text.contains(&part.id) => {
                    let prefix = part.tag.map(crate::tags::tag_prefix).unwrap_or_default();
                    return Some(format!("{prefix}{label}"));
                }
                _ => tool.output.as_ref()?,
            },
            // Reasoning, files, opaque parts and kinds the runner doesn't
            // know have no text the model reads as such.
            _ => return None,
        };
        match part.tag {
            Some(tag) if !self.final_text.contains(&part.id) => {
                Some(format!("{}{text}", crate::tags::tag_prefix(tag)))
            }
            _ => Some(text.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Entry { ordinal: u64, mid: String },
    Inserted { id: String },
}

/// Apply a view to the transcript and return what the model sees, in the
/// neutral part model. It checks the view's checksum and shape;
/// [`crate::rules::check`] checks the provider rules on top.
pub fn apply(entries: &[Entry], view: &View) -> Result<Vec<RenderedMessage>, ViewRefusal> {
    if !view.verify() {
        return Err(ViewRefusal::BadChecksum);
    }
    apply_edits(entries, &view.edits)
}

/// What a runner renders, as instructions over its own host messages
/// (contract W6.3). Runners render from it, so everything it keeps goes out
/// byte-identical: signatures, encrypted reasoning, item ids and cache
/// markers stay as the host stored them. An entry or a part the plan does not
/// list is left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderPlan {
    /// In render order.
    pub items: Vec<PlanItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum PlanItem {
    /// A message the view inserts, rendered with exactly these texts, then
    /// these images in the host's own form, and never numbered.
    Insert {
        id: String,
        role: Role,
        texts: Vec<String>,
        /// Only on a user-role message, PNG, in standard padded base64
        /// (contract W6.5). Left out when empty.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<InsertedImage>,
    },
    /// A transcript entry the model sees, with the parts it keeps in host
    /// order.
    Entry {
        ordinal: u64,
        mid: String,
        parts: Vec<PlanPart>,
    },
}

impl PlanItem {
    fn inserted(message: &InsertedMessage) -> Self {
        PlanItem::Insert {
            id: message.id.clone(),
            role: message.role.clone(),
            texts: message.texts.clone(),
            images: message.images.clone(),
        }
    }
}

/// One part a plan keeps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanPart {
    pub id: String,
    /// The part's number, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<u64>,
    /// Whether the runner renders the prefix ([`crate::tags::tag_prefix`]) in
    /// front of the part's text, or its tool result. False for a part with no
    /// number and for text a replace wrote, which is final.
    pub prefix: bool,
    /// For a finished tool result with no text: what the runner renders after
    /// the prefix ([`crate::tags::empty_result_label`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_result_label: Option<String>,
    /// Fields the runner renders with new bytes, in edit order. The runner
    /// maps each neutral field to its host's field (for example a failed
    /// call's error text for `output`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replace: Vec<FieldReplace>,
    /// For a tool part: the sends that ride its result, in delivery order.
    /// Left out when none render.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rides: Vec<PlanRide>,
}

/// A send that rides a tool result, from the part's `delivered` list. The
/// plan doesn't choose its form: the runner renders it after the result's
/// output with the contract's `rides` helper, or as a turn-scoped system
/// message where the send's `forms` and the session's recorded form allow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanRide {
    /// Its position in the part's stored `delivered` list.
    pub index: u64,
    #[serde(flatten)]
    pub delivered: Delivered,
}

/// What happens to a tool result's rides when a view replaces its output.
/// Each plan version renders with one rule ([`RideRule::for_version`]), so a
/// runner can keep rendering an applied view under the version it was first
/// served with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RideRule {
    /// Plan version 2: the rides still render after the replaced output.
    KeepAfterReplacedOutput,
    /// Plan version 3: a ride renders only while its carrying output does,
    /// so replacing the output drops its rides unless the view names
    /// `delivered` itself.
    FollowOutput,
}

impl RideRule {
    /// The rule a plan version renders with, or `None` for a version this
    /// library doesn't render.
    pub fn for_version(version: u32) -> Option<Self> {
        match version {
            2 => Some(RideRule::KeepAfterReplacedOutput),
            3 => Some(RideRule::FollowOutput),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldReplace {
    pub field: String,
    pub value: Value,
}

/// Validate edit structure, targets, running-tool restrictions, replacement
/// restrictions, conflicts, tag-prefix restrictions and the final turn
/// ([`check_final_turn`]), then produce a render plan at the current
/// [`crate::PLAN_VERSION`]. This does not check the view checksum or version,
/// that each edit's `since` names the view that first carried it
/// ([`check_carried`]), or the provider's rules for signed parts
/// ([`crate::rules::check`]). Check those separately before accepting a view.
pub fn plan(entries: &[Entry], edits: &[Edit]) -> Result<RenderPlan, ViewRefusal> {
    plan_with(RideRule::FollowOutput, entries, edits)
}

/// [`plan`] at an earlier plan version, for a view a runner first rendered
/// under it. `None` when this library doesn't render that version.
pub fn plan_at(
    version: u32,
    entries: &[Entry],
    edits: &[Edit],
) -> Option<Result<RenderPlan, ViewRefusal>> {
    RideRule::for_version(version).map(|rule| plan_with(rule, entries, edits))
}

/// [`plan`] with an explicit ride rule.
pub fn plan_with(
    rule: RideRule,
    entries: &[Entry],
    edits: &[Edit],
) -> Result<RenderPlan, ViewRefusal> {
    let by_mid: HashMap<&str, &Entry> = entries
        .iter()
        .map(|entry| (entry.mid.as_str(), entry))
        .collect();

    let mut dropped_messages: HashSet<&str> = HashSet::new();
    let mut dropped_parts: HashSet<(&str, &str)> = HashSet::new();
    let mut replaces: HashMap<(&str, &str), Vec<(&str, &Value)>> = HashMap::new();
    let mut inserts_at_start: Vec<&InsertedMessage> = Vec::new();
    let mut inserts_before: HashMap<&str, Vec<&InsertedMessage>> = HashMap::new();
    let mut insert_ids: HashSet<&str> = HashSet::new();
    let mut cut: Option<u64> = None;

    for (index, edit) in edits.iter().enumerate() {
        let unsupported = || ViewRefusal::UnsupportedEdit {
            index: index as u64,
        };
        match &edit.op {
            EditOp::Drop { mid, part } => {
                let entry = find_entry(&by_mid, mid)?;
                match part {
                    None => {
                        for part in &entry.parts {
                            refuse_running(mid, part)?;
                        }
                        dropped_messages.insert(mid);
                    }
                    Some(part_id) => {
                        let part = find_part(entry, part_id)?;
                        refuse_running(mid, part)?;
                        dropped_parts.insert((mid, part_id));
                    }
                }
            }
            EditOp::Replace {
                mid,
                part,
                field,
                value,
                expect,
            } => {
                let entry = find_entry(&by_mid, mid)?;
                let found = find_part(entry, part)?;
                refuse_running(mid, found)?;
                if &found.digest != expect {
                    return Err(ViewRefusal::StalePart {
                        mid: mid.clone(),
                        part: part.clone(),
                    });
                }
                // Every part of an entry whose role is unknown is handled as
                // an opaque part: it may be dropped, never replaced (W1.5).
                if matches!(entry.role, Role::Unknown(_)) {
                    return Err(ViewRefusal::BadField {
                        mid: mid.clone(),
                        part: part.clone(),
                        field: field.clone(),
                    });
                }
                check_field(mid, found, field, value)?;
                check_replaced_tags(mid, found, field, value)?;
                let fields = replaces.entry((mid, part)).or_default();
                if fields.iter().any(|(existing, _)| existing == field) {
                    return Err(ViewRefusal::Conflict {
                        mid: mid.clone(),
                        part: part.clone(),
                    });
                }
                fields.push((field, value));
            }
            EditOp::Cut { through } => {
                if cut.is_some() {
                    return Err(ViewRefusal::DuplicateCut);
                }
                let last = find_entry(&by_mid, through)?.ordinal;
                for entry in entries.iter().take_while(|entry| entry.ordinal <= last) {
                    for part in &entry.parts {
                        refuse_running(&entry.mid, part)?;
                    }
                    dropped_messages.insert(&entry.mid);
                }
                cut = Some(last);
            }
            EditOp::Insert { at, message } => {
                if matches!(message.role, Role::Unknown(_)) || matches!(at, InsertAt::Unknown(_)) {
                    return Err(unsupported());
                }
                if !insert_ids.insert(&message.id) {
                    return Err(ViewRefusal::DuplicateInsert {
                        id: message.id.clone(),
                    });
                }
                if message
                    .texts
                    .iter()
                    .any(|text| !tag_notation_in(text).is_empty())
                {
                    return Err(ViewRefusal::TaggedInsert {
                        id: message.id.clone(),
                    });
                }
                // W6.5: images only on a user-role message, PNG only, as
                // standard padded base64. A runner never decodes them
                // otherwise, so the check is here, before anything renders.
                if !message.images.is_empty() && message.role != Role::User {
                    return Err(unsupported());
                }
                if message.images.iter().any(|image| {
                    image.media_type != PNG || base64_decoded_len(&image.data).is_none()
                }) {
                    return Err(unsupported());
                }
                match at {
                    InsertAt::Start => inserts_at_start.push(message),
                    InsertAt::Before(mid) => {
                        find_entry(&by_mid, mid)
                            .map_err(|_| ViewRefusal::UnknownAnchor { mid: mid.clone() })?;
                        inserts_before.entry(mid).or_default().push(message);
                    }
                    _ => return Err(unsupported()),
                }
            }
            _ => return Err(unsupported()),
        }
    }

    for (mid, part) in replaces.keys() {
        if dropped_messages.contains(mid) || dropped_parts.contains(&(*mid, *part)) {
            return Err(ViewRefusal::Conflict {
                mid: (*mid).to_owned(),
                part: (*part).to_owned(),
            });
        }
    }
    check_final_turn(entries, edits)?;

    let mut items = Vec::with_capacity(entries.len() + inserts_at_start.len());
    items.extend(inserts_at_start.into_iter().map(PlanItem::inserted));
    for entry in entries {
        if let Some(before) = inserts_before.get(entry.mid.as_str()) {
            items.extend(before.iter().copied().map(PlanItem::inserted));
        }
        if dropped_messages.contains(entry.mid.as_str()) {
            continue;
        }
        let mut parts = Vec::with_capacity(entry.parts.len());
        for part in &entry.parts {
            if dropped_parts.contains(&(entry.mid.as_str(), part.id.as_str())) {
                continue;
            }
            let replace: Vec<FieldReplace> = replaces
                .get(&(entry.mid.as_str(), part.id.as_str()))
                .map(|fields| {
                    fields
                        .iter()
                        .map(|(field, value)| FieldReplace {
                            field: (*field).to_owned(),
                            value: (*value).clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let rides = rides_for(part, &replace, rule);
            let text_is_final = writes_final_text(&replace);
            // The label follows the part as replaced: keeping only some of a
            // result's attachments can change its label from `[file]` to
            // `[image]`. Only the body is needed, so an unedited part, most of
            // any window, is read as it stands instead of being rebuilt and
            // its digests computed again.
            let edited_body;
            let body = if replace.is_empty() {
                &part.body
            } else {
                edited_body = replaced_body(&part.body, &replace);
                &edited_body
            };
            let empty_result_label = match body {
                PartBody::Tool(tool) if !text_is_final => {
                    crate::tags::empty_result_label(tool).map(str::to_owned)
                }
                _ => None,
            };
            parts.push(PlanPart {
                id: part.id.clone(),
                tag: part.tag,
                prefix: part.tag.is_some() && !text_is_final,
                empty_result_label,
                replace,
                rides,
            });
        }
        // A message left with no parts is not rendered at all.
        if parts.is_empty() {
            continue;
        }
        items.push(PlanItem::Entry {
            ordinal: entry.ordinal,
            mid: entry.mid.clone(),
            parts,
        });
    }
    Ok(RenderPlan { items })
}

/// The compaction contract's rule PR6: a view never removes the turn the
/// request ends on. The
/// request ends with the newest entry that has parts; entries with no parts
/// (a step the runner opened but hasn't filled, a dead step) render nothing.
/// When that entry has a tool call, its last call must stay, because its
/// result is what ends the request: dropped, the request would end on the
/// model's own message, which providers such as Anthropic refuse. A view may
/// still replace that call's output. When the entry has no tool call, at
/// least one of its parts must stay. A cut through it removes it too.
///
/// Refuses with `UnsupportedEdit` naming the edit after which nothing that
/// must stay is left. Entries appended after the view was built fall outside
/// its edits and render as they are, so a later request ends on them.
pub fn check_final_turn(entries: &[Entry], edits: &[Edit]) -> Result<(), ViewRefusal> {
    let Some(last) = entries.iter().rev().find(|entry| !entry.parts.is_empty()) else {
        return Ok(());
    };
    let final_call = last
        .parts
        .iter()
        .rev()
        .find(|part| matches!(part.body, PartBody::Tool(_)));
    let mut removed: HashSet<&str> = HashSet::new();
    for (index, edit) in edits.iter().enumerate() {
        let gone = match &edit.op {
            EditOp::Drop { mid, part: None } => mid == &last.mid,
            EditOp::Drop {
                mid,
                part: Some(part),
            } if mid == &last.mid => {
                removed.insert(part.as_str());
                match final_call {
                    Some(call) => removed.contains(call.id.as_str()),
                    None => last
                        .parts
                        .iter()
                        .all(|part| removed.contains(part.id.as_str())),
                }
            }
            EditOp::Cut { through } => entries
                .iter()
                .find(|entry| &entry.mid == through)
                .is_some_and(|entry| entry.ordinal >= last.ordinal),
            _ => false,
        };
        if gone {
            return Err(ViewRefusal::UnsupportedEdit {
                index: index as u64,
            });
        }
    }
    Ok(())
}

/// The sends still riding a tool part after its replaces: the ones a
/// `delivered` replace names, else all of them. Under
/// [`RideRule::FollowOutput`] a replaced output takes them with it, so a
/// placeholder such as `[dropped §12§]` is never followed by a spent
/// reminder; a view that names `delivered` itself keeps its own choice.
fn rides_for(part: &Part, replace: &[FieldReplace], rule: RideRule) -> Vec<PlanRide> {
    let PartBody::Tool(tool) = &part.body else {
        return Vec::new();
    };
    let named: Option<HashSet<u64>> =
        replace
            .iter()
            .find(|edit| edit.field == "delivered")
            .map(|edit| {
                edit.value
                    .as_array()
                    .map(|indexes| indexes.iter().filter_map(Value::as_u64).collect())
                    .unwrap_or_default()
            });
    let output_replaced = replace.iter().any(|edit| edit.field == "output");
    if named.is_none() && output_replaced && rule == RideRule::FollowOutput {
        return Vec::new();
    }
    tool.delivered
        .iter()
        .enumerate()
        .map(|(index, delivered)| (index as u64, delivered))
        .filter(|(index, _)| named.as_ref().is_none_or(|named| named.contains(index)))
        .map(|(index, delivered)| PlanRide {
            index,
            delivered: delivered.clone(),
        })
        .collect()
}

/// A planned part as the model sees it: its replaces applied and, for a tool
/// part, only the rides the plan lists.
fn rendered_part(part: &Part, planned: &PlanPart) -> Part {
    let mut fields = planned.replace.clone();
    if let PartBody::Tool(tool) = &part.body {
        let names_delivered = fields.iter().any(|edit| edit.field == "delivered");
        if !names_delivered && planned.rides.len() != tool.delivered.len() {
            fields.push(FieldReplace {
                field: "delivered".to_owned(),
                value: Value::Array(
                    planned
                        .rides
                        .iter()
                        .map(|ride| Value::from(ride.index))
                        .collect(),
                ),
            });
        }
    }
    if fields.is_empty() {
        return part.clone();
    }
    replaced(part, &fields)
}

/// Whether a part's replaces write its text (a text part's text or a tool
/// result's output). Those bytes are final: no prefix goes in front.
fn writes_final_text(replace: &[FieldReplace]) -> bool {
    replace
        .iter()
        .any(|edit| matches!(edit.field.as_str(), "text" | "output"))
}

/// Apply a view's edits to the transcript and return what the model sees, in
/// the neutral part model: the render plan ([`plan`]) carried out over the
/// entries. Providers use it to know their own view's effect.
pub fn apply_edits(entries: &[Entry], edits: &[Edit]) -> Result<Vec<RenderedMessage>, ViewRefusal> {
    let plan = plan(entries, edits)?;
    let by_mid: HashMap<&str, &Entry> = entries
        .iter()
        .map(|entry| (entry.mid.as_str(), entry))
        .collect();
    Ok(plan
        .items
        .iter()
        .map(|item| match item {
            PlanItem::Insert {
                id,
                role,
                texts,
                images,
            } => render_inserted(id, role.clone(), texts, images),
            PlanItem::Entry {
                ordinal,
                mid,
                parts,
            } => {
                let entry = by_mid[mid.as_str()];
                let mut final_text = BTreeSet::new();
                let parts = parts
                    .iter()
                    .map(|planned| {
                        let part = entry
                            .parts
                            .iter()
                            .find(|part| part.id == planned.id)
                            .expect("a plan lists only parts of its entry");
                        if writes_final_text(&planned.replace) {
                            final_text.insert(part.id.clone());
                        }
                        rendered_part(part, planned)
                    })
                    .collect();
                RenderedMessage {
                    source: Source::Entry {
                        ordinal: *ordinal,
                        mid: mid.clone(),
                    },
                    role: entry.role.clone(),
                    notice: entry.notice.clone(),
                    parts,
                    final_text,
                }
            }
        })
        .collect())
}

/// Filter the existing view's edits for the new active transcript, as the
/// compaction contract's revert rule (W6.6) requires. Skip drops/replaces
/// targeting absent messages, inserts anchored before absent messages, and cuts
/// through absent messages; retain start inserts. This may change the rendering
/// of surviving entries, notably when a cut anchor disappears. Unknown edit
/// operations and anchors remain so the runner can refuse them explicitly.
pub fn edits_for_transcript(entries: &[Entry], edits: &[Edit]) -> Vec<Edit> {
    let present: HashSet<&str> = entries.iter().map(|entry| entry.mid.as_str()).collect();
    edits
        .iter()
        .filter(|edit| match &edit.op {
            EditOp::Drop { mid, .. } | EditOp::Replace { mid, .. } => {
                present.contains(mid.as_str())
            }
            EditOp::Insert {
                at: InsertAt::Before(mid),
                ..
            } => present.contains(mid.as_str()),
            EditOp::Cut { through } => present.contains(through.as_str()),
            // Inserts at the start stay. So do unknown anchors and ops, so
            // the runner refuses the view by name rather than lose an edit
            // silently.
            _ => true,
        })
        .cloned()
        .collect()
}

fn find_entry<'a>(by_mid: &HashMap<&str, &'a Entry>, mid: &str) -> Result<&'a Entry, ViewRefusal> {
    by_mid
        .get(mid)
        .copied()
        .ok_or_else(|| ViewRefusal::UnknownMessage {
            mid: mid.to_owned(),
        })
}

fn find_part<'a>(entry: &'a Entry, part_id: &str) -> Result<&'a Part, ViewRefusal> {
    entry
        .parts
        .iter()
        .find(|part| part.id == part_id)
        .ok_or_else(|| ViewRefusal::UnknownPart {
            mid: entry.mid.clone(),
            part: part_id.to_owned(),
        })
}

/// A running call, or one whose status is unknown (W1.5), is never dropped,
/// replaced or cut.
fn refuse_running(mid: &str, part: &Part) -> Result<(), ViewRefusal> {
    match &part.body {
        PartBody::Tool(tool)
            if !matches!(
                tool.status,
                ToolStatus::Completed | ToolStatus::Error | ToolStatus::Denied
            ) =>
        {
            Err(ViewRefusal::RunningTool {
                mid: mid.to_owned(),
                part: part.id.clone(),
            })
        }
        _ => Ok(()),
    }
}

fn check_field(mid: &str, part: &Part, field: &str, value: &Value) -> Result<(), ViewRefusal> {
    let bad = || ViewRefusal::BadField {
        mid: mid.to_owned(),
        part: part.id.clone(),
        field: field.to_owned(),
    };
    // A sealed part is checked by the model provider as it is, and a
    // provider-run tool call is replayed by the host as it is: neither can
    // take new bytes, only be dropped. An unknown seal counts as sealed.
    if !part.seal.is_none() {
        return Err(bad());
    }
    match (&part.body, field) {
        (PartBody::Text { .. }, "text") => value.as_str().map(|_| ()).ok_or_else(bad),
        // A file's place in its message is taken by text, such as a marker
        // that an answered image was removed.
        (PartBody::File(_), "text") => value.as_str().map(|_| ()).ok_or_else(bad),
        (PartBody::Tool(tool), _) if tool.provider_executed => Err(bad()),
        // A finished call has an output size whether it came in full or
        // compact detail (W4.3), and only then a result to rewrite. An output
        // in a format the runner doesn't know is never rewritten (W1.5).
        (PartBody::Tool(tool), "output")
            if tool.output_bytes.is_some()
                && !matches!(tool.output_format, OutputFormat::Unknown(_)) =>
        {
            value.as_str().map(|_| ()).ok_or_else(bad)
        }
        (PartBody::Tool(tool), "attachments") => {
            let existing: HashSet<&str> = tool
                .attachments
                .iter()
                .map(|attachment| attachment.id.as_str())
                .collect();
            let mut seen = BTreeSet::new();
            for id in value.as_array().ok_or_else(bad)? {
                let id = id.as_str().ok_or_else(bad)?;
                if !existing.contains(id) || !seen.insert(id) {
                    return Err(bad());
                }
            }
            Ok(())
        }
        (PartBody::Tool(tool), "delivered") => {
            let mut seen = BTreeSet::new();
            for index in value.as_array().ok_or_else(bad)? {
                let index = index.as_u64().ok_or_else(bad)?;
                if index >= tool.delivered.len() as u64 || !seen.insert(index) {
                    return Err(bad());
                }
            }
            Ok(())
        }
        (PartBody::Tool(tool), field) => {
            let key = field.strip_prefix("input.").ok_or_else(bad)?;
            match tool.input.get(key) {
                Some(Value::String(_)) => value.as_str().map(|_| ()).ok_or_else(bad),
                _ => Err(bad()),
            }
        }
        _ => Err(bad()),
    }
}

/// A replaced text field may show the part's own number (a placeholder needs
/// it) and any number its original text already showed (compressed text keeps
/// the references the model wrote), and no other. Exact replacement-tag
/// validation (W6.4) requires the original field text. With compact tool output,
/// this helper conservatively permits only the part's own number and may reject
/// otherwise-valid replacements. Runners must load replaced messages in full
/// before final validation.
fn check_replaced_tags(
    mid: &str,
    part: &Part,
    field: &str,
    value: &Value,
) -> Result<(), ViewRefusal> {
    let Some(new_text) = value.as_str() else {
        return Ok(());
    };
    let original: Option<&str> = match (&part.body, field) {
        (PartBody::Text { text }, "text") => Some(text),
        (PartBody::Tool(tool), "output") => tool.output.as_deref(),
        (PartBody::Tool(tool), field) => field
            .strip_prefix("input.")
            .and_then(|key| tool.input.get(key))
            .and_then(Value::as_str),
        _ => None,
    };
    let known: BTreeSet<String> = original
        .map(tag_notation_in)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let own = part.tag.map(|tag| tag.to_string());
    for number in tag_notation_in(new_text) {
        if Some(&number) != own.as_ref() && !known.contains(&number) {
            return Err(ViewRefusal::ForeignTag {
                mid: mid.to_owned(),
                part: part.id.clone(),
                field: field.to_owned(),
                // Digits too many for a number are reported as the largest.
                tag: number.parse().unwrap_or(u64::MAX),
            });
        }
    }
    Ok(())
}

fn replaced(part: &Part, fields: &[FieldReplace]) -> Part {
    let mut replaced =
        Part::new(part.id.clone(), replaced_body(&part.body, fields)).with_seal(part.seal.clone());
    replaced.tag = part.tag;
    replaced
}

/// A part's body with `fields` written in.
fn replaced_body(body: &PartBody, fields: &[FieldReplace]) -> PartBody {
    let mut body = body.clone();
    for FieldReplace { field, value } in fields {
        if field == "text" && matches!(body, PartBody::File(_)) {
            body = PartBody::Text {
                text: string_of(value),
            };
            continue;
        }
        match (&mut body, field.as_str()) {
            (PartBody::Text { text }, "text") => *text = string_of(value),
            (PartBody::Tool(tool), "output") => {
                let output = string_of(value);
                tool.output_bytes = Some(output.len() as u64);
                tool.output = Some(output);
            }
            (PartBody::Tool(tool), "attachments") => {
                let keep: HashSet<&str> = value
                    .as_array()
                    .map(|ids| ids.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                tool.attachments
                    .retain(|attachment| keep.contains(attachment.id.as_str()));
            }
            (PartBody::Tool(tool), "delivered") => {
                let keep: HashSet<u64> = value
                    .as_array()
                    .map(|indexes| indexes.iter().filter_map(Value::as_u64).collect())
                    .unwrap_or_default();
                let mut index = 0u64;
                tool.delivered.retain(|_| {
                    let kept = keep.contains(&index);
                    index += 1;
                    kept
                });
            }
            (PartBody::Tool(tool), field) => {
                if let (Some(key), Some(input)) =
                    (field.strip_prefix("input."), tool.input.as_object_mut())
                {
                    input.insert(key.to_owned(), Value::String(string_of(value)));
                }
            }
            _ => {}
        }
    }
    body
}

fn string_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// The one media type an inserted image may have (contract W6.5).
const PNG: &str = "image/png";

/// An inserted message in the neutral part model: its texts, then one inline
/// file part per image. Part ids are `<message-id>:<index>`, with image
/// indexes continuing after the texts; no part gets a tag number. The neutral
/// model holds media metadata only, so a file part carries the image's decoded
/// size, not its bytes; the runner renders the bytes from the plan.
fn render_inserted(
    id: &str,
    role: Role,
    texts: &[String],
    images: &[InsertedImage],
) -> RenderedMessage {
    let texts = texts
        .iter()
        .map(|text| PartBody::Text { text: text.clone() });
    let images = images.iter().map(|image| {
        PartBody::File(FileMedia::inline(
            image.media_type.clone(),
            base64_decoded_len(&image.data),
        ))
    });
    RenderedMessage {
        source: Source::Inserted { id: id.to_owned() },
        role,
        notice: None,
        parts: texts
            .chain(images)
            .enumerate()
            .map(|(index, body)| Part::new(format!("{id}:{index}"), body))
            .collect(),
        final_text: BTreeSet::new(),
    }
}

/// The number of bytes `data` decodes to, when it is standard padded base64
/// (RFC 4648 section 4) as contract W6.5 requires: the standard alphabet, a
/// length that is a multiple of four, `=` padding only as the last one or two
/// characters, and zero unused bits before the padding. An empty string is
/// refused, since no image has zero bytes. PNG contents are not checked.
pub fn base64_decoded_len(data: &str) -> Option<u64> {
    let bytes = data.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let padding = bytes.iter().rev().take_while(|&&byte| byte == b'=').count();
    if padding > 2 {
        return None;
    }
    let body = &bytes[..bytes.len() - padding];
    let value = |byte: u8| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut last = 0;
    for &byte in body {
        last = value(byte)?;
    }
    // With two `=` the last character carries 4 unused bits, with one `=` it
    // carries 2; a canonical encoder leaves them zero.
    let unused_mask = match padding {
        2 => 0b1111,
        1 => 0b11,
        _ => 0,
    };
    if last & unused_mask != 0 {
        return None;
    }
    Some((bytes.len() / 4 * 3 - padding) as u64)
}

/// Where one render plan first differs from another, both planned over the
/// same transcript: typically the plan of the view a runner applies now and
/// the plan of a newer view.
///
/// An item is one element of [`RenderPlan::items`]: the messages the view
/// inserts at the start, then the transcript entries in transcript order.
/// `item` is its zero-based index there, so item 0 is the first message of
/// the request after the system text and tools. A provider's prompt cache
/// for everything before the differing item still matches a request
/// rendered from the newer plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum FirstChange {
    /// Both plans render the same items.
    Unchanged,
    /// The first differing item is a message the newer plan inserts.
    Insert { item: u64, id: String },
    /// The first differing item is a transcript entry in the newer plan,
    /// named by its entry number and message id.
    Entry {
        item: u64,
        ordinal: u64,
        mid: String,
    },
    /// Every item both plans have is the same, and one plan has more items
    /// after them: entries appended (the newer plan is longer) or trailing
    /// entries left out (it is shorter). `item` is the index just past the
    /// shorter plan's last item.
    End { item: u64 },
}

impl FirstChange {
    /// The index in [`RenderPlan::items`] where the plans first differ, or
    /// `None` when nothing differs.
    pub fn item(&self) -> Option<u64> {
        match self {
            FirstChange::Unchanged => None,
            FirstChange::Insert { item, .. }
            | FirstChange::Entry { item, .. }
            | FirstChange::End { item } => Some(*item),
        }
    }

    /// Whether the difference starts at the request's first message. Then a
    /// request rendered from the newer plan already reuses no cached message,
    /// so a runner may also change what comes before the messages (the system
    /// text or the tool list) at no further cache cost: the provider rereads
    /// only those bytes on top of the messages it rereads anyway.
    pub fn from_first_message(&self) -> bool {
        self.item() == Some(0)
    }
}

/// Compare two render plans of the same transcript item by item and return
/// where `after` first differs from `before`. A plan item holds everything a
/// runner renders that message from: inserted texts and images, the parts
/// it keeps, replaced fields, and the module sends that ride a tool result.
/// So two items that compare equal render the same bytes.
pub fn first_change(before: &RenderPlan, after: &RenderPlan) -> FirstChange {
    let shared = before.items.len().min(after.items.len());
    for (index, (old, new)) in before.items.iter().zip(&after.items).enumerate() {
        if old == new {
            continue;
        }
        let item = index as u64;
        return match new {
            PlanItem::Insert { id, .. } => FirstChange::Insert {
                item,
                id: id.clone(),
            },
            PlanItem::Entry { ordinal, mid, .. } => FirstChange::Entry {
                item,
                ordinal: *ordinal,
                mid: mid.clone(),
            },
        };
    }
    if before.items.len() == after.items.len() {
        FirstChange::Unchanged
    } else {
        FirstChange::End {
            item: shared as u64,
        }
    }
}

/// Check that a new view's `since` values are honest against the view the
/// runner applies now: an edit claiming a version the runner already applied
/// must appear, unchanged, in that view, and no edit may claim a version later
/// than its own view. Versions in between are fine: the runner may not have
/// fetched every view the provider published. A view that passes cannot hide
/// a new edit from [`crate::rules::check`] by backdating it.
pub fn check_carried(previous: Option<&View>, next: &View) -> Result<(), ViewRefusal> {
    let previous_version = previous.map_or(0, |view| view.version);
    for edit in &next.edits {
        if edit.since == 0 || edit.since > next.version {
            return Err(ViewRefusal::BadSince);
        }
        if edit.since <= previous_version && !previous.is_some_and(|view| view.edits.contains(edit))
        {
            return Err(ViewRefusal::BadSince);
        }
    }
    Ok(())
}
