//! Generate a doctor manifest by scanning the Rust source of one Cargo package.

mod cli;
mod event;
mod scan;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use cortexkit_doctor::{template_id, LogSource, Manifest, ModuleId, Template, TemplateField};

/// Module metadata supplied by the caller, not inferred from the Rust source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ManifestMetadata {
    module_id: ModuleId,
    repository: String,
    logs: Vec<LogSource>,
}

impl ManifestMetadata {
    pub fn new(module_id: ModuleId, repository: impl Into<String>, logs: Vec<LogSource>) -> Self {
        Self {
            module_id,
            repository: repository.into(),
            logs,
        }
    }
}

/// The location and reason for a call site that did not contribute a template.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    pub path: PathBuf,
    pub line: usize,
    pub reason: String,
    column: usize,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.path.display(), self.line, self.reason)
    }
}

/// A failure resolving targets, scanning their source, or building a manifest.
#[derive(Debug)]
pub struct GenerateError {
    message: String,
    diagnostics: Vec<Diagnostic>,
}

impl GenerateError {
    /// Skipped sites found before generation failed.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GenerateError {}

/// Scan one package's Cargo-resolved lib and bin targets without building it.
///
/// Uses Cargo on `PATH` (or the executable named in `CARGO`). Diagnostics use
/// crate-relative paths for sources inside the crate. To get the exact file
/// bytes, including the checked size limit, call `Manifest::to_canonical_bytes`.
pub fn generate(
    crate_dir: impl AsRef<Path>,
    metadata: &ManifestMetadata,
) -> Result<(Manifest, Vec<Diagnostic>), GenerateError> {
    generate_with_id(crate_dir.as_ref(), metadata, template_id)
}

fn generate_with_id(
    crate_dir: &Path,
    metadata: &ManifestMetadata,
    id: impl Fn(&str, &str) -> String,
) -> Result<(Manifest, Vec<Diagnostic>), GenerateError> {
    let mut scan = scan::Scan::default();
    let result =
        scan::package(crate_dir, &mut scan).and_then(|()| assemble(scan.sites, metadata, id));
    scan.diagnostics.sort_by(|a, b| {
        (&a.path, a.line, a.column, &a.reason).cmp(&(&b.path, b.line, b.column, &b.reason))
    });
    scan.diagnostics.dedup();
    match result {
        Ok(manifest) => Ok((manifest, scan.diagnostics)),
        Err(message) => Err(GenerateError {
            message,
            diagnostics: scan.diagnostics,
        }),
    }
}

fn assemble(
    sites: Vec<scan::Site>,
    metadata: &ManifestMetadata,
    id: impl Fn(&str, &str) -> String,
) -> Result<Manifest, String> {
    let mut by_id = BTreeMap::<String, scan::Site>::new();
    for site in sites {
        let key = id(&site.target, &site.text);
        if let Some(previous) = by_id.get_mut(&key) {
            if previous.target != site.target || previous.text != site.text {
                return Err(format!(
                    "template id collision ({key}): {}:{} and {}:{}",
                    previous.path.display(),
                    previous.line,
                    site.path.display(),
                    site.line
                ));
            }
            previous.fields.retain(|name| site.fields.contains(name));
        } else {
            by_id.insert(key, site);
        }
    }
    let templates = by_id
        .into_values()
        .map(|site| {
            let fields = site
                .fields
                .into_iter()
                .map(TemplateField::new)
                .collect::<Result<_, _>>()?;
            Template::new(site.target, site.text, fields)
        })
        .collect::<Result<_, _>>()
        .map_err(|err| format!("invalid template: {err}"))?;
    Manifest::new(
        metadata.module_id.clone(),
        &metadata.repository,
        metadata.logs.clone(),
        templates,
    )
    .map_err(|err| format!("invalid manifest: {err}"))
}

#[doc(hidden)]
pub fn run_cli() -> std::process::ExitCode {
    cli::run()
}
