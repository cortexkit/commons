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
    /// the wall).
    ///
    /// **Pace on `used_percent`, not on this.** The effective number is the real
    /// headroom: a reset that is going to happen has already been accounted for.
    /// Treating this as the truer figure routes work away from an account whose
    /// credit is about to be spent — and the credit expires whether or not it is
    /// used, so the cautious-looking reading is the lossy one. Display it beside
    /// the effective number in a human-facing view, where a zero next to real
    /// consumption would otherwise look like a fault.
    ///
    /// Emitted **only where the two diverge**, so its absence means they agree
    /// and falling back to `used_percent` is exact rather than approximate.
    /// Rendering a placeholder for absence would be wrong on every unrelaxed
    /// window, which is nearly all of them.
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
    /// Absolute consumed count in the window (e.g. tokens, requests). A count
    /// of things, so integral by contract, and only ever the upstream's own
    /// figure — never recovered from a percentage and a cap (a derived figure
    /// can carry a disagreement between two provider endpoints while wearing
    /// a type that claims exactness). Omitted when the upstream reports only
    /// a percentage. Human-facing UIs can show "10,336 / 40,000" alongside
    /// the percentage for richer context.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub used_count: Option<f64>,
    /// Absolute total cap for the window, when the upstream states one. May
    /// appear without `used_count`: the cap can be known while the consumed
    /// figure is only a percentage.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_count: Option<f64>,
    /// How the window's quota comes back, when the upstream STATES a mechanic.
    ///
    /// **Absence licenses nothing.** It means the upstream said nothing about
    /// replenishment — never "this is a fixed window". Most providers state
    /// nothing, so absence is the common case and carries no information.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub regeneration: Option<Regeneration>,
}

/// A stated replenishment mechanic for a window.
///
/// Present only where an upstream describes how the quota returns. It exists
/// because the alternative — projecting a replenishment onto `resets_at` — makes
/// a continuously refilling pool indistinguishable from a hard cutoff, and the
/// projection is unrecoverable once published.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Regeneration {
    /// Which mechanic the upstream describes: `cliff`, `drip`, or `unstated`.
    ///
    /// **PACING DEPENDS ON THIS AND THE THREE ANSWERS DIFFER SHARPLY.** A rate
    /// alone cannot separate them: "1,000,000 units per 720h" describes both a
    /// lump arriving on one instant and a steady accrual every hour, and the two
    /// give opposite answers about headroom on day 14.
    ///
    /// - `cliff` — the whole amount lands at `resets_at`, and **accrual before
    ///   that instant is exactly zero**. A consumer treating this as gradual
    ///   believes it has partial headroom when it has none.
    /// - `drip` — the quota accrues continuously at `rate`, so headroom grows
    ///   between reads and an exhausted pool becomes usable again without any
    ///   reset event.
    /// - `unstated` — the upstream states that quota replenishes but nothing
    ///   establishes which mechanic. **No pacing claim exists: display the rate,
    ///   never derive headroom from it.** Same contract as `PoolFunding::Unknown`
    ///   and `PoolBasis::Unstated`, for the same reason — an honest arm keeps a
    ///   producer from guessing to satisfy the type.
    ///
    /// A plain `String` rather than an enum, and REQUIRED rather than optional.
    /// String because this is an observability wire: a variant added later must
    /// not make an old consumer drop the record that reports a state it has never
    /// seen. Required because an optional discriminator invites exactly the
    /// inference this field exists to prevent — with no value present, a consumer
    /// picks one, and the picker has less evidence than the producer.
    ///
    /// Treat an unrecognised value as `unstated`: render it, pace on nothing.
    pub mechanic: String,
    /// The stated replenishment rate, when the upstream gives one.
    ///
    /// Absent means the mechanic is described without a quantity — a real and
    /// common shape ("credits refresh monthly" with no amount). Absence here says
    /// nothing about `mechanic`, which stays authoritative.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rate: Option<RegenerationRate>,
}

/// How much quota returns, and over what period.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegenerationRate {
    /// Amount replenished per period, in the SAME units as `used_count` and
    /// `total_count` on the window. Not integral by contract: unlike a count, a
    /// rate is legitimately fractional (a monthly grant read per hour).
    pub amount: f64,
    /// Length of the replenishment period in minutes.
    ///
    /// Distinct from the window's own `window_minutes`, which they need not
    /// match: an observed payload states a 720h refill period on a balance whose
    /// percentage is measured against a larger total that includes a purchased
    /// pool that never refills.
    pub per_minutes: i64,
}

/// A per-model window bundled under one account (e.g. Antigravity's Geminis).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtraWindow {
    /// Human-facing label. Absent when the producer has no display text for this
    /// window; render `id` instead rather than dropping the entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Stable identifier to match on. Absent when the producer cannot name the
    /// window stably. **Not unique across providers** — one provider's ids are
    /// model names, another's its own scope labels — so key on
    /// `(provider, id)`, never on `id` alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The figures for this window. Absent means the provider **named a limit it
    /// could not read a figure for**, which is not the same as no limit: the
    /// entry is still evidence the limit exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<RateWindow>,
}

