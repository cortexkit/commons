//! Stable trailing turn directives from stored model text (§9.9).
//!
//! Parsing never rewrites the text and imposes no scheduling range policy.

use serde::{Deserialize, Serialize};

/// A directive name accepted at admission. A new grammar needs a new name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveName {
    Wake,
}

/// Closed reasons produced by the wake parser, not request refusals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeError {
    BadDuration,
    NoteTooLong,
    BadNote,
    Malformed,
}

impl WakeError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadDuration => "bad_duration",
            Self::NoteTooLong => "note_too_long",
            Self::BadNote => "bad_note",
            Self::Malformed => "malformed",
        }
    }
}

/// Consumer-side error decoding keeps future reasons, like a run's `reason`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum DirectiveError {
    Known(WakeError),
    Unknown(String),
}

impl From<String> for DirectiveError {
    fn from(value: String) -> Self {
        match serde_json::from_value(serde_json::Value::String(value.clone())) {
            Ok(reason) => Self::Known(reason),
            Err(_) => Self::Unknown(value),
        }
    }
}
impl From<DirectiveError> for String {
    fn from(value: DirectiveError) -> Self {
        match value {
            DirectiveError::Known(reason) => reason.as_str().into(),
            DirectiveError::Unknown(reason) => reason,
        }
    }
}

/// A parsed wake tag, including an explicit parser error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WakeDirective {
    In { in_ms: u64, note: Option<String> },
    Off,
    Error(WakeError),
}

/// A lenient directive result on `run.result` and `run.status`.
/// Unknown names and error reasons remain readable by older consumers.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Directive {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<DirectiveError>,
}

impl From<WakeDirective> for Directive {
    fn from(value: WakeDirective) -> Self {
        let mut result = Self {
            name: "wake".into(),
            in_ms: None,
            note: None,
            off: None,
            error: None,
        };
        match value {
            WakeDirective::In { in_ms, note } => {
                result.in_ms = Some(in_ms);
                result.note = note;
            }
            WakeDirective::Off => result.off = Some(true),
            WakeDirective::Error(reason) => result.error = Some(DirectiveError::Known(reason)),
        }
        result
    }
}

/// Parse the last non-whitespace content, outside fenced code. An unclosed
/// fence consumes the rest of the text. Only a trailing `<wake` candidate
/// produces a result; a tag followed by other content is not a directive.
pub fn parse_trailing(text: &str) -> Option<WakeDirective> {
    let text = text.trim_end();
    let mut fence: Option<(u8, usize)> = None;
    let mut candidate = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = trimmed.as_bytes().first().copied();
        let count = trimmed.bytes().take_while(|b| Some(*b) == marker).count();
        if matches!(marker, Some(b'`' | b'~')) && count >= 3 {
            match fence {
                None => {
                    fence = Some((marker.unwrap(), count));
                    candidate = None;
                }
                Some((kind, length))
                    if marker == Some(kind)
                        && count >= length
                        && trimmed[count..].trim().is_empty() =>
                {
                    fence = None;
                }
                _ => {}
            }
        } else if fence.is_none() {
            for (index, _) in line.match_indices("<wake") {
                let next = offset + index;
                let inside_note = candidate.is_some_and(|previous| {
                    note_body(&text[previous..next]).is_some_and(|body| !body.contains("</wake>"))
                });
                if !inside_note {
                    candidate = Some(next);
                }
            }
        }
        offset += line.len();
    }
    if fence.is_some() {
        return None;
    }
    let suffix = &text[candidate?..];
    // A complete tag followed by prose is mid-text, even when its attributes
    // are invalid. Do not turn a quoted example into a request to schedule.
    let end = if note_body(suffix).is_some() && suffix.ends_with("</wake>") {
        None
    } else {
        suffix
            .find('>')
            .filter(|end| suffix[..*end].ends_with('/'))
            .map(|n| n + 1)
            .or_else(|| suffix.find("</wake>").map(|n| n + "</wake>".len()))
    };
    if end.is_some_and(|end| !suffix[end..].trim().is_empty()) {
        return None;
    }
    Some(parse_wake(suffix).unwrap_or_else(WakeDirective::Error))
}

fn note_body(tag: &str) -> Option<&str> {
    let (_, rest) = tag.strip_prefix("<wake in=\"")?.split_once('"')?;
    rest.strip_prefix(' ').unwrap_or(rest).strip_prefix('>')
}

