use super::*;
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// SYNTHETIC alg:none JWTs whose payloads carry the account id in each claim shape.
const TOP_LEVEL: &str = "eyJhbGciOiJub25lIn0.eyJjaGF0Z3B0X2FjY291bnRfaWQiOiAiYWNjdC10b3AifQ.sig";
const NESTED: &str = "eyJhbGciOiJub25lIn0.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOiB7ImNoYXRncHRfYWNjb3VudF9pZCI6ICJhY2N0LW5lc3RlZCJ9fQ.sig";
const ORG: &str = "eyJhbGciOiJub25lIn0.eyJvcmdhbml6YXRpb25zIjogW3siaWQiOiAib3JnLWZpcnN0In1dfQ.sig";
const NO_CLAIM: &str = "eyJhbGciOiJub25lIn0.eyJlbWFpbCI6ICJ4QHkueiJ9.sig";

/// Captured from a production ChatGPT/Codex usage-limit 429 on 2026-09-16.
const USAGE_LIMIT_BODY: &str = r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","plan_type":"pro","resets_at":1790021043,"eligible_promo":null,"resets_in_seconds":441721}}"#;
/// Captured from the ChatGPT backend on 2026-09-25: its own rejected `sk-` key.
const UPSTREAM_KEY_BODY: &str = r#"{"error":{"message":"Incorrect API key provided: sk-svcac***********fvMA. You can find your API key at https://platform.openai.com/account/api-keys.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#;

fn token(claims: serde_json::Value) -> String {
    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    format!("eyJhbGciOiJub25lIn0.{payload}.sig")
}

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(1_789_579_322_081)
}

fn classify(status: u16, body: &str) -> Classification {
    classify_response(status, std::iter::empty::<(&str, &str)>(), body, now())
}

#[test]
fn a_vault_account_id_wins_over_the_token_claim() {
    assert_eq!(
        account_id(Some("acct-vault"), TOP_LEVEL).as_deref(),
        Ok("acct-vault")
    );
    assert_eq!(
        account_id(Some("acct-vault"), NO_CLAIM).as_deref(),
        Ok("acct-vault")
    );
    assert_eq!(
        account_id(Some(" acct-vault "), TOP_LEVEL).as_deref(),
        Ok(" acct-vault ")
    );
}

#[test]
fn a_blank_vault_account_id_falls_back_to_the_token() {
    for blank in ["", " \t "] {
        assert_eq!(
            account_id(Some(blank), TOP_LEVEL).as_deref(),
            Ok("acct-top")
        );
        assert_eq!(account_id(Some(blank), NO_CLAIM), Err(MissingAccountId));
    }
}

#[test]
fn token_claims_resolve_in_order() {
    assert_eq!(account_id(None, TOP_LEVEL).as_deref(), Ok("acct-top"));
    assert_eq!(account_id(None, NESTED).as_deref(), Ok("acct-nested"));
    assert_eq!(account_id(None, ORG).as_deref(), Ok("org-first"));
    let every_shape = token(json!({
        "chatgpt_account_id": "acct-top",
        "https://api.openai.com/auth": {"chatgpt_account_id": "acct-nested"},
        "organizations": [{"id": "org-first"}],
    }));
    assert_eq!(account_id(None, &every_shape).as_deref(), Ok("acct-top"));
    let blank_top = token(json!({
        "chatgpt_account_id": " ",
        "https://api.openai.com/auth": {"chatgpt_account_id": "acct-nested"},
        "organizations": [{"id": "org-first"}],
    }));
    assert_eq!(account_id(None, &blank_top).as_deref(), Ok("acct-nested"));
    let blank_nested = token(json!({
        "https://api.openai.com/auth": {"chatgpt_account_id": ""},
        "organizations": [{"id": "org-first"}],
    }));
    assert_eq!(account_id(None, &blank_nested).as_deref(), Ok("org-first"));
    let blank_first_org = token(json!({"organizations": [{"id": ""}, {"id": "org-second"}]}));
    assert_eq!(account_id(None, &blank_first_org), Err(MissingAccountId));
}