/// The window topology for one account: up to three account-wide pools plus an
/// optional list of per-model pools.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// The provider's **shortest** window, not its most constrained one. Absent
    /// when the provider reported no window of that cadence.
    ///
    /// The three slots are positions, not a ranking, and **they can have holes**:
    /// each is filled from its own optional upstream field, so `secondary` may be
    /// absent while `tertiary` is present. Walk all three plus
    /// `extra_rate_windows` rather than stopping at the first gap, and take the
    /// maximum when asking how much headroom an account has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<RateWindow>,
    /// The next cadence up, typically weekly. Absent means not reported — never
    /// that the window exists at zero. See [`Usage::primary`] on slot holes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<RateWindow>,
    /// A third account-wide window where a provider has one. Absent means not
    /// reported. See [`Usage::primary`] on slot holes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tertiary: Option<RateWindow>,
    /// Windows whose meaning has no slot — per-model pools, scoped weeklies.
    /// Absent means the provider published none.
    ///
    /// These are **real limits**, not extras in the dispensable sense: a consumer
    /// ignoring this list silently ignores whichever limits did not fit three
    /// slots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_rate_windows: Option<Vec<ExtraWindow>>,
}

/// An amount of money or credit, in integer minor units.
///
/// Not a float, and the reason is not stylistic. A balance is compared against
/// zero on every routing decision that reads it, and binary floating point
/// cannot hold ordinary decimal amounts exactly — the nearest `f64` to `0.1` is
/// not `0.1`, so sums drift and a comparison near zero can fall either way. The
/// providers agree: DeepSeek and MiniMax both send decimal strings, and
/// Anthropic sends integer minor units with an exponent.
///
/// Parse a provider's own representation once, where its precision is still
/// known, rather than passing a float along and re-rendering it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Amount {
    /// The amount in minor units: `1050` with `exponent: 2` is 10.50.
    pub minor: i64,
    /// Decimal places in `minor`. `2` for currencies with cents; `0` for whole
    /// credits or points.
    pub exponent: u8,
    /// What the amount is denominated in: a currency code like `"USD"`, or a
    /// provider's own label for its credits.
    ///
    /// A free string rather than a currency enum, because not every pool is
    /// money — some are points that convert to no currency, and an enum would
    /// force those into a currency slot or drop them.
    pub unit: String,
}

/// Where a pool's balance came from, which decides what a consumer may promise.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PoolFunding {
    /// Given by the provider: a promotion, a trial grant, a voucher. Spendable
    /// without a bill.
    Granted,
    /// Bought. Spending it costs money.
    Purchased,
    /// Included in a subscription the account already pays for.
    Subscription,
    /// The provider separates this pool but does not say what funds it, **or**
    /// the producer named a funding kind this consumer does not recognise.
    ///
    /// A correct answer rather than a failure one: some providers name their
    /// pools without defining them, and guessing the funding is how a consumer
    /// ends up spending money it meant to protect.
    ///
    /// It is also the deserialization fallback, and the two meanings genuinely
    /// agree — a funding kind added after this consumer was built is, to this
    /// consumer, of unknown funding. Without the fallback an unrecognised value
    /// fails the whole `ProviderUsage` entry rather than this one field, so a
    /// new pool kind would take an account's *usage* down with it and read as
    /// the provider being unavailable.
    #[serde(other)]
    Unknown,
}

/// How a pool's `remaining` was obtained.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PoolBasis {
    /// The provider states this pool's remaining balance directly.
    Reported,
    /// Computed from a total and a consumption figure that covers several pools
    /// at once, so the split between them is not known.
    ///
    /// The distinction is load-bearing for any "spend only granted credits"
    /// policy: against a `Reported` pool it is exact, and against a `Derived`
    /// one it can only be a ceiling.
    Derived,
    /// No basis was stated, or one was stated that this consumer does not
    /// recognise. **Treat `remaining` as a ceiling, never as exact.**
    ///
    /// This is deliberately its own variant rather than folding an unrecognised
    /// value into [`Self::Derived`]. Both are read conservatively, so the
    /// spending behaviour is the same either way — but `Derived` is a statement
    /// about how a number was obtained, and answering "I do not know" with it
    /// would have the producer assert a fact it does not hold. That is the
    /// failure this type exists to prevent, one level up.
    ///
    /// Reading it conservatively is safe in the direction that matters: an
    /// exact remainder treated as a ceiling under-spends, while a ceiling
    /// treated as exact spends money that may not be there.
    #[serde(other)]
    Unstated,
}

