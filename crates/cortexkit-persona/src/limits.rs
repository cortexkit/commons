//! Execution budgets for one render, separate from its content and identity.

use std::num::{NonZeroU64, NonZeroUsize};

use crate::{FUEL_LIMIT, RECURSION_LIMIT};

/// Execution budgets for [`LoadedRole::render_with_limits`](crate::LoadedRole::render_with_limits).
///
/// Defaults match [`LoadedRole::render`](crate::LoadedRole::render). Limits
/// decide only whether a render completes: they never change digests,
/// approvals or the bytes of a completed render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderLimits {
    recursion: NonZeroUsize,
    fuel: NonZeroU64,
}

impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            recursion: NonZeroUsize::new(RECURSION_LIMIT).expect("default recursion is nonzero"),
            fuel: NonZeroU64::new(FUEL_LIMIT).expect("default fuel is nonzero"),
        }
    }
}

impl RenderLimits {
    /// Set the recursion budget. See [`RECURSION_LIMIT`] for depth-unit costs.
    #[must_use]
    pub fn with_recursion(mut self, recursion: NonZeroUsize) -> Self {
        self.recursion = recursion;
        self
    }

    /// Set the maximum number of template instructions a render may execute.
    #[must_use]
    pub fn with_fuel(mut self, fuel: NonZeroU64) -> Self {
        self.fuel = fuel;
        self
    }

    /// The recursion budget.
    pub fn recursion(&self) -> NonZeroUsize {
        self.recursion
    }

    /// The maximum number of template instructions a render may execute.
    pub fn fuel(&self) -> NonZeroU64 {
        self.fuel
    }
}
