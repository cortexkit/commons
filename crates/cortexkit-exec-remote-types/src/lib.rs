//! Caller-facing types for the `exec-remote/v1` capability.
//!
//! [`RunRequest`] starts a run; [`StreamRecord`] carries both run and attach
//! replies. Prepare, drop, cancel and status replies are ordinary JSON objects.
//! Output is raw bytes encoded as padded base64, not UTF-8 text. Terminal
//! availability fields always serialize, including explicit nulls.
//!
//! Unknown object fields are ignored. Unknown refusal reasons retain their tag;
//! unknown terminal outcomes are graded like outcome-unknown, never as proof
//! that the command did not run. These types do not implement transport framing,
//! execution policy, UUID generation or executor-to-runner transfer control.

#![forbid(unsafe_code)]

pub mod types;

pub use types::*;
