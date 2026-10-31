//! `compaction.ride`: the runner's synchronous request for a view
//! (CONTRACT.md §8).
//!
//! The runner sends it only when its prefix is being rewritten anyway (the
//! prompt cache expired, the model, account or service tier changed, the system text or tool
//! surface changed, or a response chain must restart), or when a request
//! reached the Setup ceiling. The provider
//! answers with a view or with nothing; a missed deadline counts as nothing.
//! For the rewrite causes, nothing means the runner proceeds with its
//! current view and the run goes on: those rides never end a run. Only the
//! `ceiling` cause waits up to the ceiling policy's `wait_ms` and can end in
//! the run being refused.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{notice::is_session_ref, view::CompactionView};

/// Upper bound on the runner's own `deadline_ms` for the rewrite causes
/// `ttl_expired`, `model_change`, `account_change`, `service_tier_change`, `surface_change` and
/// `chain_reset` when
/// Setup's answer set no `ride_wait_ms`: without the provider's word, such a
/// ride stays short, because it sits in front of a request the user is
/// waiting for.
pub const MAX_REWRITE_RIDE_DEADLINE_MS: u64 = 10_000;

/// Upper bound on any ride's `deadline_ms`, and on Setup's
/// `ride_wait_max_ms` and `ride_wait_ms`: five minutes. A rewrite ride waits
/// as long as Setup's `ride_wait_ms` when the answer carried one; the user
/// sees that wait (`llm-runner/v1`'s `compaction_wait`). Bounding a
/// session's rides by its own `ride_wait_ms` is the runner's job.
pub const MAX_RIDE_WAIT_MS: u64 = 300_000;

/// Why the runner asks. A cause this crate does not know still decodes, as
/// [`RideCause::Unknown`], so a newer runner does not break an older
/// provider; the provider answers such a request `nothing`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RideCause {
    /// `ttl_expired`: the prompt cache expired. Only a runner that knows the
    /// cache lifetime it actually sent declares it.
    TtlExpired,
    /// `model_change`: the session's model changed.
    ModelChange,
    /// `account_change`: the runner knows the next request's provider account
    /// differs from the previous request's account before sending it.
    AccountChange,
    /// The next request changes the applied provider level, busting the cache.
    ServiceTierChange,
    /// `surface_change`: the frozen system text or tool surface changed.
    SurfaceChange,
    /// `chain_reset`: before rendering, the runner already knows the next
    /// request will carry full input regardless of the provider's answer.
    /// Send-time retries or refused continuations cannot ride. If the runner
    /// sends `history_edit_cost`, it must be `full_resend` for this cause.
    /// Uses the same bounded wait and answers as `model_change`, but does not
    /// require its model-specific `cache_ttl` refresh. Older providers read
    /// it as unknown.
    ChainReset,
    /// `ceiling`: the request reached the share of the context window that
    /// Setup's ceiling policy names, or overflowed (CONTRACT.md §8).
    Ceiling,
    /// A cause this crate does not know, kept as received.
    Unknown(String),
}

impl RideCause {
    pub fn as_str(&self) -> &str {
        match self {
            Self::TtlExpired => "ttl_expired",
            Self::ModelChange => "model_change",
            Self::AccountChange => "account_change",
            Self::ServiceTierChange => "service_tier_change",
            Self::SurfaceChange => "surface_change",
            Self::ChainReset => "chain_reset",
            Self::Ceiling => "ceiling",
            Self::Unknown(cause) => cause,
        }
    }