/// A prepaid balance or credit pool on an account.
///
/// Plural by necessity: one figure cannot express "9.50 granted and 40
/// purchased", which is exactly the distinction a consumer needs to spend the
/// first without spending the second.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Pool {
    /// The provider's own name for this pool, never one invented here.
    ///
    /// Providers separate pools without always defining them — a wallet may list
    /// voucher, cash and credit balances and document none of them. Passing the
    /// provider's name through lets a consumer decide; renaming one `granted`
    /// would be inventing the label a spend policy keys on.
    pub id: String,
    /// Human-readable name for display.
    pub label: String,
    /// What funds this pool.
    pub funding: PoolFunding,
    /// What is left, when it can be established.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub remaining: Option<Amount>,
    /// The pool's size, when the provider reports one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total: Option<Amount>,
    /// How `remaining` was obtained. Read it before acting on `remaining`.
    pub basis: PoolBasis,
    /// Whether the provider says this pool may currently be drawn on.
    ///
    /// Read from the provider, never inferred from `remaining > 0`: a pool can
    /// be non-empty and closed, which several providers publish directly through
    /// their own enable flags. Absent means the provider does not say.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub spendable: Option<bool>,
}

/// Account labels and subscription information supplied by a provider or vault.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    /// Account email. Absent when the upstream does not identify the account that
    /// way — not a signal about the account itself.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub email: Option<String>,
    /// Organisation label. Absent when the upstream reports none; absent does not
    /// mean a personal account.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub org_name: Option<String>,
    /// The upstream's **own** plan label, not a normalised vocabulary, so it is
    /// not comparable across providers. Display and grouping only. Absent when
    /// the upstream states no plan.
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
    /// When the next credit lapses. Absent means **no credit states an expiry**,
    /// which is not the same as none expiring soon.
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
    /// The account this entry describes, as the credential store identifies it.
    ///
    /// Absent means the producer **could not resolve an identity for this
    /// credential**, not that the provider has one account. Some credentials
    /// carry no account identity at all (a bare API key), and an entry is also
    /// emitted unlabelled while an identity is still being confirmed — so an
    /// unlabelled entry is not evidence that a labelled one does not exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Which retrieval path produced this (e.g. "oauth") — observability only.
    ///
    /// **Per lane, not per account, and it moves.** One account can be reached
    /// through more than one credential path, and which one answers is decided
    /// per fetch by whichever is healthy. So the same account can report one
    /// value on a poll and another on the next with nothing having changed about
    /// the account, the credential, or anything the consumer did. Do not key on
    /// it, branch on it, or treat a change in it as an event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Display labels for the account. Absent when the upstream supplies none of
    /// them; carries no operational meaning.
    #[serde(skip_serializing_if = "account_info_is_empty", default)]
    pub account_info: Option<AccountInfo>,
    /// When this entry's figures were last **successfully** fetched — producer
    /// time, per entry, never a common instant across the array.
    ///
    /// Absent means this credential has never had a successful fetch. It keeps
    /// its old value while a failure is being retried, so it ages honestly rather
    /// than pausing; never restamp it with your own poll time.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fetched_at: Option<String>,
    /// Banked quota-reset credits held by this account.
    ///
    /// Absent means there is no credit inventory to report — which includes the
    /// inventory lookup having **failed** on this fetch, since it is separate
    /// from the usage fetch and may fail without degrading the entry. Absent is
    /// therefore not "zero credits held".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub saved_resets: Option<SavedResets>,
    /// The windows. Absent on a degraded entry, and on an entry whose credential
    /// works but whose account reports no quota at all — read `error` and
    /// `error_class` to tell those apart, rather than inferring from this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Prepaid balances and credit pools on this account, when the provider
    /// reports any.
    ///
    /// Deliberately apart from [`Self::usage`], because a pool and a rate window
    /// are different facts that fail in opposite directions: over-consuming a
    /// window gets you throttled and recovers by waiting, while over-consuming a
    /// balance gets you billed and recovers by paying. Nothing in a routing loop
    /// can undo the second, so a balance is never expressed as a window, never
    /// carries a reset, and never appears as a percentage — a consumer that
    /// found one where it expects headroom would pace into a bill.
    ///
    /// Absent means the producer has nothing to say, which is not the same as an
    /// account having no credit. Empty means it looked and the provider reports
    /// no pools.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub spend: Option<Vec<Pool>>,
    /// Present only on a degraded entry. The consumer skips any entry with a
    /// truthy `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A stable, machine-readable name for *why* a degraded entry failed,
    /// published beside the human-readable `error`.
    ///
    /// `error` is prose with no stability promise, so consumers are told not to
    /// branch on it — which leaves them no way to separate failures that mean
    /// something from failures that are a permanent, correct state. A host that
    /// never configured a provider and a host whose credential broke this
    /// morning both produce a degraded entry, and only the second is worth
    /// anyone's attention.
    ///
    /// Classes currently produced:
    ///
    /// | Value | Meaning |
    /// |---|---|
    /// | `credential_absent` | No credential was found. Permanent and correct on a host that never configured this provider; nothing to fix. |
    /// | `credential_unusable` | A credential was found but cannot be used as it stands (empty, incomplete, or refused by the credential store). Someone configured this and it needs fixing. |
    /// | `credential_rejected` | The upstream rejected the credential (401/403). Usually means logging in again. |
    /// | `no_quota_reported` | The credential works and the account genuinely has no quota to report. Not a failure. |
    /// | `upstream_failed` | The upstream could not be reached or returned an error status. Usually transient. |
    /// | `decode_failed` | The response arrived but was not the expected shape. |
    ///
    /// **This list will grow.** A consumer must render an unrecognised class as
    /// a degraded entry with an unknown reason — never drop the entry, and
    /// never fold it into an existing bucket. It is a `String` rather than an
    /// enum for exactly that reason: on an observability surface, meeting an
    /// unknown value must not turn into a parse failure that makes a provider
    /// disappear at the moment its state changed.
    ///
    /// Absent on healthy entries, and absent from any producer that predates
    /// this field.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_class: Option<String>,
    /// Present when this entry is a last-known-good reading served through an
    /// ongoing failure, absent when it is a fresh success.
    ///
    /// Without it a preserved reading is byte-identical to a fresh one apart
    /// from `fetched_at`, so a consumer cannot separate "this figure is old
    /// because the producer has been unable to reach the provider" from "this
    /// figure is old because nothing polled recently". Those have opposite
    /// remedies — the first is a reason to stop acting on the number, the
    /// second is not — and a consumer with only a timestamp has to guess with a
    /// wall-clock threshold, which denies fresh-enough data to catch stale data.
    ///
    /// A producer serving preserved readings is behaving correctly: a brief
    /// upstream failure should not blank a window. This field discloses that it
    /// is happening rather than reporting a fault.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stale: Option<Stale>,
}

