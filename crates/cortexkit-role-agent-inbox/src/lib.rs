//! Wire contract of the `agent-inbox/v1` capability.
//!
//! `CONTRACT.md` is normative. Requests are closed; decoded replies ignore
//! unknown members and retain unknown codes and states. No transport or store
//! is supplied here. The implementer owns durable acceptance and wrapping.

#![forbid(unsafe_code)]

#[macro_use]
mod open_string;
pub mod errors;
pub mod neutralize;
pub mod post;
pub mod read;
#[cfg(any(test, feature = "test-vectors"))]
pub mod vectors;
pub mod withdraw;

pub use neutralize::{neutralize, ENVELOPE_TAG};

pub const ROLE: &str = "agent-inbox";
pub const VERSION: &str = "v1";
pub const PROVIDES: &str = "agent-inbox/v1";

pub mod ops {
    pub const INBOX_POST: &str = "inbox.post";
    pub const INBOX_WITHDRAW: &str = "inbox.withdraw";
    pub const INBOX_LIST: &str = "inbox.list";
    pub const INBOX_READ: &str = "inbox.read";
}

pub const REQUIRED_OPS: &[&str] = &[
    ops::INBOX_POST,
    ops::INBOX_WITHDRAW,
    ops::INBOX_LIST,
    ops::INBOX_READ,
];
