//! Shared normalization of the last model step's provider or host finish reason.
//!
//! See CONTRACT.md §6.6 for the source spellings, raw inputs and provider docs.

use serde::{Deserialize, Serialize};

/// A normalized model-step finish. Later wire values decode as
/// [`Finish::Unknown`], preserving their spelling rather than failing.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum Finish {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    /// A reported reason not classified as stop, length, content filtering,
    /// tool calls or error, whether the table lists it or not; not proof of
    /// completeness.
    Other,
    /// A later normalized wire value this version does not classify.
    Unknown(String),
}

impl Finish {
    pub fn parse(value: &str) -> Self {
        match value {
            "stop" => Self::Stop,
            "length" => Self::Length,
            "content_filter" => Self::ContentFilter,
            "tool_calls" => Self::ToolCalls,
            "error" => Self::Error,
            "other" => Self::Other,
            value => Self::Unknown(value.to_owned()),
        }
    }

    /// The original normalized wire spelling, including an unknown value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ContentFilter => "content_filter",
            Self::ToolCalls => "tool_calls",
            Self::Error => "error",
            Self::Other => "other",
            Self::Unknown(value) => value,
        }
    }
}

impl From<String> for Finish {
    fn from(value: String) -> Self {
        Self::parse(&value)
    }
}

impl From<Finish> for String {
    fn from(value: Finish) -> Self {
        value.as_str().to_owned()
    }
}

/// A source spelling not recognized by the shared mapping. Unlike an unknown
/// provider reason, this is an integration error, usually a routing typo.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnknownSource {
    pub source: String,
}

impl std::fmt::Display for UnknownSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown finish reason source: {}", self.source)
    }
}

impl std::error::Error for UnknownSource {}

