//! Stored tool outputs in the `owned-broca` serializer profile.
//!
//! The view describes the stored result alone. Family transport wrappers,
//! notices, part labels and view rewrites do not belong to this conversion.

use super::error_output::ErrorOutput;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

/// A stored media part as `media: "blocks"` serves it, without media bytes.
/// Unlike a descriptor reference, this value has no `type` or part key.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MediaRef {
    pub sha256: String,
    pub mime: String,
    pub kind: String,
    pub byte_size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
}

impl MediaRef {
    pub fn new(
        sha256: impl Into<String>,
        mime: impl Into<String>,
        kind: impl Into<String>,
        byte_size: u64,
    ) -> Self {
        Self {
            sha256: sha256.into(),
            mime: mime.into(),
            kind: kind.into(),
            byte_size,
            width: None,
            height: None,
        }
    }

    pub fn with_dimensions(mut self, width: u64, height: u64) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }
}

/// Text and ordered media references from one stored tool output.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolOutputView {
    pub text: String,
    pub media: Vec<MediaRef>,
    pub is_error: bool,
}

impl ToolOutputView {
    pub fn new(text: impl Into<String>, is_error: bool) -> Self {
        Self {
            text: text.into(),
            media: Vec::new(),
            is_error,
        }
    }

    pub fn with_media(mut self, media: Vec<MediaRef>) -> Self {
        self.media = media;
        self
    }
}

/// A malformed stored output, with the member at fault and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Violation {
    pub member: String,
    pub reason: String,
}

impl Violation {
    fn new(member: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            member: member.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "owned-broca tool output member {}: {}",
            self.member, self.reason
        )
    }
}

impl std::error::Error for Violation {}

/// Convert a stored `output` envelope and its sibling tool-result `is_error`.
/// Failed text results retain that flag; `denied` and `error` always imply an
/// error. Text parts join with `\n`; media contributes references, never a
/// text omission marker. JSON uses compact JSON text, not pretty printing.
pub fn tool_output(output: &Value, is_error: bool) -> Result<ToolOutputView, Violation> {
    let object = output
        .as_object()
        .ok_or_else(|| Violation::new("output", "must be an object"))?;
    let kind = string(object, "kind")?;
    let text = match kind {
        "text" => {
            members(object, &["kind", "text"])?;
            string(object, "text")?.to_owned()
        }
        "json" => {
            members(object, &["kind", "value"])?;
            let value = object
                .get("value")
                .ok_or_else(|| Violation::new("value", "is required"))?;
            serde_json::to_string(value)
                .map_err(|error| Violation::new("value", error.to_string()))?
        }
        "denied" => {
            members(object, &["kind", "reason"])?;
            format!("denied: {}", string(object, "reason")?)
        }
        "error" => ErrorOutput::decode(output)
            .map_err(|error| Violation::new(error.member.clone(), error.to_string()))?
            .render(),
        "parts" => {
            members(object, &["kind", "parts"])?;
            let parts = object
                .get("parts")
                .and_then(Value::as_array)
                .ok_or_else(|| Violation::new("parts", "must be an array"))?;
            let mut text = Vec::new();
            let mut media = Vec::new();
            for (index, part) in parts.iter().enumerate() {
                let prefix = format!("parts[{index}]");
                let result = (|| {
                    let object = part
                        .as_object()
                        .ok_or_else(|| Violation::new("part", "must be an object"))?;
                    match string(object, "kind")? {
                        "text" => {
                            members(object, &["kind", "text"])?;
                            text.push(string(object, "text")?.to_owned());
                        }
                        "media" => {
                            members(object, &["kind", "media"])?;
                            let value = object
                                .get("media")
                                .ok_or_else(|| Violation::new("media", "is required"))?;
                            let reference: MediaRef = serde_json::from_value(value.clone())
                                .map_err(|error| Violation::new("media", error.to_string()))?;
                            if !crate::content_digest::is_sha256_hex(&reference.sha256) {
                                return Err(Violation::new(
                                    "media.sha256",
                                    "must be a lowercase SHA-256 digest",
                                ));
                            }
                            media.push(reference);
                        }
                        _ => return Err(Violation::new("kind", "must be text or media")),
                    }
                    Ok(())
                })();
                result.map_err(|error: Violation| {
                    Violation::new(format!("{prefix}.{}", error.member), error.reason)
                })?;
            }
            return Ok(ToolOutputView::new(text.join("\n"), is_error).with_media(media));
        }
        _ => return Err(Violation::new("kind", "unknown stored output kind")),
    };
    Ok(ToolOutputView::new(
        text,
        is_error || matches!(kind, "denied" | "error"),
    ))
}

fn string<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str, Violation> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Violation::new(name, "must be a string"))
}