    pub fn parse(cause: &str) -> Self {
        match cause {
            "ttl_expired" => Self::TtlExpired,
            "model_change" => Self::ModelChange,
            "account_change" => Self::AccountChange,
            "service_tier_change" => Self::ServiceTierChange,
            "surface_change" => Self::SurfaceChange,
            "chain_reset" => Self::ChainReset,
            "ceiling" => Self::Ceiling,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

impl Serialize for RideCause {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RideCause {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(deserializer)?))
    }
}

/// The `compaction.ride` request. Decoded leniently.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RideRequest {
    pub session_ref: String,
    pub cause: RideCause,
    /// The new provider account's opaque, stable prompt-cache label. Required
    /// for `account_change`, ignored on other causes, and omitted when absent.
    /// Must be 1–64 bytes of `[A-Za-z0-9._-]`, and never be or contain a
    /// credential, token, email address or personal identifier. Equal labels
    /// identify the same provider prompt cache; different labels identify
    /// different caches. Read it with [`RideRequest::account`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Required on `service_tier_change`, including a return to standard;
    /// ignored on other causes. Omitted when absent, like `account`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// How long the runner waits for the answer, in milliseconds from when it
    /// sent the request. Above 0 and at most [`MAX_RIDE_WAIT_MS`]. For a
    /// rewrite cause it is Setup's `ride_wait_ms` when the answer carried
    /// one, and otherwise the runner's own short deadline, at most
    /// [`MAX_REWRITE_RIDE_DEADLINE_MS`]; for `ceiling`, the time left of the
    /// ceiling policy's `wait_ms`, at most
    /// [`crate::ceiling::MAX_CEILING_WAIT_MS`].
    pub deadline_ms: u64,
    /// On a `ceiling` ride only: present when the ride follows a model
    /// provider error saying the request exceeded the model's context window.
    /// Absent means the ceiling came from the runner's own count; a provider
    /// never reads absence as "no overflow happened". On any other cause a
    /// provider ignores it; read it with [`RideRequest::ceiling_overflow`].
    /// Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow: Option<Overflow>,
}

/// The overflow a `ceiling` ride follows: the model provider refused the
/// request as larger than the model's context window. `{}` is valid.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Overflow {
    /// The window limit, in tokens, that the model provider's error stated.
    /// Set only when the runner parsed one from the error, never guessed or
    /// filled from the model catalog. Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_limit_tokens: Option<u64>,
}

impl Overflow {
    /// An overflow whose error stated no limit the runner could parse.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the limit the model provider's error stated.
    pub fn with_reported_limit_tokens(mut self, tokens: u64) -> Self {
        self.reported_limit_tokens = Some(tokens);
        self
    }
}

/// Why a ride request is not acceptable.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RideProblem {
    SessionRef,
    Account,
    ServiceTier,
    DeadlineOutOfRange,
}

impl RideProblem {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SessionRef => "session_ref",
            Self::Account => "account",
            Self::ServiceTier => "service_tier",
            Self::DeadlineOutOfRange => "deadline_out_of_range",
        }
    }
}

impl RideRequest {
    pub fn new(session_ref: impl Into<String>, cause: RideCause, deadline_ms: u64) -> Self {
        Self {
            session_ref: session_ref.into(),
            cause,
            account: None,
            service_tier: None,
            deadline_ms,
            overflow: None,
        }
    }

    /// Set the new account's opaque prompt-cache label for `account_change`.
    pub fn with_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    pub fn with_service_tier(
        mut self,
        tier: cortexkit_role_llm_runner::service_tier::ServiceTier,
    ) -> Self {
        self.service_tier = Some(tier.as_str().into());
        self
    }

    /// The known level, for producer validation. The raw string field retains
    /// an unfamiliar level when reading a newer record.
    pub fn service_tier(&self) -> Option<cortexkit_role_llm_runner::service_tier::ServiceTier> {
        if self.cause != RideCause::ServiceTierChange {
            return None;
        }
        self.service_tier
            .as_deref()
            .and_then(cortexkit_role_llm_runner::service_tier::ServiceTier::parse)
    }

    /// The validated account label, only for `account_change`. A malformed or
    /// absent label, or a different cause, returns `None`.
    pub fn account(&self) -> Option<&str> {
        if self.cause != RideCause::AccountChange {
            return None;
        }
        self.account.as_deref().filter(|value| is_account(value))
    }