/// Normalize one model step's reported reason, without inferring missing data.
///
/// `source` is a routing wire-format spelling, or `ai-sdk` when only a host's
/// AI SDK reason is available. `raw` is absent if no reason was reported.
/// For Responses, pass the response `status`, except that an `incomplete`
/// response with `incomplete_details.reason` is passed as
/// `incomplete:<reason>`, and as bare `incomplete` when that detail is
/// absent (CONTRACT.md §6.6).
/// Unknown sources fail even when `raw` is absent. Unknown raw reasons on a
/// known source become [`Finish::Other`]. The AI SDK's legacy `unknown` means
/// no reported reason and returns `None`.
///
/// Only a mapped `stop` is upgraded by `emitted_tool_calls`; it must not hide
/// truncation, filtering, errors or a missing reason.
pub fn normalize(
    source: &str,
    raw: Option<&str>,
    emitted_tool_calls: bool,
) -> Result<Option<Finish>, UnknownSource> {
    if !matches!(
        source,
        "anthropic.messages"
            | "openai.responses"
            | "openai.chat_completions"
            | "openai.chatgpt_responses"
            | "google.generate_content"
            | "google.code_assist"
            | "aws.bedrock_converse"
            | "ai-sdk"
    ) {
        return Err(UnknownSource {
            source: source.to_owned(),
        });
    }
    let Some(raw) = raw else {
        return Ok(None);
    };
    let finish = match (source, raw) {
        ("anthropic.messages", "end_turn" | "stop_sequence") => Finish::Stop,
        ("anthropic.messages", "max_tokens" | "model_context_window_exceeded") => Finish::Length,
        ("anthropic.messages", "tool_use") => Finish::ToolCalls,
        ("anthropic.messages", "refusal") => Finish::ContentFilter,
        ("anthropic.messages", "pause_turn") => Finish::Other,
        ("openai.chat_completions", "stop") => Finish::Stop,
        ("openai.chat_completions", "length") => Finish::Length,
        ("openai.chat_completions", "content_filter") => Finish::ContentFilter,
        ("openai.chat_completions", "tool_calls" | "function_call") => Finish::ToolCalls,
        ("openai.responses" | "openai.chatgpt_responses", "completed") => Finish::Stop,
        ("openai.responses" | "openai.chatgpt_responses", "incomplete:max_output_tokens") => {
            Finish::Length
        }
        ("openai.responses" | "openai.chatgpt_responses", "incomplete:content_filter") => {
            Finish::ContentFilter
        }
        ("openai.responses" | "openai.chatgpt_responses", "failed") => Finish::Error,
        ("google.generate_content" | "google.code_assist", "STOP") => Finish::Stop,
        ("google.generate_content" | "google.code_assist", "MAX_TOKENS") => Finish::Length,
        (
            "google.generate_content" | "google.code_assist",
            "SAFETY"
            | "RECITATION"
            | "BLOCKLIST"
            | "PROHIBITED_CONTENT"
            | "SPII"
            | "IMAGE_SAFETY"
            | "IMAGE_PROHIBITED_CONTENT"
            | "IMAGE_RECITATION",
        ) => Finish::ContentFilter,
        (
            "google.generate_content" | "google.code_assist",
            "MALFORMED_FUNCTION_CALL"
            | "UNEXPECTED_TOOL_CALL"
            | "TOO_MANY_TOOL_CALLS"
            | "MISSING_THOUGHT_SIGNATURE"
            | "MALFORMED_RESPONSE",
        ) => Finish::Error,
        (
            "google.generate_content" | "google.code_assist",
            "FINISH_REASON_UNSPECIFIED"
            | "LANGUAGE"
            | "OTHER"
            | "IMAGE_OTHER"
            | "NO_IMAGE"
            | "ESCALATION"
            | "PUP_LIMITED_DISABLED"
            | "CONTINUATION",
        ) => Finish::Other,
        ("aws.bedrock_converse", "end_turn" | "stop_sequence") => Finish::Stop,
        ("aws.bedrock_converse", "max_tokens" | "model_context_window_exceeded") => Finish::Length,
        ("aws.bedrock_converse", "tool_use") => Finish::ToolCalls,
        ("aws.bedrock_converse", "guardrail_intervened" | "content_filtered") => {
            Finish::ContentFilter
        }
        ("aws.bedrock_converse", "malformed_model_output" | "malformed_tool_use") => Finish::Error,
        ("ai-sdk", "stop") => Finish::Stop,
        ("ai-sdk", "length") => Finish::Length,
        ("ai-sdk", "content-filter") => Finish::ContentFilter,
        ("ai-sdk", "tool-calls") => Finish::ToolCalls,
        ("ai-sdk", "error") => Finish::Error,
        ("ai-sdk", "other" | "interrupted") => Finish::Other,
        ("ai-sdk", "unknown") => return Ok(None),
        _ => Finish::Other,
    };
    Ok(Some(if emitted_tool_calls && finish == Finish::Stop {
        Finish::ToolCalls
    } else {
        finish
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Source {
        source: String,
        cases: Vec<Sample>,
    }

    #[derive(Deserialize)]
    struct Sample {
        raw: Option<String>,
        #[serde(default)]
        emitted_tool_calls: bool,
        finish: Option<String>,
        error: Option<String>,
    }

    #[test]
    fn every_finish_reason_sample_matches() {
        let sources: Vec<Source> = serde_json::from_str(include_str!(
            "../test-vectors/llm-runner-v1/finish-reason.json"
        ))
        .unwrap();
        assert_eq!(sources.len(), 9);
        for source in sources {
            for sample in source.cases {
                let result = normalize(
                    &source.source,
                    sample.raw.as_deref(),
                    sample.emitted_tool_calls,
                );
                if let Some(error) = sample.error {
                    assert_eq!(error, "unknown_source");
                    let error = result.unwrap_err();
                    assert_eq!(error.source, source.source);
                    assert!(error.to_string().contains(&source.source));
                } else {
                    assert_eq!(
                        result.unwrap().as_ref().map(Finish::as_str),
                        sample.finish.as_deref(),
                        "source={}, raw={:?}, emitted_tool_calls={}",
                        source.source,
                        sample.raw,
                        sample.emitted_tool_calls,
                    );
                    // Exercise the flag for every pinned mapping, including
                    // reasons that must not become a tool-call finish.
                    if !sample.emitted_tool_calls {
                        let expected = match sample.finish.as_deref() {
                            Some("stop") => Some("tool_calls"),
                            other => other,
                        };
                        assert_eq!(
                            normalize(&source.source, sample.raw.as_deref(), true)
                                .unwrap()
                                .as_ref()
                                .map(Finish::as_str),
                            expected,
                            "source={}, raw={:?}",
                            source.source,
                            sample.raw,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn finish_decodes_open_and_round_trips() {
        for (wire, expected) in [
            ("stop", Finish::Stop),
            ("length", Finish::Length),
            ("content_filter", Finish::ContentFilter),
            ("tool_calls", Finish::ToolCalls),
            ("error", Finish::Error),
            ("other", Finish::Other),
            ("future_reason", Finish::Unknown("future_reason".into())),
        ] {
            let value = serde_json::json!(wire);
            let decoded: Finish = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(decoded, expected);
            assert_eq!(serde_json::to_value(decoded).unwrap(), value);
        }
    }
}
