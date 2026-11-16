//! Whether a model provider refused a request because it is too long for the model's context
//! window, and the token counts the refusal states.
//!
//! A runner that gets such a refusal asks its compaction provider to shrink the conversation and
//! sends again; any other refusal ends the run. Both mistakes cost something: a refusal read as a
//! length refusal when it isn't sends a needless compaction and a doomed resend, and a missed one
//! ends a run that a compaction would have saved. So a refusal counts only when the provider
//! says so in a way it reserves for this case, never because the text happens to mention tokens
//! or length:
//!
//! - a dedicated error code (OpenAI's `context_length_exceeded`) is enough on its own, and the
//!   message only supplies the counts when it has a known shape;
//! - a generic error type (Anthropic's `invalid_request_error`, Google's `INVALID_ARGUMENT`,
//!   Bedrock's `ValidationException`) counts only together with a whole message of a known shape,
//!   matched from its first character to its last.
//!
//! The counts are what the message states, in the provider's own tokens. A count the message
//! doesn't state is `None`; nothing is estimated.
//!
//! Two entries read a refusal:
//! - [`classify`] reads the provider's error as it arrived, with the route's wire family and the
//!   response's status;
//! - [`classify_text`] reads only the words a host kept of it (a raw body, the body behind a
//!   status, or the bare message), for a runner that never sees the wire.

use serde_json::{Map, Value};

/// A provider's refusal of a request as too long for the model's context window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LengthRefusal {
    /// The size the provider counted against the window, in its tokens, when the refusal states
    /// it. A provider that counts the request's output cap in states the two together, and this
    /// is that total, since it is what didn't fit.
    pub request_tokens: Option<u64>,
    /// The model's context window, in the provider's tokens, when the refusal states it.
    pub limit_tokens: Option<u64>,
}

/// Whether `body` is a length refusal from a route of wire family `family`, named by its
/// `wire_api` spelling from the routing contract (`anthropic.messages`, `openai.responses`,
/// `openai.chat_completions`, `openai.chatgpt_responses`, `google.generate_content`,
/// `google.code_assist`, `aws.bedrock_converse`). Any other name gives `None`.
///
/// `body` is the provider's error as it arrived: the body of a failed HTTP response, or the
/// error event of a stream that failed after its response started. `http_status` is the
/// response's status, or `None` for an error that arrived inside a stream. Providers refuse an
/// over-long request with 400, so any other status is not a length refusal.
pub fn classify(family: &str, http_status: Option<u16>, body: &str) -> Option<LengthRefusal> {
    if http_status.is_some_and(|status| status != 400) {
        return None;
    }
    let body: Value = serde_json::from_str(body).ok()?;
    match family {
        "anthropic.messages" => anthropic(&body),
        "openai.responses" | "openai.chat_completions" | "openai.chatgpt_responses" => {
            openai(&body)
        }
        "google.generate_content" | "google.code_assist" => google(&body),
        "aws.bedrock_converse" => bedrock(&body, true),
        _ => None,
    }
}

/// Whether `text`, all a host kept of a provider's error, is a length refusal. It may be:
/// - a provider's error body, of any family;
/// - that body, or a provider's inner error object, behind its HTTP status (`400 {...}`,
///   `400: {...}`);
/// - the bare message of a refusal, behind its status or not;
/// - Claude Code's reduced form, `Prompt is too long`, which states no counts.
///
/// Without the family and the status, only what is unambiguous on its own counts. A status in
/// front other than 400 is not a length refusal, and Bedrock's `Input is too long for requested
/// model.` counts only beside its exception's name or OpenAI's length code.
pub fn classify_text(text: &str) -> Option<LengthRefusal> {
    let (status, rest) = split_status(text.trim());
    if status.is_some_and(|status| status != 400) {
        return None;
    }
    match serde_json::from_str::<Value>(rest) {
        Ok(body) => anthropic(&body)
            .or_else(|| openai(&body))
            .or_else(|| google(&body))
            .or_else(|| bedrock(&body, false))
            .or_else(|| inner_error(&body)),
        Err(_) => message(rest),
    }
}

