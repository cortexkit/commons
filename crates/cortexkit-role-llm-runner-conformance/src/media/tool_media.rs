//! Readers opt in before a runner exposes structured tool-output media.
use super::*;

pub(super) async fn invalid_params<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let session = case.open("tool-media-params", Script::default()).await?;
    let mut requests = vec![json!({"tool_media":"refs"})];
    for media in ["inline", "descriptors", "blocks"] {
        requests.push(json!({"media":media,"tool_media":"refs"}));
    }
    for value in [json!(null), json!(false), json!(3), json!("blocks")] {
        requests.push(json!({"media":"media_refs","tool_media":value}));
    }
    for params in requests {
        let error = expect_error(
            "invalid tool_media",
            call(&session.route, ops::SESSION_READ, params).await?,
        )?;
        expect_code("invalid tool_media", &error, errors::INVALID_PARAMS)?;
        expect_field(
            "invalid tool_media",
            &error,
            errors::INVALID_PARAMS,
            "tool_media",
        )?;
    }
    Ok(Ending::Passed)
}

type Outputs = BTreeMap<(String, String), Value>;

/// Remove only the output subtrees the adapter names. The remaining entries
/// retain every surrounding payload and identity for a strict comparison.
fn split<S: LlmRunnerSubject>(
    subject: &S,
    messages: &[ReadMessage],
    owned: bool,
) -> Result<(Vec<ReadMessage>, Outputs), String> {
    let mut outside = messages.to_vec();
    let mut outputs = BTreeMap::new();
    for message in &mut outside {
        let mut pointers = BTreeSet::new();
        if owned {
            if let Some(parts) = message.message["content"].as_array() {
                for (index, part) in parts.iter().enumerate() {
                    if part["type"] == "tool_result" {
                        pointers.insert(format!("/content/{index}/output"));
                    }
                }
            }
        } else {
            for (key, _) in subject.read_parts(&message.message) {
                if let Some(pointer) =
                    subject.media_refs_tool_output_pointer(&message.message, &key)
                {
                    if !pointers.insert(pointer) {
                        return Err("tool-media output pointers repeat".into());
                    }
                }
            }
        }
        for pointer in &pointers {
            if pointer.is_empty()
                || pointers
                    .iter()
                    .any(|other| other != pointer && pointer.starts_with(&format!("{other}/")))
            {
                return Err(
                    "tool-media output pointers overlap or identify the whole message".into(),
                );
            }
            let output = message
                .message
                .pointer_mut(pointer)
                .ok_or("tool-media output pointer is missing")?;
            let output = std::mem::replace(output, Value::Null);
            if outputs
                .insert((message.mid.clone(), pointer.clone()), output)
                .is_some()
            {
                return Err("tool-media output identity repeats".into());
            }
        }
    }
    Ok((outside, outputs))
}

/// Normalize typed descriptor refs and the untagged media values served by
/// owned-broca blocks to the same metadata, retaining traversal order.
fn media(value: &Value, out: &mut Vec<Value>) -> Check {
    if value["sha256"].is_string()
        && value["mime"].is_string()
        && value["kind"].is_string()
        && value["byte_size"].is_u64()
    {
        if value.as_object().is_some_and(|object| {
            object.keys().any(|key| {
                ![
                    "sha256",
                    "mime",
                    "kind",
                    "byte_size",
                    "width",
                    "height",
                    "type",
                    "part",
                ]
                .contains(&key.as_str())
            })
        }) {
            return Err(
                "tool_media reference carries non-reference fields; media bytes are forbidden"
                    .into(),
            );
        }
        let mut metadata = serde_json::Map::new();
        for field in ["sha256", "mime", "kind", "byte_size", "width", "height"] {
            if let Some(value) = value.get(field) {
                metadata.insert(field.into(), value.clone());
            }
        }
        out.push(Value::Object(metadata));
    } else if let Some(object) = value.as_object() {
        for child in object.values() {
            media(child, out)?;
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            media(child, out)?;
        }
    }
    Ok(())
}

fn mask_media(value: &Value) -> Value {
    if value["sha256"].is_string()
        && value["mime"].is_string()
        && value["kind"].is_string()
        && value["byte_size"].is_u64()
    {
        Value::Null
    } else {
        match value {
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), mask_media(value)))
                    .collect(),
            ),
            Value::Array(array) => Value::Array(array.iter().map(mask_media).collect()),
            _ => value.clone(),
        }
    }
}

