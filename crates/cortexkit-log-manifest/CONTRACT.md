# Log manifest generator contract

## Generating a manifest

The generator asks Cargo for the package's lib and bin targets and scans their
Rust module trees, including inline modules and modules with `#[path]`. It does
not build the package, expand macros, or resolve imports. It excludes test,
bench and example targets and `#[cfg(test)]` items.

The Rust library's `generate(crate_dir, &ManifestMetadata)` returns a
`cortexkit_doctor::Manifest` and the skipped call sites. It neither prints nor
writes a file. `Manifest::to_canonical_bytes()` produces the exact file bytes
and enforces the 1 MiB limit, including the final newline.

## Command-line use

```text
cortexkit-log-manifest [--check] [--crate <dir>] [--out <path>]
                       [--module-id <id>] [--repository <url>]
                       [--log-glob <glob> ...]
```

The crate directory defaults to `.` and the output to `doctor-manifest.json`
in that directory. In write mode, metadata flags replace the corresponding
metadata from the existing manifest. Creating a manifest requires all three
categories of metadata: module id, repository, and at least one log glob.
Missing metadata at creation is a usage error (exit 2).

In `--check` mode, metadata flags are assertions. A supplied value that differs
from the checked-in metadata fails with exit 1. The command validates the
existing file and checks it byte for byte against the regenerated manifest with
its checked-in metadata. It never writes or fixes the file in this mode.

Skipped call sites are reported on stderr in both modes and do not cause a
failure. A template id collision, invalid or oversized output, or failed check
exits 1 without replacing the previous manifest.
