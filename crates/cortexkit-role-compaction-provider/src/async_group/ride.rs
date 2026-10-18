//! `compaction.ride`: the runner's synchronous request for a view
//! (CONTRACT.md §20.5).
//!
//! The runner sends it only when its prefix is being rewritten anyway (the
//! prompt cache expired, the model changed, the system text or tool surface
//! changed), or when a request reached the Setup ceiling. The provider
//! answers with a view or with nothing; a missed deadline counts as nothing.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{ceiling::MAX_CEILING_WAIT_MS, notice::is_session_ref, view::CompactionView};

/// Why the runner asks. Decoded open: a provider that meets a cause it does
/// not know answers `nothing`.
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
    /// `ceiling`: the request reached the Setup ceiling (CONTRACT.md §20.5).
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
    /// sent the request. Above 0 and at most [`MAX_CEILING_WAIT_MS`].
    pub deadline_ms: u64,
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
        }
    }

    pub fn check(&self) -> Result<(), RideProblem> {
        if !is_session_ref(&self.session_ref) {
            return Err(RideProblem::SessionRef);
        }
        if self.deadline_ms == 0 || self.deadline_ms > MAX_CEILING_WAIT_MS {
            return Err(RideProblem::DeadlineOutOfRange);
        }
        Ok(())
    }
}

/// The `compaction.ride` answer. Strict on `answer`: a runner treats an
/// answer it cannot decode as `nothing`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RideAnswer {
    /// A view to apply at this step. The runner checks it exactly as a view
    /// fetched with `view.get`, against its own `digest`.
    View { view: CompactionView },
    /// No view now. The runner proceeds with its current view.
    Nothing,
}

impl RideAnswer {
    pub fn view(view: CompactionView) -> Self {
        Self::View { view }
    }

    pub fn nothing() -> Self {
        Self::Nothing
    }
}
