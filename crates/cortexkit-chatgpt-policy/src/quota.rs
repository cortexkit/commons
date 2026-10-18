use std::collections::BTreeMap;

/// One of the `x-codex-*` rate-limit response headers the Codex CLI parses.
///
/// The names come from the open-source Codex client, `openai/codex` at commit
/// `ce0323f2f2d3b25e82806cd555237da3db4e1897`:
///
/// - `codex-rs/codex-api/src/rate_limits.rs` builds the window names as
///   `x-codex-{primary,secondary}-{used-percent,window-minutes,reset-at}` from the default
///   `codex` limit id (`parse_rate_limit_for_limit` and `parse_rate_limit_window`), and reads
///   `x-codex-credits-has-credits`, `x-codex-credits-unlimited` and `x-codex-credits-balance`
///   (`parse_credits_snapshot`) and `x-codex-rate-limit-reached-type`
///   (`parse_rate_limit_reached_type`);
/// - `codex-rs/codex-api/src/api_bridge.rs` reads `x-codex-active-limit` (`ACTIVE_LIMIT_HEADER`)
///   when it maps a 429 `usage_limit_reached` refusal.
///
/// The free-text `x-codex-promo-message` header is deliberately excluded: it is marketing prose
/// for display, not quota data. Also excluded are `x-codex-limit-name` and the per-limit
/// families Codex discovers by name (such as `x-codex-<model>-primary-used-percent`), which are
/// not part of this allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QuotaHeader {
    PrimaryUsedPercent,
    PrimaryWindowMinutes,
    PrimaryResetAt,
    SecondaryUsedPercent,
    SecondaryWindowMinutes,
    SecondaryResetAt,
    ActiveLimit,
    CreditsHasCredits,
    CreditsUnlimited,
    CreditsBalance,
    RateLimitReachedType,
}

impl QuotaHeader {
    /// Every allowlisted header, in declaration order.
    pub const ALL: [Self; 11] = [
        Self::PrimaryUsedPercent,
        Self::PrimaryWindowMinutes,
        Self::PrimaryResetAt,
        Self::SecondaryUsedPercent,
        Self::SecondaryWindowMinutes,
        Self::SecondaryResetAt,
        Self::ActiveLimit,
        Self::CreditsHasCredits,
        Self::CreditsUnlimited,
        Self::CreditsBalance,
        Self::RateLimitReachedType,
    ];

    /// The header's lowercase wire name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::PrimaryUsedPercent => "x-codex-primary-used-percent",
            Self::PrimaryWindowMinutes => "x-codex-primary-window-minutes",
            Self::PrimaryResetAt => "x-codex-primary-reset-at",
            Self::SecondaryUsedPercent => "x-codex-secondary-used-percent",
            Self::SecondaryWindowMinutes => "x-codex-secondary-window-minutes",
            Self::SecondaryResetAt => "x-codex-secondary-reset-at",
            Self::ActiveLimit => "x-codex-active-limit",
            Self::CreditsHasCredits => "x-codex-credits-has-credits",
            Self::CreditsUnlimited => "x-codex-credits-unlimited",
            Self::CreditsBalance => "x-codex-credits-balance",
            Self::RateLimitReachedType => "x-codex-rate-limit-reached-type",
        }
    }

    /// The allowlisted header with this name, compared case-insensitively.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|header| header.name().eq_ignore_ascii_case(name))
    }
}

/// Keep only the allowlisted `x-codex-*` quota headers. Names match exactly and
/// case-insensitively; a repeated name keeps its first value, and invalid UTF-8 is decoded
/// lossily. Values are returned as sent, so a gateway can forward them unchanged.
pub fn quota_headers<N, V>(
    headers: impl IntoIterator<Item = (N, V)>,
) -> BTreeMap<QuotaHeader, String>
where
    N: AsRef<str>,
    V: AsRef<[u8]>,
{
    let mut out = BTreeMap::new();
    for (name, value) in headers {
        if let Some(header) = QuotaHeader::from_name(name.as_ref()) {
            out.entry(header)
                .or_insert_with(|| String::from_utf8_lossy(value.as_ref()).into_owned());
        }
    }
    out
}