/// Why an entry is being served through a failure, and since when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stale {
    /// When the producer first failed to refresh this entry, RFC3339.
    ///
    /// Distinct from `fetched_at`, which is when the served reading was
    /// obtained. The gap between them is how long the producer has been unable
    /// to look, which is the quantity a staleness policy actually wants: a
    /// reading can be minutes old with the producer perfectly healthy, or
    /// seconds old with the producer failing since just after it was taken.
    pub since: String,
    /// The failure class, using the same vocabulary as `error_class` on a
    /// degraded entry.
    ///
    /// Carried so a consumer can tell a flapping upstream from a credential
    /// that has started refusing, without branching on prose. Optional because
    /// a producer may preserve a reading for a reason it cannot classify.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub class: Option<String>,
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
            spend: None,
            error: None,
            error_class: None,
            stale: None,
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
            spend: None,
            error: Some(error.to_string()),
            error_class: None,
            stale: None,
        }
    }

    /// A degraded entry that also names *why* it failed.
    ///
    /// Prefer this over [`Self::degraded`] wherever the producer knows the
    /// class: without it a consumer can only tell an unconfigured provider from
    /// a broken one by reading prose it has been told not to parse. See
    /// [`ProviderUsage::error_class`] for the classes and for the rule that an
    /// unrecognised one must still render.
    pub fn degraded_with_class(
        provider: &str,
        error: impl std::fmt::Display,
        error_class: impl Into<String>,
    ) -> Self {
        Self {
            error_class: Some(error_class.into()),
            ..Self::degraded(provider, error)
        }
    }
}

/// Shared account-identity type (commons#13, operator-ratified 2026-08-29).
///
/// `provider` was two namespaces wearing one name across the fleet: usage
/// sources, credential configs, sweep-unit names, and catalog slugs all emit
/// plausible provider strings, so a wrong join never fails loudly. This type
/// makes the namespace explicit so no consumer ever writes the alias map that
/// a silent join failure invites.
///
/// Absence is ordinary here, not exceptional: at ratification time 34 of 40
/// live wire entries carried no verified identity. Consumers must treat a
/// missing `account_ref` as the expected shape, never as an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountIdentity {
    /// Source-scoped provider name, spelled in the emitting module's own
    /// namespace (a usage source's "claude", a registry's invented plan
    /// name). MANDATORY. The same string under two sources need not mean the
    /// same thing; this field never claims cross-source meaning.
    pub provider: String,

    /// The models.dev slug for this provider, when a join to the model
    /// catalog EXISTS. Named for its source deliberately: a second catalog
    /// source would get its own field rather than silently changing this
    /// one's meaning.
    ///
    /// `None` means NO JOIN EXISTS — never fall back to [`Self::provider`].
    /// models.dev publishes no alias metadata of any kind (measured:
    /// no `alias`, `renamed_from`, `supersedes`, or `deprecated_by`), so
    /// there is nothing for a fallback to fall back to; "use the other name"
    /// fabricates a join, and a fabricated join produces plausible rows
    /// forever instead of erroring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models_dev_provider_slug: Option<String>,

    /// Verified account identity, when the provider disclosed one.
    ///
    /// PROPAGATION RULE: provenance travels with the value and is NEVER
    /// re-derived downstream. The moment a consumer writes
    /// `if provider == "anthropic" { assume frozen }` they have re-implemented
    /// the producer's discriminant from the outside, keyed on a provider
    /// string — the exact namespace hazard this type exists to prevent,
    /// recreated one level down. The producer that holds the value at its
    /// source knows which branch produced it; nobody else does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ref: Option<AccountRef>,
}