/// A status a host put in front of an error, `400 ` or `400: `, and what follows it.
fn split_status(text: &str) -> (Option<u16>, &str) {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if digits != 3 {
        return (None, text);
    }
    let after = &text[digits..];
    let Some(rest) = after.strip_prefix(": ").or_else(|| after.strip_prefix(' ')) else {
        return (None, text);
    };
    (text[..digits].parse().ok(), rest.trim_start())
}

/// Anthropic sends only the generic `invalid_request_error`, so the whole message decides.
fn anthropic(body: &Value) -> Option<LengthRefusal> {
    anthropic_error(body.get("error")?)
}

fn anthropic_error(error: &Value) -> Option<LengthRefusal> {
    if text(error, "type")? != "invalid_request_error" {
        return None;
    }
    anthropic_message(text(error, "message")?)
}

/// `prompt is too long: 208310 tokens > 200000 maximum`.
fn anthropic_message(message: &str) -> Option<LengthRefusal> {
    let found = counts(
        message,
        &[
            Text("prompt is too long: "),
            Count,
            Text(" tokens > "),
            Count,
            Text(" maximum"),
        ],
    )?;
    let [request, limit] = found[..] else {
        return None;
    };
    Some(LengthRefusal {
        request_tokens: Some(request),
        limit_tokens: Some(limit),
    })
}

/// OpenAI's codes for a request that doesn't fit the model's context window. Either one alone
/// says so; no other refusal carries them.
const OPENAI_LENGTH_CODES: [&str; 2] = ["context_length_exceeded", "context_window_exceeded"];

/// OpenAI's routes say it with a dedicated code. The Chat Completions message states the window
/// and the request's size; the Responses message states neither.
fn openai(body: &Value) -> Option<LengthRefusal> {
    openai_coded(openai_error(body)?)
}

fn openai_coded(error: &Map<String, Value>) -> Option<LengthRefusal> {
    let code = error.get("code").and_then(Value::as_str)?;
    if !OPENAI_LENGTH_CODES.contains(&code) {
        return None;
    }
    let message = error.get("message").and_then(Value::as_str).unwrap_or("");
    Some(openai_message(message).unwrap_or_default())
}

/// The counts of an OpenAI Chat Completions length refusal's message.
fn openai_message(message: &str) -> Option<LengthRefusal> {
    let messages_only = counts(
        message,
        &[
            Text("This model's maximum context length is "),
            Count,
            Text(" tokens. However, your messages resulted in "),
            Count,
            Text(" tokens. Please reduce the length of the messages."),
        ],
    );
    if let Some([limit, request]) = messages_only.as_deref() {
        return Some(LengthRefusal {
            request_tokens: Some(*request),
            limit_tokens: Some(*limit),
        });
    }
    // A request with an output cap is refused with the cap counted in; the total is what
    // didn't fit.
    let with_completion = counts(
        message,
        &[
            Text("This model's maximum context length is "),
            Count,
            Text(" tokens. However, you requested "),
            Count,
            Text(" tokens ("),
            Count,
            Text(" in the messages, "),
            Count,
            Text(" in the completion). Please reduce the length of the messages or completion."),
        ],
    );
    if let Some([limit, requested, _messages, _completion]) = with_completion.as_deref() {
        return Some(LengthRefusal {
            request_tokens: Some(*requested),
            limit_tokens: Some(*limit),
        });
    }
    None
}

/// The error object of an OpenAI refusal: the `error` of an HTTP error body, the
/// `response.error` of a Responses stream's `response.failed` event, or a Responses stream's
/// `error` event itself, which carries its code and message at the top level.
fn openai_error(body: &Value) -> Option<&Map<String, Value>> {
    if let Some(error) = body.get("error").and_then(Value::as_object) {
        return Some(error);
    }
    match text(body, "type")? {
        "response.failed" => body.get("response")?.get("error")?.as_object(),
        "error" => body.as_object(),
        _ => None,
    }
}

