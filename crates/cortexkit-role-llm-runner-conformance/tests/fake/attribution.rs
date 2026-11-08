//! Intake metadata is durable; facts are recomputed from the saved prefix.
use super::*;
use cortexkit_role_llm_runner::attribution::{self, Attribution};

pub(super) fn selected(
    senders: &[order::PlanSender],
    caller: &str,
    template: bool,
    defects: &Defects,
) -> Option<String> {
    let enabled = senders
        .iter()
        .any(|s| s.module == super::sender_module(caller) && s.attribution.is_some());
    if enabled && (!template || defects.attribution == Some("template_header"))
        || !enabled && defects.attribution == Some("other_sender_header")
    {
        Some(caller.into())
    } else {
        None
    }
}

pub(super) fn store(body: &mut Value, module: Option<&str>, defects: &Defects) {
    let Some(module) = module else { return };
    if defects.attribution == Some("missing_header") {
        return;
    }
    let prefix = attribution::header(module);
    // Attachment intake uses the fake's own `parts` field; profiled messages
    // store the same native parts under `content` instead.
    if body.get("content").is_some() {
        if let Some(parts) = body.as_object_mut().unwrap().remove("parts") {
            body["content"] = parts;
        }
    }
    if let Some(blocks) = body.get_mut("content").and_then(Value::as_array_mut) {
        if let Some(part) = blocks.iter_mut().find(|p| p["type"] == "text") {
            let old = part["text"].as_str().unwrap_or_default();
            part["text"] = json!(format!("{prefix}{old}"));
        } else if defects.attribution != Some("photo_no_header") {
            blocks.insert(0, json!({"type":"text", "text":prefix}));
        }
    } else {
        let old = body["text"].as_str().unwrap_or_default();
        body["text"] = json!(format!("{prefix}{old}"));
    }
}

pub(super) fn fact(message: &Msg, defects: &Defects) -> Option<Attribution> {
    let module = message.attribution.as_deref()?;
    let native = media::parts(&message.body);
    let (part, text) = native.iter().enumerate().find_map(|(i, (_, p))| {
        p.get("text")
            .and_then(Value::as_str)
            .map(|text| (i as u32, text))
    })?;
    let mut fact = if defects.attribution == Some("stale_fact") {
        attribution::verified(part, module, &attribution::header(module))
    } else {
        attribution::verified(part, module, text)
    }?;
    if defects.attribution == Some("wrong_fact") {
        fact.header_bytes += 1;
    }
    Some(fact)
}

impl Module {
    pub(super) fn attribution_for(&self, session: &str, caller: &str) -> Option<String> {
        self.sessions
            .lock()
            .unwrap()
            .get(session)
            .and_then(|sess| selected(&sess.senders, caller, false, &self.world.defects))
    }
}
