//! Caller-facing types for the `exec-remote/v1` capability.
//!
//! [`RunRequest`] starts a run; [`StreamRecord`] carries both run and attach
//! replies. Prepare, drop, cancel and status replies are ordinary JSON objects.
//! Output is raw bytes encoded as padded base64, not UTF-8 text. Terminal
//! availability fields always serialize, including explicit nulls.
//!
//! Unknown object fields are ignored. Every enum received by a caller preserves
//! unknown tags in a catch-all with documented grading. Unknown stream records
//! retain their sequence number for replay cursors, but are never terminal.
//! Unknown terminal outcomes are graded like outcome-unknown, never as proof
//! that the command did not run.
//!
//! A missing requested execution platform means Linux. A runner that understands
//! the platform field reports the platform it will actually use in [`Accepted`].
//! If a caller requested a non-Linux platform, it must cancel the job and not
//! trust its result unless the acceptance acknowledges that exact platform. These
//! types do not implement transport framing, execution policy, UUID generation or
//! executor-to-runner transfer control.

#![forbid(unsafe_code)]

pub mod types;

pub use types::*;