/// An account identity value bundled with how it was obtained.
///
/// Bundled rather than parallel fields, so the bug-causing states — a value
/// with no provenance, or a provenance with no value — are unrepresentable.
/// Absent identity carries no provenance because there is nothing to qualify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRef {
    /// The identity value as the provider disclosed it (an email, an account
    /// id, an org slug — whatever the source speaks).
    pub value: String,
    /// Which staleness story produced [`Self::value`]. See
    /// [`AccountRefProvenance`] for why the two must never be conflated.
    pub provenance: AccountRefProvenance,
}

/// The two ways an account identity reaches the wire, with opposite staleness
/// properties.
///
/// The discriminant says a value *could* be wrong, not that it *is*: the
/// frozen branch goes stale only if a record is re-pointed without a
/// re-login. A consumer holding a [`Self::StoredLogin`] value knows it needs
/// watching; a [`Self::LiveClaim`] value re-corrects on every read.
///
/// This enum is CLOSED on purpose — identity-bearing, not diagnostic. An
/// unknown provenance must refuse to decode rather than default: a wrong
/// guess here poisons joins silently, which is worse than a loud decode
/// error on a version skew.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRefProvenance {
    /// Parsed from the served token at read time — re-derived on every get,
    /// self-correcting.
    LiveClaim,
    /// Stored at login time and returned unchanged — frozen, can go stale
    /// silently if the account is re-pointed without a re-login.
    StoredLogin,
}

#[cfg(test)]
mod tests {
    /// Full AccountIdentity round-trips with every field present, and the
    /// wire spelling of both provenance branches is pinned.
    #[test]
    fn account_identity_full_round_trip_pins_provenance_spellings() {
        let full = AccountIdentity {
            provider: "claude".into(),
            models_dev_provider_slug: Some("anthropic".into()),
            account_ref: Some(AccountRef {
                value: "ufuk@example.com".into(),
                provenance: AccountRefProvenance::StoredLogin,
            }),
        };
        let wire = serde_json::to_string(&full).unwrap();
        assert!(
            wire.contains("\"stored_login\""),
            "frozen branch spelling: {wire}"
        );
        let back: AccountIdentity = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, full);

