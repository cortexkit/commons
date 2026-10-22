//! `compaction.ride`: the runner's synchronous request for a view
//! (CONTRACT.md §8).
//!
//! The runner sends it only when its prefix is being rewritten anyway (the
//! prompt cache expired, the model changed, the system text or tool surface
//! changed), or when a request reached the Setup ceiling. The provider
//! answers with a view or with nothing; a missed deadline counts as nothing.
//! For the three rewrite causes, nothing means the runner proceeds with its
//! current view and the run goes on: those rides never end a run. Only the
//! `ceiling` cause waits up to the ceiling policy's `wait_ms` and can end in
//! the run being refused.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{notice::is_session_ref, view::CompactionView};

/// Upper bound on the runner's own `deadline_ms` for the rewrite causes
/// `ttl_expired`, `model_change` and `surface_change` when Setup's answer
/// set no `ride_wait_ms`: without the provider's word, such a ride stays
/// short, because it sits in front of a request the user is waiting for.
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
    /// `surface_change`: the frozen system text or tool surface changed.
    SurfaceChange,
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
            Self::SurfaceChange => "surface_change",
            Self::Ceiling => "ceiling",
            Self::Unknown(cause) => cause,
        }
    }

    pub fn parse(cause: &str) -> Self {
        match cause {
            "ttl_expired" => Self::TtlExpired,
            "model_change" => Self::ModelChange,
            "surface_change" => Self::SurfaceChange,
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
    DeadlineOutOfRange,
}

impl RideProblem {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SessionRef => "session_ref",
            Self::DeadlineOutOfRange => "deadline_out_of_range",
        }
    }
}

impl RideRequest {
    pub fn new(session_ref: impl Into<String>, cause: RideCause, deadline_ms: u64) -> Self {
        Self {
            session_ref: session_ref.into(),
            cause,
            deadline_ms,
            overflow: None,
        }
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

    /// Check the session reference and the deadline: above 0 and at most
    /// [`MAX_RIDE_WAIT_MS`], whatever the cause. A rewrite ride may wait as
    /// long as the session's Setup `ride_wait_ms`, which the request does
    /// not carry, so the per-session bound is the runner's to keep.
    /// `overflow` is never a reason to fail: on a cause other than `ceiling`
    /// it is ignored.
    pub fn check(&self) -> Result<(), RideProblem> {
        if !is_session_ref(&self.session_ref) {
            return Err(RideProblem::SessionRef);
        }
        if self.deadline_ms == 0 || self.deadline_ms > MAX_RIDE_WAIT_MS {
            return Err(RideProblem::DeadlineOutOfRange);
        }
        Ok(())
    }
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
