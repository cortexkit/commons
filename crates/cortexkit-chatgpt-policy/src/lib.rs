//! Shared ChatGPT/Codex subscription policy, independent of any HTTP client or async runtime.
//!
//! The ChatGPT backend that serves Codex needs the account id of the subscription a token
//! belongs to, answers a used-up plan allowance with its own refusal shape, and reports quota in
//! `x-codex-*` response headers. This crate holds those rules as pure functions over status
//! codes, header pairs, bodies and JSON values.
//!
//! Clients keep their own identity headers. The Codex CLI already sends its own `originator`,
//! `User-Agent` and session headers, and a gateway in front of it must not replace them, so this
//! crate builds no headers and holds no credentials.

mod account;
mod classify;
mod quota;
#[cfg(feature = "test-vectors")]
pub mod vectors;

pub use account::{account_id, MissingAccountId};
pub use classify::{
    classify_response, classify_response_with_retry_after, classify_stream_failure, Classification,
    ResetSource,
};
pub use quota::{quota_headers, QuotaHeader};

/// The `originator` header value the Codex CLI sends. The ChatGPT backend serves some newer
/// models only to clients that identify as the Codex CLI in both `originator` and `User-Agent`.
pub const CODEX_CLI_ORIGINATOR: &str = "codex_cli_rs";

#[cfg(test)]
mod tests;
