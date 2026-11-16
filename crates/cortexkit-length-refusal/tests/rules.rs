//! One test per rule, each on a body with the shape of a refusal a provider sent, its numbers
//! invented; and the near misses each rule must not take.

use cortexkit_length_refusal::{classify, LengthRefusal};

fn counted(request: u64, limit: u64) -> Option<LengthRefusal> {
    Some(LengthRefusal {
        request_tokens: Some(request),
        limit_tokens: Some(limit),
    })
}

fn uncounted() -> Option<LengthRefusal> {
    Some(LengthRefusal::default())
}

const ANTHROPIC: &str = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 208310 tokens > 200000 maximum"},"request_id":"req_011CTestInvented"}"#;

#[test]
fn anthropic_prompt_is_too_long() {
    assert_eq!(
        classify("anthropic.messages", Some(400), ANTHROPIC),
        counted(208_310, 200_000)
    );
    // The same refusal as a stream's error event.
    assert_eq!(
        classify("anthropic.messages", None, ANTHROPIC),
        counted(208_310, 200_000)
    );
}

#[test]
fn anthropic_near_misses_are_not_length_refusals() {
    let bodies = [
        // Another invalid request.
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages.1.content.0: thinking blocks cannot be modified"}}"#,
        // The message with text after it.
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 208310 tokens > 200000 maximum. Try again."}}"#,
        // The message without its counts.
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long"}}"#,
        // The message under another error type.
        r#"{"type":"error","error":{"type":"api_error","message":"prompt is too long: 208310 tokens > 200000 maximum"}}"#,
    ];
    for body in bodies {
        assert_eq!(
            classify("anthropic.messages", Some(400), body),
            None,
            "{body}"
        );
    }
    // A request larger than the provider accepts in bytes is a different refusal.
    assert_eq!(classify("anthropic.messages", Some(413), ANTHROPIC), None);
    // The rule is Anthropic's alone.
    assert_eq!(
        classify("google.generate_content", Some(400), ANTHROPIC),
        None
    );
}

#[test]
fn openai_chat_completions_states_the_window_and_the_messages() {
    let body = r#"{"error":{"message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 131072 tokens. Please reduce the length of the messages.","type":"invalid_request_error","param":"messages","code":"context_length_exceeded"}}"#;
    assert_eq!(
        classify("openai.chat_completions", Some(400), body),
        counted(131_072, 128_000)
    );
}

#[test]
fn openai_chat_completions_with_an_output_cap_counts_the_total_it_refused() {
    let body = r#"{"error":{"message":"This model's maximum context length is 128000 tokens. However, you requested 140000 tokens (131072 in the messages, 8928 in the completion). Please reduce the length of the messages or completion.","type":"invalid_request_error","param":"messages","code":"context_length_exceeded"}}"#;
    assert_eq!(
        classify("openai.chat_completions", Some(400), body),
        counted(140_000, 128_000)
    );
}

#[test]
fn openai_responses_says_so_without_counts() {
    let body = r#"{"error":{"message":"Your input exceeds the context window of this model. Please adjust your input and try again.","type":"invalid_request_error","param":"input","code":"context_length_exceeded"}}"#;
    assert_eq!(classify("openai.responses", Some(400), body), uncounted());
}

#[test]
fn chatgpt_responses_in_a_failed_stream() {
    let failed = r#"{"type":"response.failed","sequence_number":3,"response":{"id":"resp_invented","status":"failed","error":{"code":"context_length_exceeded","message":"Your input exceeds the context window of this model. Please adjust your input and try again."}}}"#;
    assert_eq!(
        classify("openai.chatgpt_responses", None, failed),
        uncounted()
    );
    let error_event = r#"{"type":"error","sequence_number":2,"code":"context_length_exceeded","message":"Your input exceeds the context window of this model. Please adjust your input and try again.","param":null}"#;
    assert_eq!(
        classify("openai.chatgpt_responses", None, error_event),
        uncounted()
    );
}

