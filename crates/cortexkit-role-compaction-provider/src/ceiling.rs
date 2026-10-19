//! The Setup ceiling (CONTRACT.md §8): when the runner pauses for a view,
//! and when it refuses the run instead of sending.

use serde::{Deserialize, Serialize};

/// The `llm-runner/v1` run-error code a runner writes when a request reached
/// the ceiling and no view brought it back under (CONTRACT.md §8). The
/// runner role defines it, with its `{window, size, cause}` detail, in
/// `cortexkit-role-llm-runner` 0.2.7; this constant quotes the spelling until
/// this crate depends on that release.
pub const CONTEXT_CEILING: &str = "context_ceiling";

/// The `cause` values of [`CONTEXT_CEILING`]'s detail, quoted from
/// `llm-runner/v1` like the code itself.
pub mod context_ceiling_causes {
    /// The ceiling wait ran out with no view that brought the request back
    /// under the ceiling.
    pub const VIEW_TIMEOUT: &str = "view_timeout";
    /// The `compaction.ride` call the runner makes at the ceiling could not
    /// reach the provider, and the current view does not bring the request
    /// back under the ceiling.
    pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";
    /// The request is larger than the model's window even under the best view
    /// the runner holds, for example after a switch to a smaller window.
    pub const OVER_WINDOW: &str = "over_window";
}

/// Upper bound on [`CeilingPolicy::wait_ms`] and on a ride's `deadline_ms`:
/// five minutes, the shortest prompt-cache lifetime a runner may hold the
/// session's cache for.
pub const MAX_CEILING_WAIT_MS: u64 = 300_000;

/// The ceiling policy in Setup's `ready` answer, frozen with the session like
/// `call_when`. Decoded leniently.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CeilingPolicy {
    /// Pause when the runner's estimate of the next request reaches this
    /// share of the model's context window. Above 0 and at most 1.
    pub at_share: f64,
    /// Also pause when the model provider refused the request as too large.
    /// Omitted when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub on_overflow: bool,
    /// How long the runner waits in total for a view that brings the request
    /// back under the ceiling, in milliseconds. Above 0 and at most
    /// [`MAX_CEILING_WAIT_MS`].
    pub wait_ms: u64,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// Why a ceiling policy is not acceptable.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CeilingProblem {
    ShareOutOfRange,
    WaitOutOfRange,
}

impl CeilingProblem {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ShareOutOfRange => "share_out_of_range",
            Self::WaitOutOfRange => "wait_out_of_range",
        }
    }
}

/// How the ceiling wait ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitEnd {
    /// A newer view was applied during the wait.
    ViewApplied,
    /// `wait_ms` ran out with no newer view.
    TimedOut,
    /// The ride call could not reach the provider, and no newer view arrived.
    ProviderUnavailable,
}

/// The runner's decision at the end of a ceiling wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeilingDecision {
    /// Send the request.
    Send,
    /// Refuse the run with `context_ceiling {window, size, cause}`; `cause`
    /// is one of [`context_ceiling_causes`].
    Refuse { cause: &'static str },
}

impl CeilingPolicy {
    pub fn new(at_share: f64, wait_ms: u64) -> Self {
        Self {
            at_share,
            on_overflow: false,
            wait_ms,
        }
    }

    pub fn with_on_overflow(mut self) -> Self {
        self.on_overflow = true;
        self
    }

    pub fn check(&self) -> Result<(), CeilingProblem> {
        if !(self.at_share > 0.0 && self.at_share <= 1.0) {
            return Err(CeilingProblem::ShareOutOfRange);
        }
        if self.wait_ms == 0 || self.wait_ms > MAX_CEILING_WAIT_MS {
            return Err(CeilingProblem::WaitOutOfRange);
        }
        Ok(())
    }

    /// Whether a request estimated at `size` tokens has reached the ceiling
    /// in a window of `window` tokens: `size >= at_share * window`.
    pub fn reached(&self, size: u64, window: u64) -> bool {
        size as f64 >= self.at_share * window as f64
    }

    /// The decision at the end of a ceiling wait, for a request estimated at
    /// `size` tokens under the best view the runner holds. `overflowed` is
    /// true when the pause began because the model provider refused the
    /// request as too large. In order: a request over the window is refused
    /// `over_window`, because a runner never sends over the window; a request
    /// that no longer reaches the ceiling is sent, except that one paused by
    /// an overflow is resent only under a newer view; anything else is
    /// refused `provider_unavailable` when the provider could not be reached,
    /// and `view_timeout` otherwise.
    pub fn decide(
        &self,
        size: u64,
        window: u64,
        overflowed: bool,
        end: WaitEnd,
    ) -> CeilingDecision {
        if size > window {
            return CeilingDecision::Refuse {
                cause: context_ceiling_causes::OVER_WINDOW,
            };
        }
        let resend_ok = !overflowed || end == WaitEnd::ViewApplied;
        if !self.reached(size, window) && resend_ok {
            return CeilingDecision::Send;
        }
        let cause = match end {
            WaitEnd::ProviderUnavailable => context_ceiling_causes::PROVIDER_UNAVAILABLE,
            WaitEnd::ViewApplied | WaitEnd::TimedOut => context_ceiling_causes::VIEW_TIMEOUT,
        };
        CeilingDecision::Refuse { cause }
    }
}
