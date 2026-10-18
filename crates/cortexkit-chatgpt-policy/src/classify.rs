use serde_json::Value;
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

/// Where an account wall's reset time came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetSource {
    /// The refusal body stated an absolute reset time (`resets_at`).
    Body,
    /// The reset is the observation time plus a relative delay: the body's `resets_in_seconds`,
    /// or the response's Retry-After hint.
    RetryAfter,
}

impl ResetSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::RetryAfter => "retry_after",
        }
    }
}

/// What a ChatGPT/Codex backend refusal means for the account and the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// The account's plan allowance is used up until a reset (HTTP 429 with `error.type`
    /// `usage_limit_reached`). Re-sending before the reset cannot succeed. An unknown reset
    /// time stays unknown.
    AccountLimitExhausted {
        resets_at_ms: Option<u64>,
        resets_at_source: Option<ResetSource>,
    },
    /// The account's plan does not include the requested use (`usage_not_included`). Waiting
    /// does not help.
    UsageNotIncluded,
    /// The backend rejected the credential the client sent; the client needs a fresh token.
    AuthFailure,
    /// A 401 whose message names an OpenAI `sk-` API key. Subscription clients send an OAuth
    /// access token, never an `sk-` key, so the rejected key belongs to the backend's own side.
    /// It is transient, and must never be reported to a credential vault as a dead credential.
    UpstreamKeyRejection,
    /// An ordinary short throttle; retry after a delay.
    RateLimited,
    /// The backend is busy or unavailable; retry after a delay.
    Overloaded,
    /// None of the classes above. Use the client's generic status handling.
    Other,
}

const UPSTREAM_KEY_PREFIX: &str = "Incorrect API key provided: sk-";
const USAGE_LIMIT_REACHED: &str = "usage_limit_reached";
const USAGE_NOT_INCLUDED: &str = "usage_not_included";

/// Classify one non-2xx HTTP answer from the ChatGPT/Codex backend.
///
/// Header names are case-insensitive, and a repeated name keeps its first value. Only the
/// Retry-After hint is read from the headers: `retry-after-ms` is preferred, then `retry-after`
/// as seconds or as an HTTP date measured from `now`. See
/// [`classify_response_with_retry_after`] for the classification rules.
pub fn classify_response<N: AsRef<str>, V: AsRef<str>>(
    status: u16,
    headers: impl IntoIterator<Item = (N, V)>,
    body: &str,
    now: SystemTime,
) -> Classification {
    let retry_after_secs = retry_after(headers, now);
    classify_response_with_retry_after(status, body, now, retry_after_secs)
}

/// Classify one non-2xx HTTP answer using a caller's already-normalized Retry-After delay.
///
/// A caller may parse Retry-After before reading the response body. Passing that value keeps an
/// HTTP-date delay tied to the time the response was observed.
///
/// Rules, in order:
///
/// 1. A 401 whose `error.message` starts with `Incorrect API key provided: sk-` is
///    [`Classification::UpstreamKeyRejection`]. A 401 that names nothing, or names a token, is
///    still a credential failure.
/// 2. A 429 whose `error.type` is `usage_limit_reached` is
///    [`Classification::AccountLimitExhausted`]. The reset comes from `error.resets_at` (epoch
///    seconds), else `now` plus `error.resets_in_seconds`, else `now` plus the Retry-After delay.
///    Only the status and body decide this: every such refusal recorded in production was a 429
///    with this body, and the open-source Codex client keys on the same status and field.
/// 3. Otherwise the provider code decides, read from the `error` object (or the body when
///    `error` is not an object) as `code`, else `type`, else `status`. `usage_not_included` is
///    [`Classification::UsageNotIncluded`] at any status. Context-window and billing codes are
///    [`Classification::Other`]. Any other 429 is [`Classification::RateLimited`].
///    `rate_limit_error` and `rate_limit_exceeded` are rate limits, `overloaded_error` is
///    [`Classification::Overloaded`], and `authentication_error` and `invalid_api_key` are
///    [`Classification::AuthFailure`]. The catch-all `api_error` is overloaded on 503 or 529 and
///    other elsewhere. Without a deciding code, 401 is a credential failure and 503 or 529 is an
///    overloaded backend.
pub fn classify_response_with_retry_after(
    status: u16,
    body: &str,
    now: SystemTime,
    retry_after_secs: Option<f64>,
) -> Classification {
    if upstream_key_rejection(status, body) {
        return Classification::UpstreamKeyRejection;
    }
    if let Some(limit) = usage_limit_reached(status, body, now, retry_after_secs) {
        return limit;
    }
    status_and_code(status, provider_code(body).as_deref())
}