#[test]
fn malformed_tokens_carry_no_account_id() {
    use base64::Engine;
    for token in ["not-a-jwt", "only.two", "", NO_CLAIM] {
        assert_eq!(account_id(None, token), Err(MissingAccountId), "{token}");
    }
    let padded =
        base64::engine::general_purpose::URL_SAFE.encode(br#"{"chatgpt_account_id":"acct"}"#);
    assert!(padded.ends_with('='));
    assert_eq!(
        account_id(None, &format!("eyJhbGciOiJub25lIn0.{padded}.sig")),
        Err(MissingAccountId)
    );
}

#[test]
fn the_missing_account_error_never_echoes_the_token() {
    let error = account_id(None, NO_CLAIM).unwrap_err();
    for text in [error.to_string(), format!("{error:?}")] {
        assert!(!text.contains(NO_CLAIM));
        assert!(!text.contains("eyJ"));
    }
    assert!(error.to_string().contains("chatgpt_account_id"));
}

#[test]
fn a_401_naming_an_sk_key_is_an_upstream_key_rejection() {
    assert_eq!(
        classify(401, UPSTREAM_KEY_BODY),
        Classification::UpstreamKeyRejection
    );
    // Any other status, or a message that does not start with the sk- key, keeps its class.
    assert_eq!(
        classify(403, UPSTREAM_KEY_BODY),
        Classification::AuthFailure
    );
    let indented = r#"{"error":{"message":" Incorrect API key provided: sk-1"}}"#;
    assert_eq!(classify(401, indented), Classification::AuthFailure);
    let session = r#"{"error":{"message":"Incorrect API key provided: sess-1"}}"#;
    assert_eq!(classify(401, session), Classification::AuthFailure);
}

#[test]
fn a_credential_401_is_an_auth_failure() {
    let expired = r#"{"error":{"message":"Your authentication token has expired.","type":"invalid_request_error","code":"token_expired"}}"#;
    assert_eq!(classify(401, expired), Classification::AuthFailure);
    assert_eq!(classify(401, "not json"), Classification::AuthFailure);
    assert_eq!(
        classify(400, r#"{"error":{"code":"invalid_api_key"}}"#),
        Classification::AuthFailure
    );
    // `api_error` is a catch-all, not a verdict on the credential.
    assert_eq!(
        classify(401, r#"{"error":{"type":"api_error"}}"#),
        Classification::Other
    );
}

#[test]
fn the_usage_limit_429_is_an_account_wall_with_the_body_reset() {
    assert_eq!(
        classify(429, USAGE_LIMIT_BODY),
        Classification::AccountLimitExhausted {
            resets_at_ms: Some(1_790_021_043_000),
            resets_at_source: Some(ResetSource::Body),
        }
    );
    // The same body with any status but 429 is not an exhausted account.
    assert_eq!(classify(403, USAGE_LIMIT_BODY), Classification::Other);
}

#[test]
fn a_usage_limit_without_resets_at_uses_relative_hints() {
    let relative =
        r#"{"error":{"type":"usage_limit_reached","resets_at":"soon","resets_in_seconds":60}}"#;
    assert_eq!(
        classify(429, relative),
        Classification::AccountLimitExhausted {
            resets_at_ms: Some(1_789_579_322_081 + 60_000),
            resets_at_source: Some(ResetSource::RetryAfter),
        }
    );
    let bare = r#"{"error":{"type":"usage_limit_reached"}}"#;
    assert_eq!(
        classify_response_with_retry_after(429, bare, now(), Some(1.5)),
        Classification::AccountLimitExhausted {
            resets_at_ms: Some(1_789_579_322_081 + 1_500),
            resets_at_source: Some(ResetSource::RetryAfter),
        }
    );
    assert_eq!(
        classify(429, bare),
        Classification::AccountLimitExhausted {
            resets_at_ms: None,
            resets_at_source: None,
        }
    );
}

#[test]
fn retry_after_prefers_milliseconds_then_seconds_then_a_date() {
    let bare = r#"{"error":{"type":"usage_limit_reached"}}"#;
    let reset = |headers: &[(&str, &str)]| match classify_response(
        429,
        headers.iter().copied(),
        bare,
        now(),
    ) {
        Classification::AccountLimitExhausted { resets_at_ms, .. } => resets_at_ms,
        other => panic!("expected an account wall, got {other:?}"),
    };
    let base = 1_789_579_322_081;
    assert_eq!(
        reset(&[("Retry-After", "1"), ("retry-after-ms", "300")]),
        Some(base + 300)
    );
    assert_eq!(
        reset(&[("retry-after-ms", "-1"), ("retry-after", " 2 ")]),
        Some(base + 2_000)
    );
    assert_eq!(
        reset(&[("retry-after", "2"), ("retry-after", "9")]),
        Some(base + 2_000)
    );
    assert_eq!(reset(&[("retry-after", "later")]), None);
    // An HTTP date is measured from the observation time, and a date in the past is no delay.
    let whole_second = UNIX_EPOCH + Duration::from_secs(1_789_579_322);
    for (date, delay_ms) in [
        (whole_second + Duration::from_secs(90), 90_000),
        (whole_second - Duration::from_secs(5), 0),
    ] {
        let value = httpdate::fmt_http_date(date);
        assert_eq!(
            classify_response(429, [("retry-after", value.as_str())], bare, whole_second),
            Classification::AccountLimitExhausted {
                resets_at_ms: Some(1_789_579_322_000 + delay_ms),
                resets_at_source: Some(ResetSource::RetryAfter),
            }
        );
    }
}

#[test]
fn usage_not_included_is_its_own_class_at_any_status() {
    for status in [429, 403, 400] {
        assert_eq!(
            classify(status, r#"{"error":{"code":"usage_not_included"}}"#),
            Classification::UsageNotIncluded
        );
        assert_eq!(
            classify(status, r#"{"error":{"type":"usage_not_included"}}"#),
            Classification::UsageNotIncluded
        );
    }
}

#[test]
fn an_ordinary_429_is_a_rate_limit() {
    let throttle = r#"{"error":{"type":"rate_limit_exceeded","message":"Rate limit reached"}}"#;
    assert_eq!(classify(429, throttle), Classification::RateLimited);
    assert_eq!(classify(429, "busy"), Classification::RateLimited);
    assert_eq!(
        classify(429, r#"{"error":{"code":"invalid_api_key"}}"#),
        Classification::RateLimited
    );
    assert_eq!(classify(400, throttle), Classification::RateLimited);
    // An exhausted paid balance is not a throttle.
    assert_eq!(
        classify(429, r#"{"error":{"code":"insufficient_quota"}}"#),
        Classification::Other
    );
}

#[test]
fn a_busy_backend_is_overloaded() {
    assert_eq!(
        classify(503, r#"{"error":{"message":"overloaded"}}"#),
        Classification::Overloaded
    );
    assert_eq!(classify(529, "{}"), Classification::Overloaded);
    assert_eq!(
        classify(500, r#"{"error":{"type":"overloaded_error"}}"#),
        Classification::Overloaded
    );
    assert_eq!(
        classify(503, r#"{"error":{"type":"api_error"}}"#),
        Classification::Overloaded
    );
    assert_eq!(classify(500, "{}"), Classification::Other);
    assert_eq!(
        classify(410, r#"{"error":{"type":"api_error"}}"#),
        Classification::Other
    );
}

#[test]
fn stream_failures_classify_by_code() {
    for (code, expected) in [
        (
            "usage_limit_reached",
            Classification::AccountLimitExhausted {
                resets_at_ms: None,
                resets_at_source: None,
            },
        ),
        ("usage_not_included", Classification::UsageNotIncluded),
        ("rate_limit_exceeded", Classification::RateLimited),
        ("slow_down", Classification::RateLimited),
        ("server_is_overloaded", Classification::Overloaded),
        ("context_length_exceeded", Classification::Other),
        ("invalid_api_key", Classification::Other),
    ] {
        for event in [
            json!({"type":"response.failed","response":{"error":{"code":code}}}),
            json!({"type":"error","error":{"code":code}}),
            json!({"type":"error","code":code}),
        ] {
            assert_eq!(classify_stream_failure(&event), expected, "{event}");
        }
    }
    // A `response.failed` event uses only the error inside `response`, and an error `type`
    // naming a class does not classify it; only `code` does.
    let outer = json!({"type":"response.failed","error":{"code":"usage_not_included"}});
    assert_eq!(classify_stream_failure(&outer), Classification::Other);
    let typed =
        json!({"type":"response.failed","response":{"error":{"type":"usage_not_included"}}});
    assert_eq!(classify_stream_failure(&typed), Classification::Other);
}

#[test]
fn quota_headers_keep_only_the_codex_allowlist() {
    let mut headers: Vec<(String, &str)> = QuotaHeader::ALL
        .iter()
        .map(|header| (header.name().to_ascii_uppercase(), "first"))
        .collect();
    headers.push(("x-codex-primary-used-percent".into(), "second"));
    for excluded in [
        "x-codex-promo-message",
        "x-codex-limit-name",
        "x-codex-primary-over-secondary-limit-percent",
        "x-codex-bengalfox-primary-used-percent",
        "authorization",
        "x-codex-turn-state",
    ] {
        headers.push((excluded.into(), "excluded"));
    }
    let kept = quota_headers(
        headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_bytes())),
    );
    assert_eq!(kept.len(), QuotaHeader::ALL.len());
    for header in QuotaHeader::ALL {
        assert_eq!(
            kept.get(&header).map(String::as_str),
            Some("first"),
            "{}",
            header.name()
        );
        assert_eq!(QuotaHeader::from_name(header.name()), Some(header));
        assert!(header.name().starts_with("x-codex-"));
    }
    assert_eq!(QuotaHeader::from_name("x-codex-promo-message"), None);
}

#[test]
fn quota_header_values_decode_lossily() {
    let kept = quota_headers([("x-codex-credits-balance", &b"12\xff"[..])]);
    assert_eq!(kept[&QuotaHeader::CreditsBalance], "12\u{fffd}");
}

#[test]
fn the_codex_cli_originator_is_its_wire_name() {
    assert_eq!(CODEX_CLI_ORIGINATOR, "codex_cli_rs");
}
