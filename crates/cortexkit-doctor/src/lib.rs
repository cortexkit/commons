//! Validated, platform-independent wire types for CortexKit module diagnostics.
//!
//! # Document boundary
//!
//! [`CheckOutput::decode`], [`FixOutput::decode`] and [`Manifest::decode`] take
//! `&[u8]` and return the decoded document or a [`DecodeError`]. Its `as_str()`
//! and `Display` expose only the first failing stage code. [`MAX_OUTPUT_BYTES`]
//! includes trailing JSON whitespace. The document types implement `Serialize`;
//! use the explicit decoders, not generic deserialization, for untrusted input.
//!
//! # Constructing and inspecting findings
//!
//! - [`FindingId`] and [`ModuleId`] provide `new(id)`, `FromStr`, `as_str()` and
//!   `Display`. Both validate the grammar and reject secret patterns.
//! - [`Evidence::new`] takes a name and an [`EvidenceValue`]. Inspect it with
//!   `name()`, `kind()` (an [`EvidenceKind`]), and `value()`; `EvidenceValue`
//!   also has `kind()`. A path value includes a [`PathCategory`].
//! - [`Fix::none`] takes `needs_stop`. [`Fix::safe`] and [`Fix::risky`] take
//!   `(&FindingId, needs_stop, Vec<String>, timeout_ms: u64)` and compute a
//!   plan. Getters are `class()` (a [`FixClass`]), `needs_stop()` and `plan()`.
//!   [`Plan::new`] takes `(&FindingId, Vec<String>, needs_stop, FixClass,
//!   timeout_ms)`; its getters are `steps()`, `digest()` and `timeout_ms()`.
//! - [`Finding::new`] takes `(FindingId, Severity, summary, Vec<Evidence>, Fix)`.
//!   Its getters are `id()`, `severity()`, `summary()`, `evidence()` and `fix()`.
//!   [`CheckOutput::new`] takes `Vec<Finding>`, read with `findings()`.
//! - A [`FixOutput`] is constructed with `fixed()`, `refused(code)` or
//!   `failed(code)`, and optionally `with_evidence(Vec<Evidence>)`. Its getters
//!   are `result()` (a [`FixResult`]), `code()` and `evidence()`.
//! - [`is_digest`] checks digest syntax. [`plan_digest`] takes `(&FindingId,
//!   &[String], needs_stop, FixClass, timeout_ms)` and computes the bound digest.
//!
//! # Constructing and inspecting manifests
//!
//! - [`LogSource::new`] takes a glob, with getters `dir()`, `glob()` and `format()`.
//! - [`TemplateField::new`] takes a field name, with getters `name()` and `kind()`.
//!   [`field_kind`] derives the kind or returns `None` for an unlisted name.
//! - [`Template::new`] takes `(target, text, Vec<TemplateField>)` with fields
//!   already sorted by name. The id is computed by [`template_id`]. Getters
//!   are `id()`, `target()`, `text()` and `fields()`. `Template::valid_target`
//!   and `Template::valid_text` check text bounds and control characters.
//! - [`Manifest::new`] takes `(ModuleId, repository, Vec<LogSource>,
//!   Vec<Template>)` with templates already sorted by id. The digest is computed
//!   by [`templates_digest`]. Getters are `module_id()`, `repository()`, `logs()`,
//!   `templates()` and `templates_digest()`. [`Manifest::to_canonical_bytes`]
//!   produces the generator's file bytes with one final LF and checks the limit.
//!
//! [`redact_secrets`] is the shared pattern pass for report strings. It is not
//! a replacement for reviewing free text before publication.
//!
//! [`build_report`] projects validated doctor results and raw log tails into
//! canonical JSON using a per-report [`ReportContext`] and [`ReportInput`].

mod evidence;
mod manifest;
#[cfg(unix)]
pub mod module;
mod output;
mod report;
mod run_error;
#[cfg(unix)]
pub mod runner;
mod secrets;
mod wire;

pub use evidence::{Evidence, EvidenceKind, EvidenceValue, PathCategory};
pub use manifest::{
    field_kind, template_id, templates_digest, LogSource, Manifest, Template, TemplateField,
};
pub use output::{
    is_digest, plan_digest, CheckOutput, Finding, FindingId, Fix, FixClass, FixOutput, FixResult,
    ModuleId, Plan, Severity,
};
pub use report::{build_report, ReportContext, ReportError, ReportInput};
pub use run_error::{Crash, RunError};
pub use secrets::redact_secrets;
pub use wire::{DecodeError, MAX_OUTPUT_BYTES};
