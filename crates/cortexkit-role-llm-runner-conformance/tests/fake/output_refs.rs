//! Output-reference projection stored beside messages in the fake's log.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use cortexkit_role_llm_runner::content_digest::sha256_hex;
use cortexkit_role_llm_runner::output_refs::{
    self as refs, Descriptor, Record, ReferenceSlot, Resolution, Target,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub sets: Vec<StoredSet>,
    pub snapshots: BTreeMap<String, Vec<u8>>,
    pub original_tool_results: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct StoredSet {
    pub descriptor: Descriptor,
    pub records: Vec<Record>,
    pub snapshots: BTreeMap<String, Vec<u8>>,
}

fn targets(sess: &Sess, defects: &Defects) -> BTreeMap<u64, Target> {
    let mut out = BTreeMap::new();
    for ((mid, part), n) in tags::fresh(sess, defects, true) {
        let Some(m) = sess.messages.iter().find(|m| m.mid == mid) else {
            continue;
        };
        let mut target = if m.body["role"] == "tool" {
            let key = m.body["call_key"].as_str().unwrap_or("held-key");
            let content = if m.body["parts"].is_array() {
                &m.body["parts"][part.parse::<usize>().unwrap()]["text"]
            } else {
                &m.body["content"]
            };
            Target::text(&part, result_text(content).into_bytes()).with_tool(
                key,
                SCRIPTED_TOOL,
                TOOL_MODULE,
            )
        } else {
            let text = m.body["text"]
                .as_str()
                .or_else(|| m.body["parts"][part.parse::<usize>().unwrap_or(0)]["text"].as_str())
                .unwrap_or_default();
            Target::text(&part, text.as_bytes().to_vec())
        };
        if m.body["held"] == true {
            target = target.held();
        }
        out.insert(n, target);
    }
    out
}

pub(super) fn capture(sess: &Sess, record: &Value, defects: &Defects) -> Value {
    if !sess.render_tags {
        return json!([]);
    }
    let request = targets(sess, defects);
    let mut sets = Vec::new();
    let mut sequence = 0;
    let mut mint = || {
        sequence += 1;
        format!("{}:ref:{}:{sequence}", sess.name, sess.events.len())
    };
    let mut reply = Resolution::new();
    for part in record["parts"].as_array().into_iter().flatten() {
        if part["type"] == "text" {
            reply.reply(
                part["text"].as_str().unwrap_or_default(),
                &request,
                &mut mint,
            );
        }
    }
    let id = format!("{}:set:{}:reply", sess.name, sess.events.len());
    let descriptor = Descriptor::new(id, "", "rich-ui/1", &reply.records);
    sets.push(StoredSet {
        descriptor,
        records: reply.records,
        snapshots: reply.snapshots,
    });
    let slots = [
        ReferenceSlot::new("data", "token"),
        ReferenceSlot::new("program", "source:rich-ui/1"),
    ];
    for (index, call) in record["calls"].as_array().into_iter().flatten().enumerate() {
        if !sess.output_refs_enabled {
            continue;
        }
        let mut r = Resolution::new();
        r.call(&call["arguments"], &slots, true, &request, &mut mint);
        let id = format!("{}:set:{}:call-{index:04}", sess.name, sess.events.len());
        let descriptor = Descriptor::new(id, "", "rich-ui/1", &r.records)
            .with_call_key(call["call_key"].as_str().unwrap());
        sets.push(StoredSet {
            descriptor,
            records: r.records,
            snapshots: r.snapshots,
        });
    }
    if let Some(defect) = defects.output_refs.filter(|name| sess.name.contains(name)) {
        match defect {
            "output_refs_scanning" => {
                if record["parts"].to_string().contains("$$tc(") && sets[0].records.is_empty() {
                    let mut r = Resolution::new();
                    r.reply("$$tc(2)", &request, &mut mint);
                    sets[0].records = r.records;
                    sets[0].snapshots = r.snapshots;
                }
            }
            "output_refs_shared_samples" => {
                sets[0].records.clear();
            }
            "output_refs_target_reasons" => {
                for set in &mut sets {
                    for r in &mut set.records {
                        r.reason = Some("unknown_tag".into());
                    }
                }
            }
            "output_refs_snapshot_size" => {
                for set in &mut sets {
                    for r in &mut set.records {
                        if r.state == "too_large" {
                            r.bytes = Some(1);
                        }
                    }
                }
            }
            "output_refs_snapshot_budget" => {
                for set in &mut sets {
                    if set.records.len() == 5 {
                        set.records[4].state = "resolved".into();
                    }
                }
            }
            "output_refs_descriptor_digest" => {
                for set in &mut sets {
                    if let Some(r) = set.records.first_mut() {
                        r.tag = Some("999".into());
                    }
                }
            }
            "output_refs_slots" => {
                for set in &mut sets {
                    if set.descriptor.call_key.is_some() {
                        set.records.push(Record::unresolved(
                            mint(),
                            Some("2".into()),
                            "unknown_tag",
                        ));
                    }
                }
            }
            "output_refs_reference_limit" => {
                for set in &mut sets {
                    if set.records.len() > 64 {
                        set.records[64].reason = Some("unknown_tag".into());
                    }
                }
            }
            _ => {}
        }
    }
    if defects.output_refs_uncapped_metadata {
        for set in &mut sets {
            for record in &mut set.records {
                if let Some(original) = record
                    .call_key
                    .as_ref()
                    .and_then(|key| sess.output_refs.original_tool_results.get(key))
                {
                    record.bytes = Some(original.len() as u64);
                    record.sha256 = Some(sha256_hex(original.as_bytes()));
                }
            }
        }
    }
    let mut known: BTreeSet<_> = sess.output_refs.snapshots.keys().cloned().collect();
    for set in &mut sets {
        set.snapshots.retain(|sha, _| known.insert(sha.clone()));
    }
    serde_json::to_value(sets).unwrap()
}

pub(super) fn apply(sess: &mut Sess, kind: &str, record: &Value, defects: &Defects) {
    let defect = defects.output_refs.filter(|name| sess.name.contains(name));
    if kind == "step" {
        let mid = sess.messages.last().unwrap().mid.clone();
        let sets: Vec<StoredSet> =
            serde_json::from_value(record["output_sets"].clone()).unwrap_or_default();
        for mut set in sets {
            set.descriptor.mid = mid.clone();
            sess.output_refs.snapshots.append(&mut set.snapshots);
            sess.output_refs.sets.push(set);
        }
    }
    if kind == "refs_held" {
        sess.push(json!({"role":"tool","content":record["placeholder"],"call_key":"held-key","held":true}),"",vec![]);
        tags::note_write(sess, defects, true);
    }
    if kind == "refs_late" {
        for m in &mut sess.messages {
            if m.body["held"] == true {
                m.body["content"] = record["answer"].clone();
            }
        }
        if defect == Some("output_refs_held_placeholder") {
            for set in &mut sess.output_refs.sets {
                for r in &mut set.records {
                    if r.state == "held" {
                        let data = result_text(&record["answer"]).into_bytes();
                        sess.output_refs
                            .snapshots
                            .insert(r.sha256.clone().unwrap(), data);
                    }
                }
            }
        }
    }
    if kind == "refs_renumber" {
        sess.lineage.push_str("-renumbered");
        for n in sess.numbering.values_mut() {
            *n += 1000;
        }
        if defect == Some("output_refs_writing_request") {
            for set in &mut sess.output_refs.sets {
                for r in &mut set.records {
                    r.state = "unresolved".into();
                    r.reason = Some("unknown_tag".into());
                }
            }
        }
    }
    if kind == "revert" && defect != Some("output_refs_rewind") {
        let surviving: BTreeSet<_> = sess
            .messages
            .iter()
            .chain(sess.archive.iter())
            .map(|m| m.mid.as_str())
            .collect();
        sess.output_refs
            .sets
            .retain(|set| surviving.contains(set.descriptor.mid.as_str()));
    }
    if kind == "host_compact" && defect == Some("output_refs_compaction") {
        sess.output_refs.sets.clear();
    }
    if kind == "deleted" {
        sess.output_refs = State::default();
    }
}

pub(super) fn handle(
    module: &Module,
    _stamp: &RouteStamp,
    session: &str,
    method: &str,
    params: &Value,
) -> Reply {
    let sessions = module.sessions.lock().unwrap();
    let id = params[if method == ops::SESSION_OUTPUT_REFS {
        "set_id"
    } else {
        "ref_id"
    }]
    .as_str()
    .unwrap_or_default();
    let foreign = module.world.defects.output_refs == Some("output_refs_session_isolation");
    let all: Vec<_> = if foreign {
        sessions.values().collect()
    } else {
        sessions.get(session).into_iter().collect()
    };
    if method == ops::SESSION_OUTPUT_REFS {
        for sess in &all {
            if let Some(set) = sess
                .output_refs
                .sets
                .iter()
                .find(|s| s.descriptor.set_id == id)
            {
                return match refs::Page::from_records(&set.records, params["after"].as_str()) {
                    Ok(page) => Reply::Response(json!(page)),
                    Err(code) => refuse(code, None),
                };
            }
        }
    } else {
        for sess in &all {
            let r = sess
                .output_refs
                .sets
                .iter()
                .flat_map(|s| &s.records)
                .find(|r| r.ref_id == id);
            if let Some(r) = r {
                if !matches!(r.state.as_str(), "resolved" | "held") {
                    continue;
                }
                if let Some(data) = r
                    .sha256
                    .as_ref()
                    .and_then(|sha| sess.output_refs.snapshots.get(sha))
                {
                    let request: refs::ReadRequest = match serde_json::from_value(params.clone()) {
                        Ok(r) => r,
                        Err(_) => return invalid("ref_id"),
                    };
                    if module.world.defects.output_refs != Some("output_refs_read_limits") {
                        if let Err(field) = request.check(data.len() as u64) {
                            return invalid(field);
                        }
                    }
                    let start = request.offset.min(data.len() as u64) as usize;
                    let end = start
                        .saturating_add(request.length as usize)
                        .min(data.len());
                    let mut reply = refs::ReadReply::new(
                        STANDARD.encode(&data[start..end]),
                        data.len() as u64,
                        r.format.as_deref().unwrap_or("text"),
                    );
                    reply.mime.clone_from(&r.mime);
                    return Reply::Response(json!(reply));
                }
            }
        }
    }
    refuse("unknown_ref", None)
}

pub(super) fn decorate(
    sess: &Sess,
    mid: &str,
    row: &mut cortexkit_role_llm_runner::display::DisplayRow,
) {
    for set in sess
        .output_refs
        .sets
        .iter()
        .filter(|s| s.descriptor.mid == mid)
    {
        if let Some(key) = &set.descriptor.call_key {
            if let Some(index) = sess
                .messages
                .iter()
                .find(|m| m.mid == mid)
                .and_then(|m| m.calls.iter().position(|k| k == key))
            {
                if let Some(cortexkit_role_llm_runner::display::DisplayPart::ToolCall(call)) = row
                    .parts
                    .iter_mut()
                    .filter(|p| {
                        matches!(
                            p,
                            cortexkit_role_llm_runner::display::DisplayPart::ToolCall(_)
                        )
                    })
                    .nth(index)
                {
                    call.output_refs = Some(set.descriptor.clone());
                }
            }
        } else {
            row.output_refs = Some(set.descriptor.clone());
        }
    }
}