/// Classify an in-band stream failure: a `response.failed` event, whose error object sits at
/// `response.error`, or an `error` event, whose error object is its `error` field or the event
/// itself.
///
/// Only the error object's `code` decides. `usage_limit_reached` is an account wall whose reset
/// is unknown (stream failures carry no reset time), `usage_not_included` is a plan without the
/// requested use, `rate_limit_exceeded` and `slow_down` are rate limits, and
/// `server_is_overloaded` is an overloaded backend. Everything else is
/// [`Classification::Other`]. A stream failure carries no HTTP status, so it is never a
/// credential failure here.
pub fn classify_stream_failure(event: &Value) -> Classification {
    let error = if event.get("type").and_then(Value::as_str) == Some("response.failed") {
        event.pointer("/response/error").unwrap_or(&Value::Null)
    } else {
        event
            .get("error")
            .filter(|error| error.is_object())
            .unwrap_or(event)
    };
    match error.get("code").and_then(Value::as_str) {
        Some(USAGE_LIMIT_REACHED) => Classification::AccountLimitExhausted {
            resets_at_ms: None,
            resets_at_source: None,
        },
        Some(USAGE_NOT_INCLUDED) => Classification::UsageNotIncluded,
        Some("rate_limit_exceeded" | "slow_down") => Classification::RateLimited,
        Some("server_is_overloaded") => Classification::Overloaded,
        _ => Classification::Other,
    }
}

fn upstream_key_rejection(status: u16, body: &str) -> bool {
    if status != 401 {
        return false;
    }
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|body| {
            body.get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(|message| message.starts_with(UPSTREAM_KEY_PREFIX))
        })
        .unwrap_or(false)
}

fn usage_limit_reached(
    status: u16,
    body: &str,
    now: SystemTime,
    retry_after_secs: Option<f64>,
) -> Option<Classification> {
    if status != 429 {
        return None;
    }
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|body| body.get("error").cloned())?;
    if detail.get("type").and_then(Value::as_str) != Some(USAGE_LIMIT_REACHED) {
        return None;
    }
    let now_ms = system_time_millis(now);
    let reset = detail
        .get("resets_at")
        .and_then(Value::as_u64)
        .map(|seconds| (seconds.saturating_mul(1_000), ResetSource::Body))
        .or_else(|| {
            detail
                .get("resets_in_seconds")
                .and_then(Value::as_u64)
                .map(|seconds| {
                    (
                        now_ms.saturating_add(seconds.saturating_mul(1_000)),
                        ResetSource::RetryAfter,
                    )
                })
        })
        .or_else(|| {
            retry_after_secs.map(|seconds| {
                (
                    now_ms.saturating_add((seconds * 1_000.0) as u64),
                    ResetSource::RetryAfter,
                )
            })
        });
    Some(Classification::AccountLimitExhausted {
        resets_at_ms: reset.map(|(time, _)| time),
        resets_at_source: reset.map(|(_, source)| source),
    })
}

fn status_and_code(status: u16, code: Option<&str>) -> Classification {
    match code {
        Some(USAGE_NOT_INCLUDED) => Classification::UsageNotIncluded,
        // A full context window and an exhausted paid balance are permanent refusals of their
        // own kinds, not one of the subscription classes, whatever the status.
        Some(
            "context_length_exceeded"
            | "context_window_exceeded"
            | "billing_exhausted"
            | "insufficient_quota",
        ) => Classification::Other,
        _ if status == 429 => Classification::RateLimited,
        Some("rate_limit_error" | "rate_limit_exceeded") => Classification::RateLimited,
        Some("overloaded_error") => Classification::Overloaded,
        // `api_error` is a catch-all type rather than a diagnosis, so the status decides. A 401
        // carrying it is not a credential verdict.
        Some("api_error") if overloaded_status(status) => Classification::Overloaded,
        Some("api_error") => Classification::Other,
        Some("authentication_error" | "invalid_api_key") => Classification::AuthFailure,
        Some("permission_error" | "invalid_request_error" | "not_found_error") => {
            Classification::Other
        }
        _ if status == 401 => Classification::AuthFailure,
        _ if overloaded_status(status) => Classification::Overloaded,
        _ => Classification::Other,
    }
}

fn overloaded_status(status: u16) -> bool {
    matches!(status, 503 | 529)
}

fn provider_code(body: &str) -> Option<String> {
    let mut body: Value = serde_json::from_str(body).ok()?;
    // An error answer can arrive as a one-element array of error objects.
    if let Value::Array(errors) = &mut body {
        if errors.len() == 1 {
            body = errors.pop()?;
        }
    }
    let error = body.get("error").filter(|v| v.is_object()).unwrap_or(&body);
    error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str))
        .or_else(|| error.get("status").and_then(Value::as_str))
        .or_else(|| body.get("__type").and_then(Value::as_str))
        .map(|code| code.rsplit('#').next().unwrap_or(code))
        .map(str::to_owned)
}

fn retry_after<N: AsRef<str>, V: AsRef<str>>(
    headers: impl IntoIterator<Item = (N, V)>,
    now: SystemTime,
) -> Option<f64> {
    let mut first = BTreeMap::new();
    for (name, value) in headers {
        let name = name.as_ref().to_ascii_lowercase();
        if name == "retry-after-ms" || name == "retry-after" {
            first
                .entry(name)
                .or_insert_with(|| value.as_ref().trim().to_owned());
        }
    }
    if let Some(milliseconds) = first
        .get("retry-after-ms")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|milliseconds| milliseconds.is_finite() && *milliseconds >= 0.0)
    {
        return Some(milliseconds / 1_000.0);
    }
    let value = first.get("retry-after")?;
    if let Some(seconds) = value
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
    {
        return Some(seconds);
    }
    let retry_at = httpdate::parse_http_date(value).ok()?;
    Some(
        retry_at
            .duration_since(now)
            .unwrap_or_default()
            .as_secs_f64(),
    )
}

fn system_time_millis(time: SystemTime) -> u64 {
    u64::try_from(
        time.duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
