//! `classify_text`: the words a host kept of a provider's refusal, with no wire family or
//! status beside them. Bodies have the shape of refusals providers sent, numbers invented.

use cortexkit_length_refusal::{classify_text, LengthRefusal};

fn counted(request: u64, limit: u64) -> Option<LengthRefusal> {
    Some(LengthRefusal {
        request_tokens: Some(request),
        limit_tokens: Some(limit),
    })
}

fn uncounted() -> Option<LengthRefusal> {
    Some(LengthRefusal::default())
}

const ANTHROPIC_MESSAGE: &str = "prompt is too long: 213456 tokens > 200000 maximum";
const ANTHROPIC: &str = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 213456 tokens > 200000 maximum"}}"#;
const OPENAI_MESSAGE: &str = "This model's maximum context length is 128000 tokens. However, your messages resulted in 140123 tokens. Please reduce the length of the messages.";
const OPENAI_INNER: &str = r#"{"message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 140123 tokens. Please reduce the length of the messages.","type":"invalid_request_error","param":"messages","code":"context_length_exceeded"}"#;

#[test]
fn an_anthropic_refusal_as_a_body_a_message_or_behind_its_status() {
    for text in [
        ANTHROPIC.to_owned(),
        ANTHROPIC_MESSAGE.to_owned(),
        format!("400 {ANTHROPIC}"),
        format!("400: {ANTHROPIC}"),
        format!("400 {ANTHROPIC_MESSAGE}"),
    ] {
        assert_eq!(classify_text(&text), counted(213_456, 200_000), "{text}");
    }
}

#[test]
fn claude_code_reduced_form_states_no_counts() {
    assert_eq!(classify_text("Prompt is too long"), uncounted());
}

#[test]
fn an_openai_refusal_as_a_body_a_message_or_its_inner_error_behind_the_status() {
    for text in [
        format!(r#"{{"error":{OPENAI_INNER}}}"#),
        OPENAI_MESSAGE.to_owned(),
        format!("400: {OPENAI_INNER}"),
    ] {
        assert_eq!(classify_text(&text), counted(140_123, 128_000), "{text}");
    }
}

#[test]
fn openai_you_requested_counts_the_total_it_refused() {
    let text = "This model's maximum context length is 4097 tokens. However, you requested 4500 tokens (3500 in the messages, 1000 in the completion). Please reduce the length of the messages or completion.";
    assert_eq!(classify_text(text), counted(4_500, 4_097));
}

#[test]
fn openai_length_code_alone_states_no_counts() {
    let text = r#"{"error":{"message":"Input is too long for requested model.","type":"invalid_request_error","code":"context_length_exceeded"}}"#;
    assert_eq!(classify_text(text), uncounted());
}

#[test]
fn a_google_refusal_body() {
    let text = r#"{"error":{"code":400,"message":"The input token count (1250000) exceeds the maximum number of tokens allowed (1048576).","status":"INVALID_ARGUMENT"}}"#;
    assert_eq!(classify_text(text), counted(1_250_000, 1_048_576));
}

#[test]
fn bedrock_too_long_counts_only_beside_its_exception() {
    let typed = r#"{"__type":"com.amazon.coral.validate#ValidationException","message":"Input is too long for requested model."}"#;
    assert_eq!(classify_text(typed), uncounted());
    // Without the exception's name, nothing says which provider sent it.
    assert_eq!(
        classify_text(r#"{"message":"Input is too long for requested model."}"#),
        None
    );
    assert_eq!(
        classify_text("Input is too long for requested model."),
        None
    );
}

#[test]
fn near_misses_are_not_length_refusals() {
    let texts = [
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages: text content blocks must be non-empty"}}"#,
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"max_tokens: 300000 > 128000, which is the maximum allowed number of output tokens for claude-opus-5-5"}}"#,
        r#"{"error":{"message":"max_tokens is too large: 200000. This model supports at most 16384 completion tokens, whereas you provided 200000.","type":"invalid_request_error","param":"max_tokens","code":"invalid_value"}}"#,
        r#"{"error":{"code":400,"message":"Request contains an invalid argument.","status":"INVALID_ARGUMENT"}}"#,
        r#"{"type":"error","error":{"type":"rate_limit_error","message":"Number of request tokens has exceeded your per-minute rate limit"}}"#,
        r#"{"error":{"message":"Rate limit reached for gpt-5 on tokens per min (TPM): Limit 30000, Used 29000, Requested 2000.","type":"tokens","param":null,"code":"rate_limit_exceeded"}}"#,
        r#"{"error":{"code":429,"message":"Resource has been exhausted (e.g. check quota).","status":"RESOURCE_EXHAUSTED"}}"#,
        "the provider failed",
        "",
        "prompt is too long",
    ];
    for text in texts {
        assert_eq!(classify_text(text), None, "{text}");
    }
    // A status other than 400 in front rules the refusal out.
    assert_eq!(classify_text(&format!("413 {ANTHROPIC}")), None);
}