fn check_output(
    served: &Value,
    baseline: &Value,
    inline: &Value,
    stored: &Value,
    described: &Value,
    owned: bool,
) -> Result<bool, String> {
    let mut expected = Vec::new();
    media(described, &mut expected)?;
    if expected.is_empty() {
        if served != baseline {
            return Err("tool_media changed an output without media".into());
        }
        return Ok(false);
    }
    if inline.to_string().contains("[media not shown") && baseline != inline {
        return Err("tool media structured output leaked into a read without tool_media".into());
    }
    if served.to_string().contains("[media not shown") {
        return Err("tool_media kept a marker for nameable media".into());
    }
    let mut actual = Vec::new();
    media(served, &mut actual)?;
    if actual != expected {
        return Err(
            "tool_media references differ from descriptors or are missing/out of order".into(),
        );
    }
    if owned && served != stored {
        return Err("owned-broca tool_media output differs from blocks".into());
    }
    // The short tool text stays inline in this fixture's descriptors read.
    // This comparison assumes opted-in non-media structure matches descriptors;
    // masking only media values keeps adjacent text and metadata checked.
    if !owned && mask_media(served) != mask_media(described) {
        return Err("tool_media non-media structure differs from descriptors; the check assumes they match after masking media references".into());
    }
    Ok(true)
}

pub(super) async fn check<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    route: &S::Route,
    described: &[ReadMessage],
    owned: bool,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let baseline = pages(case, route, MediaMode::MediaRefs, None).await?;
    let inline = pages(case, route, MediaMode::Inline, None).await?;
    let blocks = pages(case, route, MediaMode::Blocks, None).await?;
    let opted = pages_with_tool_media(case, route, MediaMode::MediaRefs, None, true).await?;
    let (outside, actual) = split(case.subject, &opted, owned)?;
    let (normal, baseline) = split(case.subject, &baseline, owned)?;
    let (_, inline) = split(case.subject, &inline, owned)?;
    let (_, blocks) = split(case.subject, &blocks, owned)?;
    let (_, described) = split(case.subject, described, owned)?;
    if outside != normal || actual.keys().ne(baseline.keys()) {
        return Err("tool_media changed a payload or identity outside tool output".into());
    }
    let mut found = false;
    for (key, served) in actual {
        found |= check_output(
            &served,
            &baseline[&key],
            inline.get(&key).ok_or("inline omitted tool output")?,
            blocks.get(&key).ok_or("blocks omitted tool output")?,
            described
                .get(&key)
                .ok_or("descriptors omitted tool output")?,
            owned,
        )?;
    }
    if !found {
        return Err(
            "tool_media fixture exposed no nameable tool media; supply the output-pointer hook"
                .into(),
        );
    }
    let again = pages(case, route, MediaMode::MediaRefs, None).await?;
    let (again_outside, again_outputs) = split(case.subject, &again, owned)?;
    if again_outside != normal || again_outputs != baseline {
        return Err("tool_media changed subsequent reads without the member".into());
    }
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_media_refs_owned_output_equals_blocks_and_keeps_text() {
        let reference = json!({"sha256":"1".repeat(64),"mime":"image/png","kind":"image","byte_size":4,"width":1,"height":1});
        let stored = json!({"kind":"parts","parts":[{"kind":"text","text":"before"},{"kind":"media","media":reference},{"kind":"text","text":"after"}]});
        let described = json!({"kind":"parts","parts":[{"kind":"text","text":"before"},{"kind":"media","media":{
            "type":"media_ref","sha256":"1".repeat(64),"mime":"image/png","kind":"image","byte_size":4,"width":1,"height":1
        }},{"kind":"text","text":"after"}]});
        let baseline = json!({"kind":"text","text":"before\n[media not shown: kind=image reason=not_requested]\nafter"});
        assert_eq!(
            check_output(&stored, &baseline, &baseline, &stored, &described, true),
            Ok(true)
        );
        let mut changed = stored.clone();
        changed["parts"][0]["text"] = json!("different");
        assert!(
            check_output(&changed, &baseline, &baseline, &stored, &described, true)
                .unwrap_err()
                .contains("differs from blocks")
        );
        assert!(
            check_output(&baseline, &baseline, &baseline, &stored, &described, true)
                .unwrap_err()
                .contains("kept a marker")
        );
        assert!(
            check_output(&stored, &stored, &baseline, &stored, &described, true)
                .unwrap_err()
                .contains("leaked")
        );
        changed = stored.clone();
        changed["parts"][1]["media"]["mime"] = json!("image/jpeg");
        assert!(
            check_output(&changed, &baseline, &baseline, &stored, &described, true)
                .unwrap_err()
                .contains("references differ")
        );
        changed = stored.clone();
        changed["parts"][1]["media"]["data"] = json!("AAAA");
        assert!(check_output(&changed, &baseline, &baseline, &stored, &described, true).is_err());
    }

    #[test]
    fn tool_media_refs_outputs_without_media_are_byte_unchanged() {
        let plain = json!({"kind":"text","text":"ok"});
        assert_eq!(
            check_output(&plain, &plain, &plain, &plain, &plain, true),
            Ok(false)
        );
        assert!(check_output(&json!("changed"), &plain, &plain, &plain, &plain, false).is_err());
    }
}
