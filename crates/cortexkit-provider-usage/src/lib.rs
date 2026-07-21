//! Shared wire types for the `ai-provider-quota` module's `usage.get` payload.
//!
//! The quota module serves an array of [`ProviderUsage`] per request; ALF's
//! router (`codexbar-window-extractors.ts`), astrocyte's capacity axis, and the
//! `ck quota` renderer all consume that shape. This crate is the single
//! definition those consumers compile against, so the wire shape cannot drift
//! without a shared-crate PR every side reviews.
//!
//! # Shape, not policy
//!
//! These are pure data types. Read-time transform semantics are PRODUCER
//! behavior documented on the relevant fields but NOT enforced here:
//! - **Banked-reset relaxation:** the quota module may zero
//!   [`RateWindow::used_percent`] (the EFFECTIVE number consumers pace on) and
//!   carry the provider-reported truth in [`RateWindow::raw_used_percent`].
//!   A consumer renders whatever the wire says; a sudden `0 → high` transition
//!   is an honest disarm (credits spent / auth broke), not a glitch.
//! - **Cache-only partial arrays:** the quota module never blocks on a fetch,
//!   so a result may omit providers not yet swept. Missing ≠ zero.
//! - **Degraded entries ride in-band:** a provider fetch failure is a normal
//!   [`ProviderUsage`] carrying `error`, not a request-level failure.
//!
//! # Serialization contract consumers depend on
//! - camelCase keys (`usedPercent`, `resetsAt`, `windowMinutes`,
//!   `extraRateWindows`, `rawUsedPercent`, `accountInfo`, `savedResets`,
//!   `usedCount`, `totalCount`).
//! - A healthy entry MUST NOT carry `error` (consumers skip truthy-`error`
//!   entries), so it is omitted when absent.
//! - A window is emitted when it has a `usedPercent`; `resetsAt` is OPTIONAL and
//!   omitted when the provider reports no reset (never fabricated).

use serde::{Deserialize, Serialize};

/// One rate-limit window: how much of a quota pool is spent and when it resets.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RateWindow {
    /// 0..100 percent of the window's quota consumed. This is the EFFECTIVE
    /// number consumers pace on: when banked-reset relaxation applies it is
    /// zeroed, and the provider-reported percent moves to `raw_used_percent`.
    pub used_percent: f64,
    /// The provider-reported percent when `used_percent` has been relaxed to
    /// an effective value (banked resets guarantee the window resets before
    /// the wall). Present only on relaxed windows; human-facing UIs should
    /// display this truth alongside the effective number.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub raw_used_percent: Option<f64>,
    /// ISO 8601 / RFC 3339 timestamp when the window resets. Omitted when the
    /// provider reports no reset (e.g. an idle session window with nothing
    /// pending) — never fabricated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    /// Window length in minutes. Omitted when the provider does not report one;
    /// the consumer then paces on utilization alone rather than a burn rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<i64>,
    /// Absolute consumed count in the window (e.g. tokens, requests). Present
    /// only when the provider reports or derives it; human-facing UIs can show
    /// "10,336 / 40,000" alongside the percentage for richer context.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub used_count: Option<f64>,
    /// Absolute total cap for the window. Present alongside `used_count` when
    /// the provider knows the ceiling; omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_count: Option<f64>,
}

/// A per-model window bundled under one account (e.g. Antigravity's Geminis).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtraWindow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<RateWindow>,
}

/// The window topology for one account: up to three account-wide pools plus an
/// optional list of per-model pools.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<RateWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<RateWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tertiary: Option<RateWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_rate_windows: Option<Vec<ExtraWindow>>,
}

/// Account labels and subscription information supplied by a provider or vault.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub org_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub plan_type: Option<String>,
}

impl AccountInfo {
    pub fn is_empty(&self) -> bool {
        self.email.is_none() && self.org_name.is_none() && self.plan_type.is_none()
    }
}

/// One saved reset credit and its expiry time.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreditExpiry {
    pub expires_at: String,
}

/// Saved reset credits reported by Codex's read-only credits endpoint.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SavedResets {
    #[serde(default)]
    pub available_count: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub soonest_expires_at: Option<String>,
    #[serde(default)]
    pub credits: Vec<CreditExpiry>,
}

fn account_info_is_empty(value: &Option<AccountInfo>) -> bool {
    value.as_ref().map(AccountInfo::is_empty).unwrap_or(true)
}

