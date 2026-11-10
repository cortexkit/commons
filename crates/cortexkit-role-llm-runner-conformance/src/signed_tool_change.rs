//! Tool-set invalidation checked against raw scripted-model ingress bytes.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::{
    cases::{Case, Ending},
    route::RunnerRoute,
    subject::LlmRunnerSubject,
};

/// The isolated tool change the adapter performs on a prefix-bound route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignedToolChange {
    Edit,
    Reorder,
    DeferredAddition,
    ToolNote,
    Resume,
}

/// Raw JSON bodies captured outside the runner at the scripted model's ingress.
/// Tools use the Messages shape: top-level `tools`, each with `name`,
/// `description`, `input_schema`, and optional `defer_loading`. Preserve bytes,
/// including whitespace and member order; never parse and reserialize a capture.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SignedToolChangeObservation {
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    /// The same pending request rendered after restart, with no new input or
    /// committed model response. Required only for `Resume`.
    pub resumed: Option<Vec<u8>>,
    /// Unique literal ASCII markers in signed reasoning text and its signature.
    /// Supply at least two, using only letters, digits, `_` or `-`, so JSON
    /// escaping cannot hide them from byte-level checks.
    pub signed_markers: Vec<String>,
    /// A separate marker in ordinary transcript text that must survive stripping.
    pub unsigned_marker: String,
}

type Tool = (String, String, Vec<u8>);

#[derive(serde::Deserialize)]
struct RawPrefix<'a> {
    #[serde(borrow)]
    tools: &'a serde_json::value::RawValue,
    #[serde(borrow)]
    messages: Vec<&'a serde_json::value::RawValue>,
}

fn tools(body: &Value) -> Result<Vec<Tool>, String> {
    body["tools"]
        .as_array()
        .ok_or_else(|| "raw model request has no tools array".to_owned())?
        .iter()
        .map(|tool| {
            let name = tool["name"].as_str().ok_or("tool lacks a name")?;
            let description = tool["description"]
                .as_str()
                .ok_or("tool lacks a description")?;
            let schema = tool.get("input_schema").ok_or("tool lacks input_schema")?;
            Ok((
                name.to_owned(),
                description.to_owned(),
                serde_jcs::to_vec(schema).map_err(|e| e.to_string())?,
            ))
        })
        .collect()
}

fn contains(bytes: &[u8], marker: &str) -> bool {
    !marker.is_empty()
        && bytes
            .windows(marker.len())
            .any(|part| part == marker.as_bytes())
}

