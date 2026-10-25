//! Wire contract of the live usage step feed, contract version 1.
//!
//! The usage feed provider serves two operations. `usage.steps.subscribe`
//! sends a [`Notice`] whenever new steps may exist, and `usage.steps.read`
//! returns them as [`StepRow`]s in a [`ReadReply`]. Apps on every platform
//! decode these shapes; `CONTRACT.md` in this crate is the normative text and
//! `test-vectors/usage-steps-v1/` holds the pinned bytes.
//!
//! Readers are lenient toward a newer provider: an unknown member on a reply,
//! notice or row is ignored, and an unknown [`Verdict`], [`StepSource`] or
//! [`RequestSize`] value decodes as `Other`. Requests are strict: the
//! provider refuses an unknown request member `malformed`, so the request
//! types refuse one too.
//!
//! [`LatestRows`] applies rows in the order reads return them and keeps the
//! last row per identity (`harness` plus `step_ref`).

mod cursor;
mod latest;
mod open_set;
mod ops;
mod row;

pub use cursor::Cursor;
pub use latest::{Applied, LatestRows};
pub use open_set::{RequestSize, StepSource, Verdict};
pub use ops::{Notice, ReadReply, ReadRequest, Refusal, SubscribeRequest};
pub use row::{live_row_identity, BadCounts, NotClosed, StepRow};

/// The version of this contract. Changing a member's meaning or type, removing
/// a member, or changing any threshold below makes a new version.
pub const CONTRACT_VERSION: u32 = 1;

/// Operation that opens the notice stream.
pub const OP_SUBSCRIBE: &str = "usage.steps.subscribe";
/// Operation that returns a page of rows.
pub const OP_READ: &str = "usage.steps.read";

/// `limit` used when a read names none.
pub const DEFAULT_READ_LIMIT: u32 = 200;
/// Largest `limit` a read may name; above it the read is refused
/// `range_too_large`.
pub const MAX_READ_LIMIT: u32 = 1_000;

/// A retention of at least this is `stable`. Part of the contract version.
pub const STABLE_RETENTION_AT_LEAST: f64 = 0.95;
/// A retention of at least this (and below the stable threshold) is
/// `warning`; anything lower is `bust`. Part of the contract version.
pub const WARNING_RETENTION_AT_LEAST: f64 = 0.80;
/// `prompt_dropped` is true when a step's `prompt` is at least this many
/// tokens smaller than the earlier step it is compared with. Part of the
/// contract version.
pub const PROMPT_DROP_TOKENS: u64 = 15_000;

/// Refusal and ending codes. The provider sends them as the `code` of an
/// error body; see [`Refusal`].
pub mod codes {
    /// The caller is not an admitted reader (a local same-user app, or the
    /// operator's phone relayed by the federation module).
    pub const NOT_PERMITTED: &str = "not_permitted";
    /// The request did not decode, or it carries an unknown member.
    pub const MALFORMED: &str = "malformed";
    /// Subscribe: more than 16 subscriptions would be open.
    pub const SUBSCRIPTION_CAPACITY: &str = "subscription_capacity";
    /// Subscribe: the provider is stopping.
    pub const DRAINING: &str = "draining";
    /// Read: `limit` is 0, or `after` is not a cursor the provider issued.
    /// Read and subscribe: the request names `harness` without `session`, or
    /// `session` without `harness`.
    pub const INVALID_REQUEST: &str = "invalid_request";
    /// Read: `limit` is above [`crate::MAX_READ_LIMIT`].
    pub const RANGE_TOO_LARGE: &str = "range_too_large";
    /// Read: `after` is older than the feed holds. The detail carries
    /// `through`, the newest cursor.
    pub const CURSOR_EXPIRED: &str = "cursor_expired";
    /// Subscription ending: the subscriber fell behind its queue bound. The
    /// detail carries `through`, the last notified cursor.
    pub const LAGGED: &str = "lagged";
    /// Subscription ending after the caller's own Cancel. The provider may
    /// send it as the last frame, and the daemon may drop it because the
    /// caller asked to stop. A reader treats it, after its own Cancel, as the
    /// normal end.
    pub const CANCELLED: &str = "cancelled";

    /// Every code that refuses opening a subscription.
    pub const SUBSCRIBE_REFUSALS: &[&str] = &[
        NOT_PERMITTED,
        SUBSCRIPTION_CAPACITY,
        DRAINING,
        MALFORMED,
        INVALID_REQUEST,
    ];
    /// Every code that ends an open subscription with an error.
    pub const SUBSCRIBE_ENDINGS: &[&str] = &[LAGGED, CANCELLED];
    /// Every code that refuses a read.
    pub const READ_REFUSALS: &[&str] = &[
        MALFORMED,
        INVALID_REQUEST,
        RANGE_TOO_LARGE,
        CURSOR_EXPIRED,
        NOT_PERMITTED,
    ];
}
