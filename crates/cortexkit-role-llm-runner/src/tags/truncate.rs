//! Profile-exact part truncation for negotiated compaction views.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::serializer_profile::{ANTHROPIC_MESSAGES, OPENCODE_AISDK, OPENCODE_V2, OWNED_BROCA, PI};

/// A truncation mode. Unknown wire values survive decoding so a runner can
/// refuse the view with `truncate_kind`, rather than reject its JSON.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum TruncateMode {
    Placeholder,
    Skeleton,
    EditMarker,
    Unknown(String),
}

impl TruncateMode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Placeholder => "placeholder",
            Self::Skeleton => "skeleton",
            Self::EditMarker => "edit_marker",
            Self::Unknown(value) => value,
        }
    }
}

impl From<String> for TruncateMode {
    fn from(value: String) -> Self {
        match value.as_str() {
            "placeholder" => Self::Placeholder,
            "skeleton" => Self::Skeleton,
            "edit_marker" => Self::EditMarker,
            _ => Self::Unknown(value),
        }
    }
}

impl From<TruncateMode> for String {
    fn from(value: TruncateMode) -> Self {
        value.as_str().to_owned()
    }
}

/// Context derived from the runner's stored row, never from a provider edit.
/// Pi result blocks do not carry their message's role themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PartContainer {
    ToolResult,
    Other,
}

/// The requested mode, part kind or container cannot be rendered exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TruncateRefusal;

impl TruncateRefusal {
    pub const fn reason(&self) -> &'static str {
        "truncate_kind"
    }
}

impl std::fmt::Display for TruncateRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for TruncateRefusal {}

/// Render a negotiated truncate edit without modifying the stored part.
/// `number` is absent when the row has no number or the session plan's
/// `render_tags` is false. Callers render its ordinary tag after truncation, as for a
/// kept part. A container inconsistent with a self-identifying result refuses.
/// On Pi, attachment removal affects only the addressed block. To remove a
/// result's other image blocks, the provider sends a separate `drop` edit for
/// each image part; no edit reaches its siblings.
pub fn truncate_part(
    profile: &str,
    part: &Value,
    container: PartContainer,
    mode: TruncateMode,
    number: Option<u64>,
    drop_attachments: bool,
) -> Result<Value, TruncateRefusal> {
    use TruncateMode::{EditMarker, Placeholder, Skeleton, Unknown};
    let kind = part["type"].as_str().ok_or(TruncateRefusal)?;
    if matches!(mode, Unknown(_)) {
        return Err(TruncateRefusal);
    }
    let result = match profile {
        OPENCODE_AISDK | OPENCODE_V2 => kind == "tool",
        ANTHROPIC_MESSAGES | OWNED_BROCA => kind == "tool_result",
        PI => container == PartContainer::ToolResult && kind != "toolCall",
        _ => return Err(TruncateRefusal),
    };
    if profile != PI && result != (container == PartContainer::ToolResult) {
        return Err(TruncateRefusal);
    }
    if drop_attachments && !result {
        return Err(TruncateRefusal);
    }
    let placeholder = number.map_or_else(|| "[dropped]".to_owned(), super::dropped_placeholder);
    let text = || json!({"type":"text", "text":placeholder});
    let mut out = part.clone();
    match (profile, kind, &mode) {
        (OPENCODE_AISDK | OPENCODE_V2, "text", Placeholder)
        | (PI | ANTHROPIC_MESSAGES, "text", Placeholder)
            if !result =>
        {
            out["text"] = Value::String(placeholder);
        }
        (OPENCODE_AISDK | OPENCODE_V2, "file", Placeholder)
        | (OWNED_BROCA, "text" | "media", Placeholder) => out = text(),
        (OPENCODE_AISDK, "tool", Placeholder | Skeleton | EditMarker) => {
            let state = out
                .get_mut("state")
                .and_then(Value::as_object_mut)
                .ok_or(TruncateRefusal)?;
            let output = if state.get("status").and_then(Value::as_str) == Some("error") {
                "error"
            } else {
                "output"
            };
            state.insert(output.into(), Value::String(placeholder));
            if mode == EditMarker {
                if let Some(input) = state.get_mut("input") {
                    trim_input(input);
                }
            }
            if drop_attachments {
                state.insert("attachments".into(), json!([]));
            }
        }
        (OPENCODE_V2, "tool", Placeholder | Skeleton | EditMarker) => {
            let state = out
                .get_mut("state")
                .and_then(Value::as_object_mut)
                .ok_or(TruncateRefusal)?;
            state.insert("content".into(), json!([text()]));
            if mode == EditMarker {
                if let Some(input) = state.get_mut("input") {
                    trim_input(input);
                }
            }
        }
        (PI, "text" | "image", Placeholder | Skeleton) if result => out = text(),
        (PI, "toolCall", EditMarker) => {
            if let Some(input) = out.get_mut("arguments") {
                trim_input(input);
            }
        }
        (ANTHROPIC_MESSAGES, "tool_result", Placeholder | Skeleton) => {
            out["content"] = Value::String(placeholder);
        }
        (OWNED_BROCA, "tool_result", Placeholder | Skeleton) => {
            out["output"] = json!({"kind":"text", "text":placeholder});
        }
        (ANTHROPIC_MESSAGES, "tool_use", EditMarker) | (OWNED_BROCA, "tool_call", EditMarker) => {
            if let Some(input) = out.get_mut("input") {
                trim_input(input);
            }
        }
        _ => return Err(TruncateRefusal),
    }
    // Replacing a result's content also removes the media embedded in it.
    // opencode-aisdk keeps attachments beside the output, in
    // `state.attachments`, which `drop_attachments` clears in this same edit.
    Ok(out)
}

fn trim_input(input: &mut Value) {
    let Some(object) = input.as_object_mut() else {
        return;
    };
    for key in [
        "oldString",
        "newString",
        "content",
        "old_string",
        "new_string",
    ] {
        let Some(Value::String(value)) = object.get_mut(key) else {
            continue;
        };
        if value.ends_with("...[truncated]") || value.encode_utf16().count() <= 40 {
            continue;
        }
        let mut units: Vec<u16> = value.encode_utf16().take(40).collect();
        if units
            .last()
            .is_some_and(|unit| (0xD800..=0xDBFF).contains(unit))
        {
            units.pop();
        }
        *value = String::from_utf16(&units).expect("prefix ends on a scalar boundary")
            + "...[truncated]";
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_part_matches_every_reference_vector() {
        let file: Value =
            serde_json::from_str(crate::vectors::file("truncate.json").unwrap()).unwrap();
        let cases = file["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 184);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let original = case["part"].clone();
            let result = truncate_part(
                case["profile"].as_str().unwrap(),
                &original,
                serde_json::from_value(case["container"].clone()).unwrap(),
                serde_json::from_value(case["mode"].clone()).unwrap(),
                case["number"].as_u64(),
                case["drop_attachments"].as_bool().unwrap(),
            );
            if let Some(expected) = case.get("expected") {
                assert_eq!(result, Ok(expected.clone()), "{name}");
            } else {
                assert_eq!(result.unwrap_err().reason(), case["refusal"], "{name}");
            }
            assert_eq!(original, case["part"], "{name}: mutated stored part");
        }
    }
}