#[test]
fn openai_context_window_exceeded_is_the_same_refusal() {
    let body = r#"{"error":{"message":"Your input exceeds the context window of this model. Please adjust your input and try again.","type":"invalid_request_error","param":"input","code":"context_window_exceeded"}}"#;
    assert_eq!(classify("openai.responses", Some(400), body), uncounted());
}

#[test]
fn openai_near_misses_are_not_length_refusals() {
    let bodies = [
        // The message without the code.
        r#"{"error":{"message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 131072 tokens. Please reduce the length of the messages.","type":"invalid_request_error","param":"messages","code":null}}"#,
        // Another invalid request.
        r#"{"error":{"message":"Invalid value for 'temperature'.","type":"invalid_request_error","param":"temperature","code":"invalid_value"}}"#,
        // A stream event that isn't an error.
        r#"{"type":"response.completed","response":{"error":{"code":"context_length_exceeded"}}}"#,
    ];
    for body in bodies {
        assert_eq!(
            classify("openai.responses", Some(400), body),
            None,
            "{body}"
        );
    }
    let rate_limited = r#"{"error":{"message":"Rate limit reached.","type":"requests","code":"context_length_exceeded"}}"#;
    assert_eq!(classify("openai.responses", Some(429), rate_limited), None);
}

#[test]
fn google_input_token_count_exceeds_the_maximum() {
    let body = r#"{"error":{"code":400,"message":"The input token count (1200000) exceeds the maximum number of tokens allowed (1048576).","status":"INVALID_ARGUMENT"}}"#;
    assert_eq!(
        classify("google.generate_content", Some(400), body),
        counted(1_200_000, 1_048_576)
    );
}

#[test]
fn google_code_assist_answers_a_stream_with_an_array() {
    let body = r#"[{"error":{"code":400,"message":"The input token count (1200000) exceeds the maximum number of tokens allowed (1048576).","status":"INVALID_ARGUMENT"}}]"#;
    assert_eq!(
        classify("google.code_assist", Some(400), body),
        counted(1_200_000, 1_048_576)
    );
}

#[test]
fn google_near_misses_are_not_length_refusals() {
    let bodies = [
        r#"{"error":{"code":400,"message":"Request contains an invalid argument.","status":"INVALID_ARGUMENT"}}"#,
        r#"{"error":{"code":400,"message":"The input token count (1200000) exceeds the maximum number of tokens allowed (1048576).","status":"FAILED_PRECONDITION"}}"#,
        r#"{"error":{"code":400,"message":"The input token count exceeds the maximum number of tokens allowed.","status":"INVALID_ARGUMENT"}}"#,
        r#"[]"#,
    ];
    for body in bodies {
        assert_eq!(
            classify("google.generate_content", Some(400), body),
            None,
            "{body}"
        );
    }
}

#[test]
fn bedrock_input_is_too_long() {
    let body = r#"{"message":"Input is too long for requested model."}"#;
    assert_eq!(
        classify("aws.bedrock_converse", Some(400), body),
        uncounted()
    );
    let typed = r#"{"__type":"com.amazon.coral.validate#ValidationException","message":"Input is too long for requested model."}"#;
    assert_eq!(
        classify("aws.bedrock_converse", Some(400), typed),
        uncounted()
    );
    let streamed =
        r#"{"validationException":{"message":"Input is too long for requested model."}}"#;
    assert_eq!(
        classify("aws.bedrock_converse", None, streamed),
        uncounted()
    );
}

#[test]
fn bedrock_near_misses_are_not_length_refusals() {
    let bodies = [
        r#"{"message":"Malformed input request, please reformat your input and try again."}"#,
        r#"{"__type":"com.amazon.coral.service#ThrottlingException","message":"Input is too long for requested model."}"#,
        r#"{"message":"Input is too long for requested model. Reduce it."}"#,
    ];
    for body in bodies {
        assert_eq!(
            classify("aws.bedrock_converse", Some(400), body),
            None,
            "{body}"
        );
    }
}

#[test]
fn an_unknown_family_or_an_unreadable_body_is_not_a_length_refusal() {
    assert_eq!(classify("anthropic", Some(400), ANTHROPIC), None);
    assert_eq!(
        classify("anthropic.messages", Some(400), "prompt is too long"),
        None
    );
}