/// Google sends the generic `INVALID_ARGUMENT` status, so the whole message decides.
fn google(body: &Value) -> Option<LengthRefusal> {
    // A streamed request is answered with an array of responses, the error in the first.
    let body = body
        .as_array()
        .map_or(Some(body), |responses| responses.first())?;
    let error = body.get("error")?;
    if text(error, "status")? != "INVALID_ARGUMENT" {
        return None;
    }
    google_message(text(error, "message")?)
}

/// `The input token count (1200000) exceeds the maximum number of tokens allowed (1048576).`
fn google_message(message: &str) -> Option<LengthRefusal> {
    let found = counts(
        message,
        &[
            Text("The input token count ("),
            Count,
            Text(") exceeds the maximum number of tokens allowed ("),
            Count,
            Text(")."),
        ],
    )?;
    let [request, limit] = found[..] else {
        return None;
    };
    Some(LengthRefusal {
        request_tokens: Some(request),
        limit_tokens: Some(limit),
    })
}

/// Bedrock's message for a request too long for the model; it states no counts.
const BEDROCK_TOO_LONG: &str = "Input is too long for requested model.";

/// Bedrock sends the generic `ValidationException`, so the whole message decides. Its HTTP
/// error body is `{"message": ...}`, naming the exception in `__type` or only in a response
/// header; a stream names it as the event's one key. `header_named` says whether a body without
/// `__type` may stand for a `ValidationException` named in a header the caller didn't pass,
/// which only a caller that knows the route is Bedrock's can say.
fn bedrock(body: &Value, header_named: bool) -> Option<LengthRefusal> {
    let (exception, message) = match body.get("validationException") {
        Some(event) => (Some("ValidationException"), text(event, "message")?),
        None => {
            let exception = text(body, "__type")
                .map(|kind| kind.rsplit_once('#').map_or(kind, |(_, name)| name))
                .or(header_named.then_some("ValidationException"));
            (exception, text(body, "message")?)
        }
    };
    if exception != Some("ValidationException") {
        return None;
    }
    (message == BEDROCK_TOO_LONG).then(LengthRefusal::default)
}

/// A provider's inner error object on its own, as a host may keep it behind the status: OpenAI's
/// with its length code, or Anthropic's `invalid_request_error` with its message.
fn inner_error(body: &Value) -> Option<LengthRefusal> {
    openai_coded(body.as_object()?).or_else(|| anthropic_error(body))
}

/// A refusal's bare message: Anthropic's, an OpenAI Chat Completions one, Google's, or Claude
/// Code's reduced `Prompt is too long`. Each is specific enough to stand without its error type.
fn message(text: &str) -> Option<LengthRefusal> {
    if text == "Prompt is too long" {
        return Some(LengthRefusal::default());
    }
    anthropic_message(text)
        .or_else(|| openai_message(text))
        .or_else(|| google_message(text))
}

/// One piece of a message's shape: literal text, or a count written as decimal digits.
enum Piece {
    Text(&'static str),
    Count,
}
use Piece::{Count, Text};

/// The counts in `message`, in order, when the whole message has the shape `pieces` describes;
/// `None` when any character of it differs.
fn counts(message: &str, pieces: &[Piece]) -> Option<Vec<u64>> {
    let mut rest = message;
    let mut found = Vec::new();
    for piece in pieces {
        match piece {
            Text(literal) => rest = rest.strip_prefix(literal)?,
            Count => {
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                if digits == 0 {
                    return None;
                }
                found.push(rest[..digits].parse().ok()?);
                rest = &rest[digits..];
            }
        }
    }
    rest.is_empty().then_some(found)
}

/// The string member `key` of `value`.
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}
