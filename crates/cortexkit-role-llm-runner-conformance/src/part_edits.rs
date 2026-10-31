//! Negotiated part edits are observed at the model, not inferred from reads.

use cortexkit_role_compaction_provider::{
    ops as provider_ops,
    setup::{VIEW_FEATURE_TAG_BYTES_EDITS, VIEW_FEATURE_TRUNCATE},
    view::{PartEdit, TruncateMode},
};
use cortexkit_role_llm_runner::{
    read::EntrySource,
    serializer_profile::{self, ANTHROPIC_MESSAGES, OPENCODE_AISDK, OPENCODE_V2, OWNED_BROCA, PI},
    tags::{parse_tag, render_counted_text, truncate_part, PartContainer},
};
use serde_json::{Map, Value};

use crate::{
    cases::{Case, Ending},
    compaction::{self, CompactionMessage, CompactionSetup},
    drive::read_all,
    subject::{LlmRunnerSubject, Script},
    RunnerRoute,
};

fn parts(profile: &str, body: &Value) -> Result<Vec<Value>, String> {
    let content = match profile {
        OPENCODE_AISDK => body["parts"].clone(),
        OWNED_BROCA | ANTHROPIC_MESSAGES => body["content"].clone(),
        PI => serializer_profile::pi_projected_content(body).map_err(|e| e.to_string())?,
        OPENCODE_V2 => {
            serializer_profile::opencode_v2_projected_content(body).map_err(|e| e.to_string())?
        }
        _ => return Err(format!("unknown serializer profile {profile}")),
    };
    match content {
        Value::Array(parts) => Ok(parts),
        Value::String(text) => Ok(vec![serde_json::json!({"type":"text", "text":text})]),
        _ => Err("fixture message has no content parts".into()),
    }
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let truncate = match name {
        "part_edits_truncate_renders_with_own_tag" => true,
        "part_edits_bytes_render_with_own_tag" => false,
        _ => return Err(format!("unknown part-edit case {name}")),
    };
    let marker = case.mint.next("part-edit-original");
    let mut session = compaction::install(
        case,
        "part-edit",
        CompactionSetup::Ready(CompactionMessage {
            compaction_id: "empty-head".into(),
            version: 0,
            from: 0,
            to: 0,
            replacement: vec![],
        }),
        Script {
            turns: vec![compaction::text(&marker), compaction::text("finished")],
        },
    )
    .await?;
    let fields = crate::tags::rendering_fields(case.subject, &session.name)?;
    let first = crate::tags::send(
        case,
        &mut session,
        Some(&fields),
        "first prompt",
        &Map::new(),
    )
    .await?;
    compaction::release_model(case.subject, &session.name).await?;
    compaction::result(case, &session, &first).await?;
    let calls = match case.subject.compaction_provider_calls(&session.name).await {
        Ok(calls) => calls,
        Err(e) => {
            return Ok(Ending::Inapplicable(format!(
                "adapter lacks Setup request observation: {e}"
            )))
        }
    };
    let setup = calls
        .iter()
        .find(|call| call.method == provider_ops::COMPACTION_SETUP)
        .ok_or("provider observation contains no Setup request")?;
    let token = if truncate {
        VIEW_FEATURE_TRUNCATE
    } else {
        VIEW_FEATURE_TAG_BYTES_EDITS
    };
    if !setup
        .params
        .get("view_features")
        .and_then(Value::as_array)
        .is_some_and(|features| {
            features
                .iter()
                .any(|feature| feature.as_str() == Some(token))
        })
    {
        return Ok(Ending::Inapplicable(format!(
            "runner sends no {token} in Setup view_features"
        )));
    }
    let Some(profile) = case.subject.serializer_profile(&session.name) else {
        return Ok(Ending::Inapplicable(
            "adapter lacks serializer_profile".into(),
        ));
    };
    let (_, raw) = read_all(&session.route).await?;
    let mut target = None;
    for row in &raw {
        let values = parts(&profile, &row.message)?;
        if values.len() != row.tags.len() {
            return Err("part rows do not align with profile parts".into());
        }
        for (index, value) in values.iter().enumerate() {
            if value["type"] == "text" && value["text"] == marker {
                let number = row.tags[index]
                    .tag
                    .as_deref()
                    .and_then(parse_tag)
                    .ok_or("fixture assistant text has no own tag")?;
                target = Some((row, index, value.clone(), number));
            }
        }
    }
    let (row, index, original, number) = target.ok_or("fixture assistant text not exposed")?;
    let replacement = case.mint.next("provider-edited-text");
    let (edit, mut expected) = if truncate {
        (
            PartEdit::truncate(
                row.ordinal,
                &row.mid,
                &row.tags[index].part,
                TruncateMode::Placeholder,
            ),
            truncate_part(
                &profile,
                &original,
                PartContainer::Other,
                TruncateMode::Placeholder,
                Some(number),
                false,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        let mut value = original.clone();
        value["text"] = Value::String(replacement);
        (
            PartEdit::replace(
                row.ordinal,
                &row.mid,
                &row.tags[index].part,
                serde_json::to_string(&value).unwrap(),
            ),
            value,
        )
    };
    let text = expected["text"]
        .as_str()
        .ok_or("expected part has no text")?;
    expected["text"] = Value::String(render_counted_text(number, text));
    let version = match case
        .subject
        .publish_compaction_part_edits(&session.name, vec![edit])
        .await
    {
        Ok(version) if version > 0 => version,
        Ok(_) => return Err("part-edit publisher returned version zero".into()),
        Err(e) => {
            return Ok(Ending::Inapplicable(format!(
                "adapter lacks publish_compaction_part_edits: {e}"
            )))
        }
    };
    let reply = compaction::send(case, &mut session, "next prompt").await?;
    let mut inputs = Vec::new();
    for _ in 0..crate::drive::MAX_POLLS {
        inputs = compaction::observation(case.subject, &session.name)
            .await?
            .model_inputs;
        if inputs.len() >= 2 {
            break;
        }
        case.subject.pause().await;
    }
    compaction::release_model(case.subject, &session.name).await?;
    compaction::result(case, &session, &reply).await?;
    let request = inputs
        .get(1)
        .ok_or("edited view never reached the next model request")?;
    let entry = request
        .iter()
        .find(|entry| {
            entry.source
                == EntrySource::Message {
                    ordinal: row.ordinal,
                    mid: row.mid.clone(),
                }
        })
        .ok_or("edited message missing from model request")?;
    let rendered = parts(&profile, &entry.message)?;
    if rendered.get(index) != Some(&expected) {
        return Err(format!("{name}: model part must equal edited output with its own tag; got {:?}, expected {expected}", rendered.get(index)));
    }
    let calls = case
        .subject
        .compaction_provider_calls(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    if !calls
        .iter()
        .any(|call| call.method == provider_ops::VIEW_GET && call.params["view_version"] == version)
    {
        return Err("edited view was not fetched from the provider".into());
    }
    let (_, after) = read_all(&session.route).await?;
    if after
        .iter()
        .find(|after| after.mid == row.mid)
        .map(|after| &after.message)
        != Some(&row.message)
    {
        return Err("part edit rewrote the stored transcript".into());
    }
    Ok(Ending::Passed)
}