        let live = serde_json::to_string(&AccountRefProvenance::LiveClaim).unwrap();
        assert_eq!(live, "\"live_claim\"");
    }

    /// A minimal identity (provider only) keeps absent fields ABSENT on the
    /// wire — not null — so pre-identity consumers see byte-identical output.
    #[test]
    fn a_minimal_identity_serializes_provider_only() {
        let min = AccountIdentity {
            provider: "kimi-for-coding".into(),
            models_dev_provider_slug: None,
            account_ref: None,
        };
        let wire = serde_json::to_string(&min).unwrap();
        assert_eq!(wire, r#"{"provider":"kimi-for-coding"}"#);
        let back: AccountIdentity = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, min);
    }

    /// A null slug stays None through a round trip and never inherits the
    /// provider's value: the fallback is fabrication, and this asserts the
    /// bytes, not the intent.
    #[test]
    fn a_none_slug_never_becomes_the_provider_value() {
        let id = AccountIdentity {
            provider: "qwen-cloud".into(),
            models_dev_provider_slug: None,
            account_ref: None,
        };
        let wire = serde_json::to_string(&id).unwrap();
        assert!(
            !wire.contains("models_dev_provider_slug"),
            "absent, not null: {wire}"
        );
        let back: AccountIdentity = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.models_dev_provider_slug, None);
        assert_ne!(
            back.models_dev_provider_slug.as_deref(),
            Some(back.provider.as_str()),
            "a decode path that mirrors provider into the slug has fabricated a join"
        );
    }

    /// Unknown provenance REFUSES to decode. Identity-bearing enum: a
    /// wrong-guess default poisons joins silently, so version skew must fail
    /// loud here, unlike the diagnostic value enums.
    #[test]
    fn an_unknown_provenance_refuses_to_decode() {
        let err = serde_json::from_str::<AccountRef>(r#"{"value":"x","provenance":"vibes"}"#);
        assert!(err.is_err(), "unknown provenance must refuse, got {err:?}");
        // And a bare value with no provenance is unrepresentable:
        let err2 = serde_json::from_str::<AccountRef>(r#"{"value":"x"}"#);
        assert!(
            err2.is_err(),
            "value without provenance must refuse, got {err2:?}"
        );
    }

    /// A window with no stated mechanic serializes exactly as before.
    ///
    /// The additive guarantee: every existing producer omits the field, and a
    /// consumer on the previous version must see byte-identical output.
    #[test]
    fn a_window_without_regeneration_is_unchanged_on_the_wire() {
        let window = RateWindow {
            used_percent: 42.0,
            raw_used_percent: None,
            resets_at: Some("2026-08-17T00:00:00Z".to_string()),
            window_minutes: Some(300),
            used_count: None,
            total_count: None,
            regeneration: None,
        };
        let json = serde_json::to_string(&window).expect("serializes");
        assert!(
            !json.contains("regeneration"),
            "an absent mechanic must not appear on the wire: {json}"
        );
    }

    /// A payload from before this field decodes into the new shape.
    #[test]
    fn a_pre_regeneration_payload_still_decodes() {
        let json = r#"{"usedPercent":42.0,"resetsAt":"2026-08-17T00:00:00Z","windowMinutes":300}"#;
        let window: RateWindow = serde_json::from_str(json).expect("older payloads must decode");
        assert_eq!(window.regeneration, None);
    }

    /// The observed cliff shape round-trips, rate and all.
    ///
    /// SHAPE FROM A LIVE CAPTURE (insula#1, 2026-08-17): a credentialed JetBrains
    /// account stating `tariff: { amount: "1000000", duration: "PT720H" }` beside
    /// a known next-refill instant. First observed rate-bearing payload; the
    /// numbers here are its scrubbed values.
    #[test]
    fn a_stated_cliff_refill_round_trips() {
        let window = RateWindow {
            used_percent: 0.67,
            raw_used_percent: None,
            resets_at: Some("2026-08-15T06:00:00.000Z".to_string()),
            window_minutes: None,
            used_count: Some(8100.0),
            total_count: Some(1_207_000.0),
            regeneration: Some(Regeneration {
                mechanic: "cliff".to_string(),
                rate: Some(RegenerationRate {
                    amount: 1_000_000.0,
                    per_minutes: 43_200,
                }),
            }),
        };
        let json = serde_json::to_string(&window).expect("serializes");
        let back: RateWindow = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, window);
        assert!(
            json.contains("\"perMinutes\":43200"),
            "camelCase on the wire: {json}"
        );
    }

    /// A mechanic with no rate is a valid statement.
    ///
    /// "Credits refresh monthly" with no amount is a real upstream shape, and it
    /// is precisely the case a rate-only field could not have expressed — which
    /// is why this is an object rather than a bare rate.
    #[test]
    fn a_mechanic_without_a_rate_is_valid() {
        let json = r#"{"usedPercent":10.0,"regeneration":{"mechanic":"drip"}}"#;
        let window: RateWindow = serde_json::from_str(json).expect("decodes");
        let regeneration = window.regeneration.expect("the mechanic is stated");
        assert_eq!(regeneration.mechanic, "drip");
        assert_eq!(
            regeneration.rate, None,
            "the mechanic stands without a rate"
        );
    }

    /// A mechanic this version has never seen decodes intact.
    ///
    /// THE REASON `mechanic` IS A STRING. On an observability wire a new variant
    /// must not delete the record that reports it: an enum here would fail the
    /// whole entry, so the state a consumer most needs to see is the one that
    /// would vanish. Consumers treat an unrecognised value as `unstated` --
    /// render it, pace on nothing.
    #[test]
    fn an_unrecognised_mechanic_decodes_rather_than_dropping_the_window() {
        let json = r#"{"usedPercent":10.0,"regeneration":{"mechanic":"stepped_thaw"}}"#;
        let window: RateWindow = serde_json::from_str(json).expect("a future variant must decode");
        assert_eq!(
            window.regeneration.expect("present").mechanic,
            "stepped_thaw"
        );
        assert_eq!(window.used_percent, 10.0, "the rest of the window survives");
    }

    /// An entry without the field serializes exactly as before.
    ///
    /// Every consumer decoding today's shape must keep working, so absence has
    /// to be byte-identical rather than merely tolerated.
    #[test]
    fn a_fresh_entry_carries_no_stale_key() {
        let entry = ProviderUsage::healthy("codex", None, "oauth", Usage::default());
        let json = serde_json::to_string(&entry).expect("entry serializes");

        assert!(
            !json.contains("stale"),
            "a fresh entry must not emit the key at all: {json}"
        );
    }

    /// A preserved reading states when the producer stopped being able to look.
    ///
    /// `since` is deliberately not `fetchedAt`: the reading was taken when it
    /// was taken, and the failure began afterwards. The gap between the two is
    /// how long the producer has been blind, which is the quantity a staleness
    /// policy wants -- an entry can be minutes old with the producer healthy,
    /// or seconds old with it failing since just after the read.
    #[test]
    fn a_preserved_reading_states_when_the_failure_began() {
        let mut entry = ProviderUsage::healthy("codex", None, "oauth", Usage::default());
        entry.fetched_at = Some("2026-08-13T10:00:00Z".to_string());
        entry.stale = Some(Stale {
            since: "2026-08-13T10:02:00Z".to_string(),
            class: Some("upstream_failed".to_string()),
        });

        let json = serde_json::to_string(&entry).expect("entry serializes");
        let back: ProviderUsage = serde_json::from_str(&json).expect("entry round-trips");
        let stale = back.stale.expect("the disclosure survives the round trip");

        assert_eq!(stale.since, "2026-08-13T10:02:00Z");
        assert_eq!(stale.class.as_deref(), Some("upstream_failed"));
        assert_ne!(
            Some(stale.since.as_str()),
            back.fetched_at.as_deref(),
            "the two timestamps answer different questions and must not be conflated"
        );
        assert!(
            json.contains("\"stale\""),
            "the key is camelCase on the wire: {json}"
        );
    }

    /// A producer that cannot classify the failure still discloses the state.
    ///
    /// Optional rather than required so a preserved reading is never suppressed
    /// for want of a label -- disclosing "this is stale, cause unstated" beats
    /// looking fresh.
    #[test]
    fn a_disclosure_without_a_class_still_decodes() {
        let json = r#"{"provider":"codex","stale":{"since":"2026-08-13T10:02:00Z"}}"#;
        let entry: ProviderUsage = serde_json::from_str(json).expect("decodes");
        let stale = entry.stale.expect("present");

        assert_eq!(stale.class, None);
        assert_eq!(stale.since, "2026-08-13T10:02:00Z");
    }

    /// An entry from a producer that predates the field decodes unchanged.
    #[test]
    fn an_entry_without_the_field_decodes() {
        let json = r#"{"provider":"codex","source":"oauth"}"#;
        let entry: ProviderUsage = serde_json::from_str(json).expect("decodes");

        assert_eq!(entry.stale, None);
        assert_eq!(entry.provider, "codex");
    }

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
                    regeneration: None,
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
            regeneration: None,
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
            regeneration: None,
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
            regeneration: None,
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

    /// The field is additive: a producer that does not set it must serialize
    /// exactly as before, or adding it changes every existing entry on the wire.
    #[test]
    fn an_entry_without_a_class_serializes_as_it_did_before_the_field_existed() {
        let entry = ProviderUsage::degraded("codex", "no session: nothing configured");
        let json = serde_json::to_string(&entry).unwrap();

        assert!(!json.contains("errorClass"), "absent class must be omitted");
        // Not vacuous: the entry really is a degraded one carrying its message,
        // so this cannot pass by serializing something empty.
        assert!(json.contains("\"error\":\"no session: nothing configured\""));

        let back: ProviderUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
        assert_eq!(back.error_class, None);
    }

    #[test]
    fn a_class_round_trips_under_its_camel_case_wire_name() {
        let entry = ProviderUsage::degraded_with_class(
            "gemini",
            "credential unusable: gemini creds have no refresh_token",
            "credential_unusable",
        );
        let json = serde_json::to_string(&entry).unwrap();

        assert!(json.contains("\"errorClass\":\"credential_unusable\""));
        let back: ProviderUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    /// The classes are open by design, so a consumer built today must survive a
    /// producer that ships a class it has never heard of. Modelling the field as
    /// a `String` is what buys that: an enum would make this a parse failure,
    /// and on an observability surface a parse failure means the entry vanishes
    /// at the moment its state changed.
    #[test]
    fn an_unknown_class_decodes_rather_than_failing() {
        let json = r#"{"provider":"someprovider","error":"something new","errorClass":"a_class_from_the_future"}"#;

        let entry: ProviderUsage =
            serde_json::from_str(json).expect("an unrecognised class must not fail to decode");

        assert_eq!(
            entry.error_class.as_deref(),
            Some("a_class_from_the_future")
        );
        // The rest of the entry survives intact, so a consumer can still render
        // it as degraded-with-unknown-reason rather than dropping it.
        assert_eq!(entry.provider, "someprovider");
        assert_eq!(entry.error.as_deref(), Some("something new"));
    }

    /// An entry with no pools serializes exactly as it did before pools existed.
    ///
    /// Consumers pin these payloads, so an additive field that appears as `null`
    /// on every existing entry is not additive in practice. The check is on the
    /// rendered text rather than on the field, because that is what a consumer
    /// parses.
    #[test]
    fn an_entry_without_pools_does_not_mention_them() {
        let entry = ProviderUsage::healthy("codex", None, "oauth", Usage::default());
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("spend"), "unexpected spend key: {json}");
    }

    /// Pools survive a round trip, including the two fields a consumer must read
    /// before acting on an amount.
    ///
    /// `basis` and `funding` are what separate "you have 10 granted credits
    /// left" from "you were granted 10 credits and we cannot tell how many
    /// remain". A consumer that loses either one is left with a number it cannot
    /// safely spend against.
    #[test]
    fn pools_round_trip_with_their_basis_and_funding() {
        let pool = Pool {
            id: "granted_balance".to_string(),
            label: "Granted".to_string(),
            funding: PoolFunding::Granted,
            remaining: Some(Amount {
                minor: 1050,
                exponent: 2,
                unit: "CNY".to_string(),
            }),
            total: None,
            basis: PoolBasis::Reported,
            spendable: Some(true),
        };
        let mut entry = ProviderUsage::healthy("deepseek", None, "api", Usage::default());
        entry.spend = Some(vec![pool.clone()]);

        let json = serde_json::to_string(&entry).unwrap();
        let back: ProviderUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.spend, Some(vec![pool]));

        // Rendered as the wire spells them, since consumers key on these.
        assert!(json.contains(r#""funding":"granted""#), "{json}");
        assert!(json.contains(r#""basis":"reported""#), "{json}");
        // 10.50 CNY is carried as minor units, never as a float.
        assert!(json.contains(r#""minor":1050"#), "{json}");
        assert!(
            !json.contains("10.5"),
            "an amount was rendered as a decimal: {json}"
        );
    }

    /// An unrecognised funding kind must not take the entry down with it.
    ///
    /// This payload crosses a repository boundary: one project produces it,
    /// others consume it, and their versions move independently. A closed enum
    /// makes the first new funding kind fail deserialization of the WHOLE
    /// `ProviderUsage` entry rather than one field, so an account's rate windows
    /// would vanish because of a credit pool the consumer had never heard of --
    /// and a vanished entry reads as the provider being unavailable.
    ///
    /// Asserted on a mixed entry rather than on the enum alone, because the
    /// blast radius is the point: the usage figure below is what a router acts
    /// on, and it is downstream of the pool that failed.
    #[test]
    fn an_unknown_funding_kind_does_not_discard_the_entry() {
        let json = r#"{
            "provider": "minimax",
            "usage": { "primary": { "usedPercent": 42.0 } },
            "spend": [
              { "id": "a", "label": "A", "funding": "granted",     "basis": "reported" },
              { "id": "b", "label": "B", "funding": "crypto_grant", "basis": "reported" }
            ]
        }"#;

        let entry: ProviderUsage = serde_json::from_str(json).expect("entry must survive");
        let pools = entry.spend.expect("pools present");
        assert_eq!(pools.len(), 2, "no pool may be dropped");
        assert_eq!(pools[0].funding, PoolFunding::Granted);
        // The unrecognised kind lands on Unknown, which is the correct reading:
        // a funding this consumer cannot name is one it must not spend from.
        assert_eq!(pools[1].funding, PoolFunding::Unknown);
        // And the part a router acts on survived.
        assert_eq!(
            entry.usage.and_then(|u| u.primary).map(|w| w.used_percent),
            Some(42.0)
        );
    }

    /// An unrecognised basis reads as unstated, never as exact.
    ///
    /// The two poles are not symmetrical. Treating an exact remainder as a
    /// ceiling under-spends and costs nothing; treating a ceiling as exact
    /// spends money that may not be there. So the fallback folds to the
    /// conservative side, and does so under its own name rather than claiming
    /// the number was derived -- which would assert a fact about a computation
    /// the consumer knows nothing about.
    #[test]
    fn an_unknown_basis_is_unstated_rather_than_exact() {
        let json = r#"{ "id": "a", "label": "A", "funding": "granted",
                        "basis": "sampled_hourly" }"#;
        let pool: Pool = serde_json::from_str(json).expect("pool must survive");
        assert_eq!(pool.basis, PoolBasis::Unstated);
        assert_ne!(
            pool.basis,
            PoolBasis::Reported,
            "an unknown basis must never read as an exact remainder"
        );
    }

    /// A healthy entry must never carry a class: the field's presence is itself
    /// a signal, and a class on a working provider would be a contradiction a
    /// consumer has to resolve.
    #[test]
    fn a_healthy_entry_carries_no_class() {
        let entry = ProviderUsage::healthy("codex", None, "oauth", Usage::default());
        assert_eq!(entry.error_class, None);
        assert!(!serde_json::to_string(&entry)
            .unwrap()
            .contains("errorClass"));
    }
}
