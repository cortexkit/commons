use serde_json::Value;
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

pub const UNIFIED_STATUS: &str = "anthropic-ratelimit-unified-status";
pub const UNIFIED_RESET: &str = "anthropic-ratelimit-unified-reset";
pub const UNIFIED_REPRESENTATIVE_CLAIM: &str = "anthropic-ratelimit-unified-representative-claim";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetSource {
    Header,
    RetryAfter,
}

impl ResetSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::RetryAfter => "retry_after",
        }
    }
}

/// An account wall, not an ordinary short throttle. Unknown reset times stay unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLimitExhausted {
    pub resets_at_ms: Option<u64>,
    pub resets_at_source: Option<ResetSource>,
    pub limit_window: Option<String>,
}

fn first_headers<N: AsRef<str>, V: AsRef<str>>(
    headers: impl IntoIterator<Item = (N, V)>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (name, value) in headers {
        out.entry(name.as_ref().to_ascii_lowercase())
            .or_insert_with(|| value.as_ref().to_owned());
    }
    out
}

/// Classify an Anthropic account window using HTTP 429 plus unified status `rejected`.
/// An exhausted subscription and a short throttle can share the same
/// `rate_limit_error` body; only the headers distinguish them. Body wording is not
/// evidence of account exhaustion. Callers must exclude errors already classified
/// as service-tier or feature-access denials, because waiting for a quota reset cannot
/// grant access to an unavailable tier. Reset preference is unified epoch seconds,
/// then Retry-After (milliseconds, seconds or HTTP date). Header names are
/// case-insensitive, and repeated names keep their first value.
pub fn classify_account_limit_exhausted<N: AsRef<str>, V: AsRef<str>>(
    status: u16,
    headers: impl IntoIterator<Item = (N, V)>,
    body: &str,
    now: SystemTime,
) -> Option<AccountLimitExhausted> {
    let headers = first_headers(headers);
    let retry_after_secs = retry_after(&headers, now);
    classify_account_limit_exhausted_with_retry_after(status, headers, body, now, retry_after_secs)
}

/// Classify a rejected Anthropic 429 using a caller's already-normalized retry hint.
/// A caller may parse Retry-After before reading the response body. Passing the
/// normalized value keeps an HTTP-date delay tied to the original observation time
/// and preserves the caller's choice to prefer `retry-after-ms` when both hints exist.
pub fn classify_account_limit_exhausted_with_retry_after<N: AsRef<str>, V: AsRef<str>>(
    status: u16,
    headers: impl IntoIterator<Item = (N, V)>,
    _body: &str,
    now: SystemTime,
    retry_after_secs: Option<f64>,
) -> Option<AccountLimitExhausted> {
    if status != 429 {
        return None;
    }
    let headers = first_headers(headers);
    let rejected = headers
        .get(UNIFIED_STATUS)
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("rejected"));
    if !rejected {
        return None;
    }
    let now_ms = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let reset = headers
        .get(UNIFIED_RESET)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| (seconds.saturating_mul(1_000), ResetSource::Header))
        .or_else(|| {
            retry_after_secs.map(|seconds| {
                (
                    now_ms.saturating_add((seconds * 1_000.0) as u64),
                    ResetSource::RetryAfter,
                )
            })
        });
    let limit_window = headers
        .get(UNIFIED_REPRESENTATIVE_CLAIM)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some(AccountLimitExhausted {
        resets_at_ms: reset.map(|(time, _)| time),
        resets_at_source: reset.map(|(_, source)| source),
        limit_window,
    })
}

fn retry_after(headers: &BTreeMap<String, String>, now: SystemTime) -> Option<f64> {
    let nonnegative = |value: &str| {
        value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite() && *number >= 0.0)
    };
    if let Some(milliseconds) = headers
        .get("retry-after-ms")
        .and_then(|value| nonnegative(value))
    {
        return Some(milliseconds / 1_000.0);
    }
    let value = headers.get("retry-after")?.trim();
    nonnegative(value).or_else(|| {
        let retry_at = httpdate::parse_http_date(value).ok()?;
        Some(
            retry_at
                .duration_since(now)
                .unwrap_or_default()
                .as_secs_f64(),
        )
    })
}

/// Anthropic's out-of-credit message for an API key whose prepaid balance is used up.
/// The caller pairs this with `invalid_request_error` and the Anthropic family.
///
/// Example HTTP 400 body (illustrative, not a captured provider response):
/// `{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}}`.
/// The message phrase distinguishes billing exhaustion from other
/// `invalid_request_error` cases, such as malformed requests. This is a billing
/// classification, not an authentication failure.
pub fn anthropic_credit_balance_too_low(message: &str) -> bool {
    message
        .to_ascii_lowercase()
        .contains("credit balance is too low")
}

/// Body-only entry point for clients that already know the endpoint is Anthropic.
/// Status handling remains with the caller.
pub fn credit_balance_exhausted(body: &str) -> bool {
    let Ok(body) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    let error = body
        .get("error")
        .filter(|value| value.is_object())
        .unwrap_or(&body);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str));
    let message = body
        .pointer("/error/message")
        .or_else(|| body.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    code == Some("invalid_request_error") && anthropic_credit_balance_too_low(message)
}
