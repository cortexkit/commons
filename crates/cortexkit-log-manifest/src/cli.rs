use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use cortexkit_doctor::{LogSource, Manifest, ModuleId, MAX_OUTPUT_BYTES};

use crate::{generate, Diagnostic, GenerateError, ManifestMetadata};

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Require the existing manifest to match exactly without writing to it.
    #[arg(long)]
    check: bool,
    /// Directory containing the package Cargo.toml.
    #[arg(long = "crate", value_name = "DIR", default_value = ".")]
    crate_dir: PathBuf,
    /// Manifest path (defaults to doctor-manifest.json in the crate directory).
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
    #[arg(long, value_name = "ID")]
    module_id: Option<String>,
    #[arg(long, value_name = "URL")]
    repository: Option<String>,
    #[arg(long = "log-glob", value_name = "GLOB")]
    log_globs: Vec<String>,
}

#[derive(Debug)]
enum Failure {
    Usage(&'static str),
    Other(String),
}

impl Failure {
    fn code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Other(_) => 1,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Usage(message) => message,
            Self::Other(message) => message,
        }
    }
}

pub(crate) fn run() -> ExitCode {
    let args = Args::parse();
    let mut stderr = io::stderr().lock();
    match execute(args, |path, metadata| generate(path, metadata), &mut stderr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(stderr, "error: {}", err.message());
            ExitCode::from(err.code())
        }
    }
}

fn execute(
    args: Args,
    generate: impl FnOnce(
        &Path,
        &ManifestMetadata,
    ) -> Result<(Manifest, Vec<Diagnostic>), GenerateError>,
    mut stderr: impl Write,
) -> Result<(), Failure> {
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| args.crate_dir.join("doctor-manifest.json"));
    let needs_existing =
        args.module_id.is_none() || args.repository.is_none() || args.log_globs.is_empty();
    let label = |err: &dyn std::fmt::Display| Failure::Other(format!("{}: {err}", out.display()));
    let existing = if args.check || needs_existing {
        match read_existing(&out) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == io::ErrorKind::NotFound && !args.check => {
                return Err(Failure::Usage(
                    "creating a manifest requires --module-id, --repository and --log-glob",
                ));
            }
            Err(err) => return Err(label(&err)),
        }
    } else {
        None
    };
    let stored = existing
        .as_deref()
        .map(Manifest::decode)
        .transpose()
        .map_err(|err| label(&err))?;
    let metadata = metadata(
        args.module_id,
        args.repository,
        args.log_globs,
        stored.as_ref(),
    )?;
    if args.check && stored.as_ref().map(stored_metadata).as_ref() != Some(&metadata) {
        return Err(label(&"metadata flags differ from the checked-in manifest"));
    }

    let result = generate(&args.crate_dir, &metadata);
    let diagnostics = match &result {
        Ok((_, diagnostics)) => diagnostics,
        Err(err) => err.diagnostics(),
    };
    for diagnostic in diagnostics {
        writeln!(stderr, "{diagnostic}").map_err(|err| Failure::Other(err.to_string()))?;
    }
    let (manifest, _) = result.map_err(|err| Failure::Other(err.to_string()))?;
    let bytes = manifest.to_canonical_bytes().map_err(|err| label(&err))?;
    if args.check {
        return if existing.as_deref() == Some(&bytes) {
            Ok(())
        } else {
            Err(label(&"manifest is stale or not canonical"))
        };
    }

    let dir = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // Validate the whole document before replacing the previous manifest.
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|err| label(&err))?;
    tmp.write_all(&bytes).map_err(|err| label(&err))?;
    tmp.persist(&out).map_err(|err| label(&err))?;
    Ok(())
}

fn read_existing(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn stored_metadata(manifest: &Manifest) -> ManifestMetadata {
    ManifestMetadata::new(
        manifest.module_id().clone(),
        manifest.repository(),
        manifest.logs().to_vec(),
    )
}

fn metadata(
    module_id: Option<String>,
    repository: Option<String>,
    log_globs: Vec<String>,
    stored: Option<&Manifest>,
) -> Result<ManifestMetadata, Failure> {
    let module_id = match module_id {
        Some(id) => ModuleId::new(id)
            .map_err(|err| Failure::Other(format!("invalid --module-id: {err}")))?,
        None => stored
            .ok_or(Failure::Usage("missing --module-id"))?
            .module_id()
            .clone(),
    };
    let repository = repository
        .or_else(|| stored.map(|manifest| manifest.repository().to_owned()))
        .ok_or(Failure::Usage("missing --repository"))?;
    let logs = if log_globs.is_empty() {
        stored
            .ok_or(Failure::Usage("missing --log-glob"))?
            .logs()
            .to_vec()
    } else {
        log_globs
            .into_iter()
            .map(LogSource::new)
            .collect::<Result<_, _>>()
            .map_err(|err| Failure::Other(format!("invalid --log-glob: {err}")))?
    };
    Ok(ManifestMetadata::new(module_id, repository, logs))
}

#[cfg(test)]
mod generator_contract;