/// One provider/account's usage entry. The `/usage` response is an array of
/// these. A fetch failure becomes an entry carrying `error` (silent-degrade),
/// never a failure of the whole array.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    /// CodexBar provider name (e.g. "codex"), which consumers map to their own id.
    pub provider: String,
    /// Canonical API provider identifier — the models.dev slug for the same
    /// provider (e.g. "openai" when `provider == "codex"`, "anthropic" for
    /// "claude", "google" for "gemini", "xai" for "grok"). Present when the
    /// producer knows the canonical name; absent for providers with no models.dev
    /// counterpart, where consumers fall back to `provider`. Lets every consumer
    /// key on one canonical name instead of each maintaining its own
    /// CodexBar-name → canonical map.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub api_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Which retrieval path produced this (e.g. "oauth") — observability only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "account_info_is_empty", default)]
    pub account_info: Option<AccountInfo>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub saved_resets: Option<SavedResets>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Present only on a degraded entry. The consumer skips any entry with a
    /// truthy `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProviderUsage {
    /// A healthy entry with resolved windows.
    pub fn healthy(provider: &str, account: Option<String>, source: &str, usage: Usage) -> Self {
        Self {
            provider: provider.to_string(),
            api_provider: None,
            account,
            source: Some(source.to_string()),
            account_info: None,
            fetched_at: None,
            saved_resets: None,
            usage: Some(usage),
            error: None,
        }
    }

    /// A degraded entry: the provider is named so the consumer can correlate,
    /// but it carries only an error string and no windows.
    pub fn degraded(provider: &str, error: impl std::fmt::Display) -> Self {
        Self {
            provider: provider.to_string(),
            api_provider: None,
            account: None,
            source: None,
            account_info: None,
            fetched_at: None,
            saved_resets: None,
            usage: None,
            error: Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_info_is_omitted_when_empty_and_keeps_partial_labels() {
        let bare = ProviderUsage::healthy(
            "codex",
            None,
            "oauth",
            Usage {
                primary: Some(RateWindow {
                    used_percent: 10.0,
                    raw_used_percent: None,
                    resets_at: None,
                    window_minutes: Some(300),
                    used_count: None,
                    total_count: None,
                }),
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&bare).unwrap();
        assert!(
            !json.contains("accountInfo"),
            "empty accountInfo must be omitted"
        );

        let mut labeled = bare.clone();
        labeled.account_info = Some(AccountInfo {
            email: Some("a@b.com".to_string()),
            org_name: None,
            plan_type: Some("pro".to_string()),
        });
        let json = serde_json::to_string(&labeled).unwrap();
        assert!(json.contains("\"email\":\"a@b.com\""));
        assert!(json.contains("\"planType\":\"pro\""));
        assert!(!json.contains("orgName"), "absent orgName must be omitted");
    }

    #[test]
    fn saved_resets_use_camel_case_and_round_trip() {
        let entry = ProviderUsage {
            saved_resets: Some(SavedResets {
                available_count: 2,
                soonest_expires_at: Some("2026-07-31T20:11:35Z".to_string()),
                credits: vec![CreditExpiry {
                    expires_at: "2026-07-31T20:11:35Z".to_string(),
                }],
            }),
            ..ProviderUsage::degraded("codex", "x")
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"savedResets\""));
        assert!(json.contains("\"availableCount\":2"));
        assert!(json.contains("\"soonestExpiresAt\""));
        let back: ProviderUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn raw_used_percent_is_absent_from_unrelaxed_windows_and_camel_case_when_present() {
        let unrelaxed = RateWindow {
            used_percent: 41.0,
            raw_used_percent: None,
            resets_at: Some("2026-07-20T00:00:00Z".to_string()),
            window_minutes: Some(10080),
            used_count: None,
            total_count: None,
        };
        let json = serde_json::to_string(&unrelaxed).unwrap();
        assert!(
            !json.contains("rawUsedPercent"),
            "unrelaxed window must not carry the field"
        );

        let relaxed = RateWindow {
            used_percent: 0.0,
            raw_used_percent: Some(70.0),
            resets_at: Some("2026-07-20T00:00:00Z".to_string()),
            window_minutes: Some(10080),
            used_count: None,
            total_count: None,
        };
        let json = serde_json::to_string(&relaxed).unwrap();
        assert!(json.contains("\"rawUsedPercent\":70.0"));
        let back: RateWindow = serde_json::from_str(&json).unwrap();
        assert_eq!(back, relaxed);
    }

    #[test]
    fn healthy_entry_omits_error_and_degraded_entry_omits_usage() {
        let healthy = ProviderUsage::healthy("codex", None, "oauth", Usage::default());
        let json = serde_json::to_string(&healthy).unwrap();
        assert!(!json.contains("error"));

        let degraded = ProviderUsage::degraded("codex", "no session");
        let json = serde_json::to_string(&degraded).unwrap();
        assert!(json.contains("\"error\":\"no session\""));
        assert!(!json.contains("usage"));
    }

    #[test]
    fn api_provider_is_camel_case_present_when_set_and_omitted_when_absent() {
        let mut entry = ProviderUsage::healthy("codex", None, "oauth", Usage::default());
        let json = serde_json::to_string(&entry).unwrap();
        assert!(
            !json.contains("apiProvider"),
            "absent api_provider must be omitted"
        );

        entry.api_provider = Some("openai".to_string());
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"apiProvider\":\"openai\""));
        let back: ProviderUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn used_count_and_total_count_are_camel_case_and_omitted_when_absent() {
        let window = RateWindow {
            used_percent: 25.8,
            raw_used_percent: None,
            resets_at: Some("2026-07-26T14:09:00Z".to_string()),
            window_minutes: Some(10080),
            used_count: None,
            total_count: None,
        };
        let json = serde_json::to_string(&window).unwrap();
        assert!(
            !json.contains("usedCount"),
            "absent used_count must be omitted"
        );
        assert!(
            !json.contains("totalCount"),
            "absent total_count must be omitted"
        );

        let enriched = RateWindow {
            used_count: Some(10336.0),
            total_count: Some(40000.0),
            ..window
        };
        let json = serde_json::to_string(&enriched).unwrap();
        assert!(json.contains("\"usedCount\":10336.0"));
        assert!(json.contains("\"totalCount\":40000.0"));
        let back: RateWindow = serde_json::from_str(&json).unwrap();
        assert_eq!(back, enriched);
    }
}
