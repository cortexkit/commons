use base64::Engine;
use serde_json::Value;
use std::fmt;

/// No usable account id was supplied, and the access token carries none.
///
/// The error holds no data, so neither its `Display` nor its `Debug` output can contain any part
/// of the token it was derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingAccountId;

impl fmt::Display for MissingAccountId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "no ChatGPT account id was supplied and the access token carries no \
             chatgpt_account_id claim",
        )
    }
}

impl std::error::Error for MissingAccountId {}

/// Resolve the ChatGPT account id for a request.
///
/// A non-blank `vault_account_id` wins and is returned exactly as given. A credential vault reads
/// the id from the token when it refreshes it, so its value follows a re-login that switches
/// accounts. Otherwise the id comes from the access token's JWT payload, tried in this order:
///
/// 1. the top-level `chatgpt_account_id` claim;
/// 2. `chatgpt_account_id` nested under the `https://api.openai.com/auth` claim;
/// 3. the `id` of the first entry in `organizations`.
///
/// Blank and non-string values are skipped, and returned values are not trimmed. The payload is
/// the second dot-separated segment of the token and must be unpadded base64url JSON. The
/// signature is not verified: a client reads its own token here, it does not validate it.
pub fn account_id(
    vault_account_id: Option<&str>,
    access_token: &str,
) -> Result<String, MissingAccountId> {
    vault_account_id
        .filter(|account_id| !account_id.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| token_account_id(access_token))
        .ok_or(MissingAccountId)
}

// Adapted from OpenCode's codex.ts `extractAccountIdFromClaims`
// (MIT, Copyright (c) 2025 opencode).
fn token_account_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let non_blank = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|account_id| !account_id.trim().is_empty())
            .map(str::to_owned)
    };
    non_blank(claims.get("chatgpt_account_id"))
        .or_else(|| {
            non_blank(
                claims
                    .get("https://api.openai.com/auth")
                    .and_then(|auth| auth.get("chatgpt_account_id")),
            )
        })
        .or_else(|| {
            non_blank(
                claims
                    .get("organizations")
                    .and_then(Value::as_array)
                    .and_then(|organizations| organizations.first())
                    .and_then(|organization| organization.get("id")),
            )
        })
}