    /// Mark the ride as following a context-window overflow. Meaningful only
    /// with `cause: "ceiling"`.
    pub fn with_overflow(mut self, overflow: Overflow) -> Self {
        self.overflow = Some(overflow);
        self
    }

    /// The overflow this ride follows, only when its cause is `ceiling`. On
    /// any other cause the member carries no meaning and this returns `None`.
    pub fn ceiling_overflow(&self) -> Option<&Overflow> {
        match self.cause {
            RideCause::Ceiling => self.overflow.as_ref(),
            _ => None,
        }
    }

    /// Check the session reference, the required label on `account_change`,
    /// and the deadline: above 0 and at most
    /// [`MAX_RIDE_WAIT_MS`], whatever the cause. A rewrite ride may wait as
    /// long as the session's Setup `ride_wait_ms`, which the request does
    /// not carry, so the per-session bound is the runner's to keep.
    /// `overflow` is never a reason to fail: on a cause other than `ceiling`
    /// it is ignored. `account` is ignored on causes other than `account_change`.
    /// Producers must use a recognised `service_tier` on `service_tier_change`.
    /// Readers must not use this producer check to reject a newer record's tier.
    pub fn check(&self) -> Result<(), RideProblem> {
        if !is_session_ref(&self.session_ref) {
            return Err(RideProblem::SessionRef);
        }
        if self.cause == RideCause::AccountChange && self.account().is_none() {
            return Err(RideProblem::Account);
        }
        if self.cause == RideCause::ServiceTierChange && self.service_tier().is_none() {
            return Err(RideProblem::ServiceTier);
        }
        if self.deadline_ms == 0 || self.deadline_ms > MAX_RIDE_WAIT_MS {
            return Err(RideProblem::DeadlineOutOfRange);
        }
        Ok(())
    }
}

/// The shared account-label grammar for rides and notice events.
pub(crate) fn is_account(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The `compaction.ride` answer. Strict on `answer`: a runner treats an
/// answer it cannot decode as `nothing`.
///
/// The enum and both variants are non-exhaustive, so a later optional
/// member is not a breaking change. Build an answer with
/// [`RideAnswer::view`] or [`RideAnswer::nothing`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RideAnswer {
    /// A view to apply at this step. The runner checks it exactly as a view
    /// fetched with `view.get`, against its own `digest`. `cache_ttl` is the
    /// prompt cache's lifetime for the ride's model, on every answer to a
    /// `model_change` ride (CONTRACT.md §8.1); omitted when absent.
    #[non_exhaustive]
    View {
        /// Boxed: a view is far larger than the `nothing` answer.
        view: Box<CompactionView>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_ttl: Option<crate::cache_ttl::CacheTtl>,
    },
    /// No view now. The runner proceeds with its current view. `cache_ttl`
    /// as on `View`: an answer to a `model_change` ride carries it even
    /// when it brings no view.
    #[non_exhaustive]
    Nothing {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_ttl: Option<crate::cache_ttl::CacheTtl>,
    },
}

impl RideAnswer {
    pub fn view(view: CompactionView) -> Self {
        Self::View {
            view: Box::new(view),
            cache_ttl: None,
        }
    }

    pub fn nothing() -> Self {
        Self::Nothing { cache_ttl: None }
    }

    /// The answer with `cache_ttl` set, as every answer to a `model_change`
    /// ride carries it.
    pub fn with_cache_ttl(self, entry: crate::cache_ttl::CacheTtl) -> Self {
        match self {
            Self::View { view, .. } => Self::View {
                view,
                cache_ttl: Some(entry),
            },
            Self::Nothing { .. } => Self::Nothing {
                cache_ttl: Some(entry),
            },
        }
    }

    /// The `cache_ttl` the answer carries, if any.
    pub fn cache_ttl(&self) -> Option<&crate::cache_ttl::CacheTtl> {
        match self {
            Self::View { cache_ttl, .. } | Self::Nothing { cache_ttl } => cache_ttl.as_ref(),
        }
    }
}
