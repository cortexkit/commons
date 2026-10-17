//! Shared Anthropic policy, independent of any renderer, HTTP client or async runtime.
//!
//! Clients retain their own user-agent and system identity. Auth metadata selects the
//! header shape; token spelling never does. No request body is rendered by this crate.

mod beta;
mod limits;
#[cfg(feature = "test-vectors")]
pub mod vectors;

pub use beta::{merge_beta_headers, remove_beta_token, InvalidBetaHeader};
pub use limits::{
    anthropic_credit_balance_too_low, classify_account_limit_exhausted,
    classify_account_limit_exhausted_with_retry_after, credit_balance_exhausted,
    AccountLimitExhausted, ResetSource, UNIFIED_REPRESENTATIVE_CLAIM, UNIFIED_RESET,
    UNIFIED_STATUS,
};

use std::collections::BTreeMap;

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const OAUTH_BETA: &str = "oauth-2025-04-20";
/// Interleaved thinking is supported for both OAuth and API-key authentication.
pub const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";
/// Enables the API to report signed thinking blocks whose prefix does not match
/// the expected request history. This beta does not by itself make a mismatch an error;
/// enforcement must be enabled separately.
pub const THINKING_BINDING_BETA: &str = "thinking-binding-controls-2026-08-01";
/// Fast mode needs this beta beside the request body's `speed` field.
pub const FAST_MODE_BETA: &str = "fast-mode-2026-02-01";
pub const RATELIMIT_UNIFIED_HEADER_PREFIX: &str = "anthropic-ratelimit-unified-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    OAuth,
    ApiKey,
}

/// These flags alone control thinking and fast-mode beta additions; choosing OAuth
/// or API-key authentication does not enable those features.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    pub interleaved_thinking: bool,
    pub thinking_binding_controls: bool,
    pub fast_mode: bool,
}

/// Credential placement as data, with no credential retained in the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialHeader {
    pub name: &'static str,
    pub prefix: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderPolicy {
    pub credential: CredentialHeader,
    pub x_app: Option<&'static str>,
    pub beta: Option<String>,
}

/// Headers to add for the selected auth and features. Native proxies merge `beta`
/// with client betas rather than replacing them, and retain the native user-agent.
pub fn header_policy(auth: AuthKind, features: Features) -> HeaderPolicy {
    let mut betas = Vec::new();
    if auth == AuthKind::OAuth {
        betas.push(OAUTH_BETA);
    }
    if features.interleaved_thinking {
        betas.push(INTERLEAVED_THINKING_BETA);
    }
    if features.thinking_binding_controls {
        betas.push(THINKING_BINDING_BETA);
    }
    if features.fast_mode {
        betas.push(FAST_MODE_BETA);
    }
    let beta = merge_beta_headers(betas).expect("static Anthropic betas are valid header values");
    HeaderPolicy {
        credential: credential_header(auth),
        x_app: (auth == AuthKind::OAuth).then_some("cli"),
        beta: (!beta.is_empty()).then_some(beta),
    }
}

pub fn credential_header(auth: AuthKind) -> CredentialHeader {
    match auth {
        AuthKind::OAuth => CredentialHeader {
            name: "authorization",
            prefix: "Bearer ",
        },
        AuthKind::ApiKey => CredentialHeader {
            name: "x-api-key",
            prefix: "",
        },
    }
}

/// Collect header names beginning with `anthropic-ratelimit-unified-`, including
/// future suffixes, and return lowercase names. Keep the first value for each name,
/// decoding invalid UTF-8 lossily. Authentication headers do not match this prefix.
pub fn collect_unified_quota_headers<N, V>(
    headers: impl IntoIterator<Item = (N, V)>,
) -> BTreeMap<String, String>
where
    N: AsRef<str>,
    V: AsRef<[u8]>,
{
    let mut out = BTreeMap::new();
    for (name, value) in headers {
        let name = name.as_ref().to_ascii_lowercase();
        if name.starts_with(RATELIMIT_UNIFIED_HEADER_PREFIX) {
            out.entry(name)
                .or_insert_with(|| String::from_utf8_lossy(value.as_ref()).into_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests;