fn marker_valid(marker: &str) -> bool {
    !marker.is_empty()
        && marker
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn references(value: &Value, name: &str) -> bool {
    match value {
        Value::String(text) => text.contains(name),
        Value::Array(values) => values.iter().any(|value| references(value, name)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(name) || references(value, name)),
        _ => false,
    }
}

fn check(
    change: SignedToolChange,
    observation: &SignedToolChangeObservation,
) -> Result<(), String> {
    let before: Value = serde_json::from_slice(&observation.before).map_err(|e| e.to_string())?;
    let mut after: Value = serde_json::from_slice(&observation.after).map_err(|e| e.to_string())?;
    let before_tools = tools(&before)?;
    let after_tools = tools(&after)?;
    let before_set: BTreeSet<_> = before_tools.iter().cloned().collect();
    let after_set: BTreeSet<_> = after_tools.iter().cloned().collect();
    if before_set.is_empty()
        || before_set.len() != before_tools.len()
        || after_set.len() != after_tools.len()
    {
        return Err("fixture needs a nonempty initial tool set without duplicate triples".into());
    }
    match change {
        SignedToolChange::Edit | SignedToolChange::Resume => {
            if !before_tools
                .iter()
                .any(|old| after_tools.iter().any(|new| old.0 == new.0 && old != new))
            {
                return Err("fixture did not edit a tool description or canonical schema".into());
            }
        }
        SignedToolChange::Reorder => {
            if before_set != after_set || before_tools == after_tools {
                return Err(
                    "fixture must reorder tools without changing their canonical triples".into(),
                );
            }
        }
        SignedToolChange::DeferredAddition => {
            if !before_set.is_subset(&after_set) || before_set == after_set {
                return Err("fixture must only add tools".into());
            }
            let added: Vec<_> = after_set.difference(&before_set).collect();
            for tool in &added {
                if before_tools.iter().any(|old| old.0 == tool.0)
                    || !after["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|raw| raw["name"] == tool.0 && raw["defer_loading"] == true)
                {
                    return Err("addition is not a new defer_loading: true tool".into());
                }
            }
            after["tools"]
                .as_array_mut()
                .unwrap()
                .retain(|tool| !added.iter().any(|added| tool["name"] == added.0));
            if added.iter().any(|tool| references(&after, &tool.0)) {
                return Err("request references a newly added deferred tool".into());
            }
        }
        SignedToolChange::ToolNote => {
            let raw_before: RawPrefix<'_> =
                serde_json::from_slice(&observation.before).map_err(|e| e.to_string())?;
            let raw_after: RawPrefix<'_> =
                serde_json::from_slice(&observation.after).map_err(|e| e.to_string())?;
            if raw_before.tools.get() != raw_after.tools.get()
                || raw_after.messages.len() != raw_before.messages.len() + 1
                || raw_before
                    .messages
                    .iter()
                    .zip(&raw_after.messages)
                    .any(|(before, after)| before.get() != after.get())
                || after["messages"].as_array().unwrap().last().unwrap()["role"] != "system"
                || after["messages"].as_array().unwrap().last().unwrap()["content"]
                    .as_array()
                    .is_none_or(Vec::is_empty)
            {
                return Err("fixture must append a system tool note with byte-identical tools and earlier messages".into());
            }
            let note = &after["messages"].as_array().unwrap().last().unwrap()["content"];
            if !before["tools"].as_array().unwrap().iter().any(|tool| {
                tool["defer_loading"] == true && references(note, tool["name"].as_str().unwrap())
            }) {
                return Err("fixture must surface an already-declared deferred tool".into());
            }
        }
    }
    if observation.signed_markers.len() < 2
        || observation
            .signed_markers
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != observation.signed_markers.len()
        || !marker_valid(&observation.unsigned_marker)
        || observation.signed_markers.iter().any(|marker| {
            !marker_valid(marker)
                || marker == &observation.unsigned_marker
                || !contains(&observation.before, marker)
        })
    {
        return Err("fixture lacks signed reasoning/signature markers before the change".into());
    }
    if !contains(&observation.before, &observation.unsigned_marker)
        || !contains(&observation.after, &observation.unsigned_marker)
    {
        return Err("ordinary transcript text did not survive the tool change".into());
    }
    let strip = matches!(
        change,
        SignedToolChange::Edit | SignedToolChange::DeferredAddition | SignedToolChange::Resume
    );
    for marker in &observation.signed_markers {
        if contains(&observation.after, marker) == strip {
            return Err(if strip {
                "tool change retained earlier signed reasoning or its signature".into()
            } else {
                "tool reorder or appended tool note stripped signed reasoning".into()
            });
        }
    }
    if change == SignedToolChange::Resume
        && observation.resumed.as_ref() != Some(&observation.after)
    {
        return Err("resume did not render byte-identical request bytes".into());
    }
    Ok(())
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let change = match name {
        "signed_tool_edit_strips" => SignedToolChange::Edit,
        "signed_tool_reorder_keeps" => SignedToolChange::Reorder,
        "signed_tool_deferred_addition_strips" => SignedToolChange::DeferredAddition,
        "signed_tool_note_keeps" => SignedToolChange::ToolNote,
        "signed_tool_resume_identical" => SignedToolChange::Resume,
        _ => return Err(format!("unknown signed tool change case {name}")),
    };
    let Some(observed) = case
        .subject
        .signed_tool_change_observation(&case.mint.next(name), change)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no prefix-bound signed-tool-change fixture".into(),
        ));
    };
    check(change, &observed)?;
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FakeRunner {
        faulty: bool,
    }

    impl FakeRunner {
        fn execute(&self, change: SignedToolChange) -> SignedToolChangeObservation {
            let first =
                json!({"name":"first","description":"first tool","input_schema":{"type":"object"}});
            let mut second = json!({"name":"second","description":"second tool","input_schema":{"type":"object"}});
            if change == SignedToolChange::ToolNote {
                second["defer_loading"] = json!(true);
            }
            let before = json!({
                "tools":[first,second],
                "messages":[{"role":"assistant","content":[
                    {"type":"thinking","thinking":"SIGNED_TEXT","signature":"SIGNED_SIGNATURE"},
                    {"type":"text","text":"ORDINARY_TEXT"}
                ]}]
            });
            let mut after = before.clone();
            match change {
                SignedToolChange::Edit | SignedToolChange::Resume => {
                    after["tools"][0]["description"] = json!("edited tool");
                }
                SignedToolChange::Reorder => {
                    after["tools"].as_array_mut().unwrap().reverse();
                }
                SignedToolChange::DeferredAddition => {
                    after["tools"].as_array_mut().unwrap().push(json!({
                        "name":"deferred","description":"deferred tool",
                        "input_schema":{"type":"object"},"defer_loading":true
                    }));
                }
                SignedToolChange::ToolNote => {
                    after["messages"].as_array_mut().unwrap().push(json!({
                        "role":"system", "content":[{"type":"text","text":"Tool addition: second is now available."}]
                    }));
                }
            }
            let strip = matches!(
                change,
                SignedToolChange::Edit
                    | SignedToolChange::DeferredAddition
                    | SignedToolChange::Resume
            );
            if strip != (self.faulty && change != SignedToolChange::Resume) {
                after["messages"][0]["content"]
                    .as_array_mut()
                    .unwrap()
                    .remove(0);
            }
            let after = serde_json::to_vec(&after).unwrap();
            let resumed = (change == SignedToolChange::Resume).then(|| {
                let mut bytes = after.clone();
                if self.faulty {
                    bytes.push(b' ');
                }
                bytes
            });
            SignedToolChangeObservation {
                before: serde_json::to_vec(&before).unwrap(),
                after,
                resumed,
                signed_markers: vec!["SIGNED_TEXT".into(), "SIGNED_SIGNATURE".into()],
                unsigned_marker: "ORDINARY_TEXT".into(),
            }
        }
    }

    #[test]
    fn signed_tool_change_faulty_runners_fail_each_named_case() {
        for (name, change, expected) in [
            (
                "signed_tool_edit_strips",
                SignedToolChange::Edit,
                "retained earlier",
            ),
            (
                "signed_tool_reorder_keeps",
                SignedToolChange::Reorder,
                "stripped signed",
            ),
            (
                "signed_tool_deferred_addition_strips",
                SignedToolChange::DeferredAddition,
                "retained earlier",
            ),
            (
                "signed_tool_note_keeps",
                SignedToolChange::ToolNote,
                "byte-identical",
            ),
            (
                "signed_tool_resume_identical",
                SignedToolChange::Resume,
                "byte-identical",
            ),
        ] {
            check(change, &FakeRunner { faulty: false }.execute(change)).unwrap();
            let failure = check(change, &FakeRunner { faulty: true }.execute(change))
                .map_err(|error| format!("{name}: {error}"))
                .unwrap_err();
            assert!(
                failure.starts_with(name) && failure.contains(expected),
                "{failure}"
            );
        }
    }

    #[test]
    fn signed_tool_change_deferred_addition_requires_stripping() {
        let change = SignedToolChange::DeferredAddition;
        let mut observed = FakeRunner { faulty: false }.execute(change);
        let before: Value = serde_json::from_slice(&observed.before).unwrap();
        let mut after: Value = serde_json::from_slice(&observed.after).unwrap();
        after["messages"][0]["content"] = json!([{"type":"text","text":"ORDINARY_TEXT"}]);
        observed.after = serde_json::to_vec(&after).unwrap();
        check(change, &observed).expect("a deferred addition must strip earlier signed reasoning");

        after["messages"] = before["messages"].clone();
        observed.after = serde_json::to_vec(&after).unwrap();
        assert!(check(change, &observed)
            .unwrap_err()
            .contains("retained earlier"));
    }

    #[test]
    fn signed_tool_change_canonical_schemas_ignore_key_order_and_number_spelling() {
        let mut observed = FakeRunner { faulty: false }.execute(SignedToolChange::Reorder);
        let before = String::from_utf8(observed.before).unwrap();
        observed.before = before
            .replace("{\"type\":\"object\"}", "{\"b\":1.0,\"a\":true}")
            .into_bytes();
        let after = String::from_utf8(observed.after).unwrap();
        observed.after = after
            .replace("{\"type\":\"object\"}", "{\"a\":true,\"b\":1}")
            .into_bytes();
        check(SignedToolChange::Reorder, &observed).unwrap();
        check(SignedToolChange::Edit, &observed).unwrap_err();
    }

    #[test]
    fn signed_tool_change_rejects_invalid_or_vacuous_fixtures() {
        let mut observed = FakeRunner { faulty: false }.execute(SignedToolChange::DeferredAddition);
        let mut body: Value = serde_json::from_slice(&observed.after).unwrap();
        body["tool_choice"] = json!({"type":"tool","name":"deferred"});
        observed.after = serde_json::to_vec(&body).unwrap();
        assert!(check(SignedToolChange::DeferredAddition, &observed)
            .unwrap_err()
            .contains("references"));
        body["tool_choice"] = Value::Null;
        body["tools"][2]["defer_loading"] = json!(false);
        observed.after = serde_json::to_vec(&body).unwrap();
        assert!(check(SignedToolChange::DeferredAddition, &observed)
            .unwrap_err()
            .contains("defer_loading"));
        let mut observed = FakeRunner { faulty: false }.execute(SignedToolChange::Edit);
        observed.before = observed.after.clone();
        assert!(check(SignedToolChange::Edit, &observed).is_err());
        let mut observed = FakeRunner { faulty: false }.execute(SignedToolChange::Edit);
        observed.signed_markers.clear();
        assert!(check(SignedToolChange::Edit, &observed)
            .unwrap_err()
            .contains("markers"));
    }

    #[test]
    fn signed_tool_change_note_requires_byte_identical_prefix() {
        let change = SignedToolChange::ToolNote;
        let observed = FakeRunner { faulty: false }.execute(change);
        check(change, &observed).unwrap();
        for original in ["\"tools\":[", "\"messages\":[{"] {
            let mut changed = observed.clone();
            changed.after = String::from_utf8(changed.after)
                .unwrap()
                .replacen(original, &format!("{original} "), 1)
                .into_bytes();
            // Whitespace inside a captured array or object still changes the prefix bytes.
            assert!(check(change, &changed)
                .unwrap_err()
                .contains("byte-identical"));
        }
        let mut missing_tool = observed.clone();
        missing_tool.after = String::from_utf8(missing_tool.after)
            .unwrap()
            .replace("Tool addition: second", "Tool addition: unknown")
            .into_bytes();
        assert!(check(change, &missing_tool)
            .unwrap_err()
            .contains("already-declared"));
    }
}