fn members(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), Violation> {
    for name in object.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(Violation::new(name, "unknown member"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    const CASES: &str = include_str!("../../test-vectors/owned-broca-tool-output/cases.json");

    #[test]
    fn tool_output_recorded_views() {
        let cases: Vec<Value> = serde_json::from_str(CASES).unwrap();
        assert_eq!(cases.len(), 10);
        for case in cases {
            let actual = tool_output(&case["output"], case["is_error"].as_bool().unwrap())
                .unwrap_or_else(|error| panic!("{}: {error}", case["id"]));
            let expected: ToolOutputView =
                serde_json::from_value(case["expected"].clone()).unwrap();
            assert_eq!(actual, expected, "{}", case["id"]);
        }
    }

    #[test]
    fn tool_output_vectors_match_checksums() {
        let files = [
            ("cases.json", CASES),
            (
                "README.md",
                include_str!("../../test-vectors/owned-broca-tool-output/README.md"),
            ),
        ];
        let manifest = include_str!("../../test-vectors/owned-broca-tool-output/SHA256SUMS");
        assert_eq!(manifest.lines().count(), files.len());
        for (line, (name, bytes)) in manifest.lines().zip(files) {
            let (digest, listed) = line.split_once("  ").unwrap();
            assert_eq!(listed, name);
            assert_eq!(digest, format!("{:x}", Sha256::digest(bytes.as_bytes())));
        }
    }

    #[test]
    fn tool_output_flags_and_json_values() {
        for (output, expected, implied_error) in [
            (json!({"kind":"text","text":"ok"}), "ok", false),
            (json!({"kind":"json","value":null}), "null", false),
            (json!({"kind":"json","value":"x\ny"}), "\"x\\ny\"", false),
            (json!({"kind":"json","value":[true,2]}), "[true,2]", false),
            (json!({"kind":"denied","reason":"no"}), "denied: no", true),
            (json!({"kind":"error","message":"bad"}), "bad", true),
            (json!({"kind":"parts","parts":[]}), "", false),
        ] {
            for flag in [false, true] {
                let view = tool_output(&output, flag).unwrap();
                assert_eq!(view.text, expected);
                assert_eq!(view.is_error, flag || implied_error);
                assert!(view.media.is_empty());
            }
        }
    }

    #[test]
    fn tool_output_media_order_and_empty_text_parts() {
        let first =
            json!({"sha256":"1".repeat(64),"mime":"image/png","kind":"image","byte_size":3});
        let second =
            json!({"sha256":"2".repeat(64),"mime":"audio/wav","kind":"audio","byte_size":4});
        let output = json!({"kind":"parts","parts":[
            {"kind":"text","text":""}, {"kind":"media","media":first},
            {"kind":"text","text":"middle"}, {"kind":"media","media":second},
            {"kind":"text","text":""}
        ]});
        let view = tool_output(&output, true).unwrap();
        assert_eq!(view.text, "\nmiddle\n");
        assert!(view.is_error);
        assert_eq!(
            serde_json::to_value(view.media).unwrap(),
            json!([first, second])
        );
    }

    #[test]
    fn tool_output_malformed_envelopes_are_violations() {
        for (output, member) in [
            (json!(null), "output"),
            (json!([]), "output"),
            (json!({}), "kind"),
            (json!({"kind":3}), "kind"),
            (json!({"kind":"unknown"}), "kind"),
            (json!({"kind":"text"}), "text"),
            (json!({"kind":"text","text":null}), "text"),
            (
                json!({"kind":"text","text":"x","is_error":true}),
                "is_error",
            ),
            (
                json!({"output":{"kind":"text","text":"x"},"is_error":true}),
                "kind",
            ),
            (json!({"kind":"json"}), "value"),
            (json!({"kind":"denied","reason":false}), "reason"),
            (
                json!({"kind":"error","message":"x","nothing_sent":false}),
                "nothing_sent",
            ),
            (json!({"kind":"parts","parts":null}), "parts"),
            (json!({"kind":"parts","parts":[null]}), "parts[0].part"),
            (
                json!({"kind":"parts","parts":[{"kind":"unknown"}]}),
                "parts[0].kind",
            ),
            (
                json!({"kind":"parts","parts":[{"kind":"text","text":5}]}),
                "parts[0].text",
            ),
            (
                json!({"kind":"parts","parts":[{"kind":"media"}]}),
                "parts[0].media",
            ),
        ] {
            let error = tool_output(&output, false).unwrap_err();
            assert_eq!(error.member, member, "{output}");
            assert!(error.to_string().contains(member));
        }
        let reference =
            json!({"sha256":"1".repeat(64),"mime":"image/png","kind":"image","byte_size":3});
        for (key, bad) in [
            ("sha256", json!("bad")),
            ("byte_size", json!(-1)),
            ("width", json!("wide")),
            ("data", json!("AAAA")),
        ] {
            let mut media = reference.clone();
            media[key] = bad;
            assert!(
                tool_output(
                    &json!({"kind":"parts","parts":[{"kind":"media","media":media}]}),
                    false
                )
                .is_err(),
                "{key}"
            );
        }
    }
}