fn parse_wake(tag: &str) -> Result<WakeDirective, WakeError> {
    if matches!(tag, "<wake off/>" | "<wake off />") {
        return Ok(WakeDirective::Off);
    }
    let body = tag
        .strip_prefix("<wake in=\"")
        .ok_or(WakeError::Malformed)?;
    let (duration, rest) = body.split_once('"').ok_or(WakeError::Malformed)?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let note = if rest == "/>" {
        None
    } else if let Some(body) = rest
        .strip_prefix('>')
        .and_then(|s| s.strip_suffix("</wake>"))
    {
        let note = body.trim();
        if note.chars().any(|c| c.is_control() || c == '<') {
            return Err(WakeError::BadNote);
        }
        if note.len() > 200 {
            return Err(WakeError::NoteTooLong);
        }
        (!note.is_empty()).then(|| note.to_owned())
    } else {
        return Err(WakeError::Malformed);
    };
    let bytes = duration.as_bytes();
    let (unit, digits) = bytes.split_last().ok_or(WakeError::BadDuration)?;
    if digits.is_empty() || digits.len() > 9 || !digits.iter().all(u8::is_ascii_digit) {
        return Err(WakeError::BadDuration);
    }
    let multiplier = match unit {
        b's' => 1_000,
        b'm' => 60_000,
        b'h' => 3_600_000,
        _ => return Err(WakeError::BadDuration),
    };
    let amount = digits
        .iter()
        .try_fold(0u64, |n, d| {
            n.checked_mul(10)?.checked_add(u64::from(d - b'0'))
        })
        .ok_or(WakeError::BadDuration)?;
    let in_ms = amount
        .checked_mul(multiplier)
        .filter(|n| *n != 0)
        .ok_or(WakeError::BadDuration)?;
    Ok(WakeDirective::In { in_ms, note })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        run::{join_text_parts, RunResult, RunStatus},
        send::SendRequest,
        vectors,
    };
    use serde_json::{json, Value};

    fn wire(text: &str) -> Value {
        parse_trailing(text)
            .map(Directive::from)
            .map(|d| serde_json::to_value(d).unwrap())
            .unwrap_or(Value::Null)
    }

    #[test]
    fn every_directive_vector_parses() {
        for (name, _) in vectors::FILES
            .iter()
            .filter(|(name, _)| name.starts_with("directives/") && !name.ends_with("wire.json"))
        {
            let case = vectors::load(name);
            assert_eq!(
                wire(case["text"].as_str().unwrap()),
                case["expected"],
                "{name}"
            );
            if let Some(parts) = case["parts"].as_array() {
                let text = join_text_parts(
                    parts
                        .iter()
                        .filter(|part| part["type"] == "text")
                        .map(|part| part["text"].as_str().unwrap()),
                );
                assert_eq!(text, case["text"], "{name}: joined final_message.text");
            }
        }
    }

    #[test]
    fn unclosed_fence_hides_wake() {
        for text in [
            "```\n<wake in=\"90s\"/>",
            "~~~rust\n<wake off/>",
            "<wake off/>\n```\n<wake off/>",
        ] {
            assert_eq!(parse_trailing(text), None, "{text}");
        }
    }

    #[test]
    fn fence_matching_and_trailing_content() {
        for text in [
            "```\n<wake off/>\n```",
            "```\n~~~\n<wake off/>",
            "````\n```\n<wake off/>",
            "<wake off/>\nmore",
            "<wake bad/> more",
            "ordinary text",
            "",
        ] {
            assert_eq!(parse_trailing(text), None, "{text}");
        }
        for text in [
            "```rust\nexample\n```\n<wake off/>",
            "  ~~~\nexample\n  ~~~\n<wake off/>",
            "<wake in=\"1s\"/>\n<wake off/>",
        ] {
            assert_eq!(parse_trailing(text), Some(WakeDirective::Off), "{text}");
        }
    }

    #[test]
    fn exact_spacing_and_duration_grammar() {
        for text in ["<wake off/>", "<wake off />"] {
            assert_eq!(parse_trailing(text), Some(WakeDirective::Off));
        }
        for text in [
            "<wake in=\"1s\"/>",
            "<wake in=\"1s\" />",
            "<wake in=\"1s\"></wake>",
            "<wake in=\"1s\" >  </wake>",
        ] {
            assert_eq!(
                parse_trailing(text),
                Some(WakeDirective::In {
                    in_ms: 1000,
                    note: None
                }),
                "{text}"
            );
        }
        for text in [
            "<wake  off/>",
            "<wake\toff/>",
            "<wake off  />",
            "<wake off></wake>",
            "<wake in='1s'/>",
            "<wake in =\"1s\"/>",
            "<wake in=\"1s\"  />",
            "<wake in=\"1s\" extra=\"x\"/>",
            "<wake/>",
            "<wakeup/>",
            "<wake in=\"1s\">x",
            "<wake in=\"1s\">x</wake >",
        ] {
            assert_eq!(
                parse_trailing(text),
                Some(WakeDirective::Error(WakeError::Malformed)),
                "{text}"
            );
        }
        for duration in [
            "",
            "0s",
            "000s",
            "+1s",
            "-1s",
            "1.5s",
            "١s",
            "1S",
            "1d",
            "1",
            "1000000000s",
            "1 s",
        ] {
            assert_eq!(
                parse_trailing(&format!("<wake in=\"{duration}\"/>")),
                Some(WakeDirective::Error(WakeError::BadDuration)),
                "{duration}"
            );
        }
        assert_eq!(
            wire("<wake in=\"999999999h\"/>")["in_ms"],
            3_599_999_996_400_000u64
        );
        assert_eq!(wire("<wake in=\"000000001m\"/>")["in_ms"], 60_000);
    }

    #[test]
    fn note_bytes_and_controls() {
        assert_eq!(wire("<wake in=\"3m\">  café  </wake>")["note"], "café");
        for note in [
            "a\nb",
            "a\tb",
            "a\0b",
            "a\u{7f}b",
            "a\u{85}b",
            "a<b",
            "a<wake off/>",
        ] {
            assert_eq!(
                parse_trailing(&format!("<wake in=\"1s\">{note}</wake>")),
                Some(WakeDirective::Error(WakeError::BadNote))
            );
        }
        assert_eq!(
            wire("<wake in=\"1s\">literal /> okay</wake>")["note"],
            "literal /> okay"
        );
        assert!(wire("<wake in=\"1s\"> \n </wake>").get("note").is_none());
    }

    #[test]
    fn directive_request_and_results_round_trip_without_rewriting() {
        let request: SendRequest =
            vectors::exact_object_round_trip("directives/wire.json", "\"send\": ", 2);
        assert_eq!(request.directives(), Ok(Some(vec![DirectiveName::Wake])));
        assert_eq!(request.check_directives(&[DirectiveName::Wake]), Ok(()));
        assert_eq!(request.check_directives(&[]), Err("directives".into()));
        let built = SendRequest::new("x", "id").with_directives(vec![DirectiveName::Wake]);
        assert_eq!(built.directives(), request.directives());
        assert_eq!(
            SendRequest::new("x", "id").check_directives(&[DirectiveName::Wake]),
            Ok(())
        );
        for raw in [
            json!(null),
            json!("wake"),
            json!(["unknown"]),
            json!(["wake", "wake"]),
            json!([1]),
        ] {
            let mut invalid = built.clone();
            invalid.runner_params.insert("directives".into(), raw);
            let before = serde_json::to_vec(&invalid).unwrap();
            assert_eq!(invalid.directives(), Err("directives".into()));
            assert_eq!(serde_json::to_vec(&invalid).unwrap(), before);
        }
        let file = vectors::load("directives/wire.json");
        let result: RunResult = vectors::round_trip("directive result", &file["result"]);
        let status = RunStatus::new("completed").with_directives(result.directives.unwrap());
        assert_eq!(
            serde_json::to_value(status).unwrap()["directives"],
            file["result"]["directives"]
        );
        let unknown: Directive =
            serde_json::from_value(json!({"name":"future", "error":"future_reason", "extra":true}))
                .unwrap();
        assert_eq!(
            unknown.error,
            Some(DirectiveError::Unknown("future_reason".into()))
        );
        assert!(serde_json::to_value(RunResult::new("id", "active"))
            .unwrap()
            .get("directives")
            .is_none());
        assert!(serde_json::to_value(RunStatus::new("active"))
            .unwrap()
            .get("directives")
            .is_none());
    }
}
