//! Wire contract for `awaited-work/v1` on the `agent-orchestrator/v1` role.
//!
//! `CONTRACT.md` is normative. Requests are closed; replies ignore unknown
//! members and preserve unknown codes and states. This crate does not supply
//! transport or runtime behavior.

#![forbid(unsafe_code)]

#[macro_use]
mod open_string;
pub mod errors;
mod object;
pub mod params;
pub mod replies;
pub mod request;
#[cfg(feature = "test-vectors")]
pub mod vectors;

pub const ROLE: &str = "agent-orchestrator";
pub const VERSION: &str = "v1";
pub const AWAITED_WORK: &str = "awaited-work/v1";

pub mod ops {
    pub const AWAITED_WORK_OPEN: &str = "awaited_work.open";
    pub const AWAITED_WORK_FINISH: &str = "awaited_work.finish";
    pub const AWAITED_WORK_PROGRESS: &str = "awaited_work.progress";
    pub const AWAITED_WORK_CANCEL: &str = "awaited_work.cancel";
    pub const AWAITED_WORK_SYNC: &str = "awaited_work.sync";
    pub const AWAITED_WORK_LIST: &str = "awaited_work.list";
}

pub const REQUIRED_OPS: &[&str] = &[
    ops::AWAITED_WORK_OPEN,
    ops::AWAITED_WORK_FINISH,
    ops::AWAITED_WORK_PROGRESS,
    ops::AWAITED_WORK_CANCEL,
    ops::AWAITED_WORK_SYNC,
    ops::AWAITED_WORK_LIST,
];
