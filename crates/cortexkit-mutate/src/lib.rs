#![forbid(unsafe_code)]

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub file: String,
    pub old: String,
    pub new: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub id: String,
    pub guards: String,
    pub file: Option<String>,
    pub old: Option<String>,
    pub new: Option<String>,
    #[serde(default)]
    pub edits: Vec<Edit>,
    pub test_file: String,
    pub runner: String,
    pub package: Option<String>,
    pub target: Option<String>,
    /// Cargo features enabled for every build, list and test in this row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_default_features: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_features: Option<bool>,
    /// An argv template, not a shell string; substitutes one expected test id.
    pub command: Option<Vec<String>>,
    /// Literal output pattern with one {count} decimal placeholder, per invocation.
    pub test_count_pattern: Option<String>,
    /// Compare command stdout rather than relying on a test's exit status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_on: Option<String>,
    /// Regexes deleted from stdout before deterministic output comparison.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_normalize: Vec<String>,
    pub expect_red: Vec<String>,
    /// Substring or /regex/ required in each expected red test's own output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_message: Option<String>,
    /// Why a signal death demonstrates this row's intended failure mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_is_catch: Option<String>,
    #[serde(default)]
    pub only: bool,
    pub equivalent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equivalent_guard: Option<String>,
    /// A person's explanation of why the mutated code has no production caller.
    pub unreachable: Option<String>,
    /// Why this proof requires a real desktop and cannot run in automation.
    pub desk_only: Option<String>,
    /// A person's explanation of why tests in several targets legitimately fail
    /// for this mutant: they assert the same shared property on purpose.
    pub hub: Option<String>,
    /// The test targets, other than the expected tests' own, that a reviewer
    /// approved to fail alongside them in a `run --broad` replay. Stable names,
    /// without Cargo's executable hash.
    pub hub_targets: Option<Vec<String>>,
    /// Rust target-OS names; absent means the row runs on every host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    /// Bounds the test run only; the build has its own deadline below.
    #[serde(default = "default_timeout")]
    pub timeout_s: u64,
    /// Bounds the separate build of the mutant (and the compile inside list
    /// mode). Kept apart from `timeout_s` so a slow compile on a loaded host is
    /// never read as a hung test.
    #[serde(default = "default_build_timeout")]
    pub build_timeout_s: u64,
}
fn default_timeout() -> u64 {
    600
}
pub fn default_build_timeout() -> u64 {
    1800
}

impl Control {
    pub fn matches_platform(&self) -> bool {
        self.platforms
            .as_ref()
            .is_none_or(|names| names.iter().any(|name| name == std::env::consts::OS))
    }

    fn validate_platforms(&self) -> Result<()> {
        if let Some(names) = &self.platforms {
            if names.is_empty() {
                return Err(format!(
                    "{}: platforms must be nonempty when present",
                    self.id
                ));
            }
            for name in names {
                if !known_os(name) {
                    return Err(format!(
                        "{}: unknown target_os in platforms: {name:?}",
                        self.id
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_runner(&self) -> Result<()> {
        self.validate_platforms()?;
        let invalid = |field: &str, reason: &str| format!("{}: {field} {reason}", self.id);
        if self.equivalent.is_some() != self.equivalent_guard.is_some() {
            return Err(invalid(
                "equivalent/equivalent_guard",
                "are required together",
            ));
        }
        if self
            .equivalent
            .as_ref()
            .is_some_and(|s| s.trim().is_empty())
            || self
                .equivalent_guard
                .as_ref()
                .is_some_and(|s| s.trim().is_empty())
        {
            return Err(invalid("equivalent/equivalent_guard", "must be nonempty"));
        }
        if let Some(catch_on) = &self.catch_on {
            if self.runner != "command" || catch_on != "output_differs" {
                return Err(invalid(
                    "catch_on",
                    "must be output_differs on a command row",
                ));
            }
            if self.expect_message.is_some() {
                return Err(invalid(
                    "expect_message",
                    "does not apply to output_differs",
                ));
            }
        } else if !self.output_normalize.is_empty() {
            return Err(invalid(
                "output_normalize",
                "requires catch_on = output_differs",
            ));
        }
        for pattern in &self.output_normalize {
            regex::Regex::new(pattern).map_err(|e| invalid("output_normalize", &e.to_string()))?;
        }
        if self
            .signal_is_catch
            .as_ref()
            .is_some_and(|s| s.trim().is_empty())
        {
            return Err(invalid("signal_is_catch", "requires a nonempty reason"));
        }
        if let Some(pattern) = &self.expect_message {
            message_matches(pattern, "").map_err(|e| invalid("expect_message", &e))?;
        }
        match self.runner.as_str() {
            "command" => {
                for (field, present) in [
                    ("features", self.features.is_some()),
                    ("no_default_features", self.no_default_features.is_some()),
                    ("all_features", self.all_features.is_some()),
                ] {
                    if present {
                        return Err(invalid(field, "must be absent for runner = command"));
                    }
                }
                if self.package.is_some() {
                    return Err(invalid("package", "must be absent for runner = command"));
                }
                if self.target.is_some() {
                    return Err(invalid("target", "must be absent for runner = command"));
                }
                if self.only {
                    return Err(invalid("only", "cannot be true for runner = command"));
                }
                let argv = self
                    .command
                    .as_ref()
                    .ok_or_else(|| invalid("command", "is required"))?;
                if argv.first().is_none_or(|s| s.trim().is_empty())
                    || argv.iter().any(|s| s.contains('\0'))
                {
                    return Err(invalid(
                        "command",
                        "requires a nonempty program and NUL-free argv",
                    ));
                }
                if self.catch_on.is_some() {
                    if argv.iter().any(|s| s.contains("{test}")) {
                        return Err(invalid("command", "output_differs does not use {test}"));
                    }
                    if self.test_count_pattern.is_some() || !self.expect_red.is_empty() {
                        return Err(invalid(
                            "catch_on",
                            "output_differs requires expect_red = [] and no test_count_pattern",
                        ));
                    }
                } else {
                    if argv
                        .iter()
                        .map(|s| s.matches("{test}").count())
                        .sum::<usize>()
                        != 1
                    {
                        return Err(invalid("command", "must contain {test} exactly once"));
                    }
                    if !argv.iter().any(|s| s == "{test}") {
                        return Err(invalid(
                            "command",
                            "requires {test} as a complete argv element",
                        ));
                    }
                    validate_count_pattern(self.test_count_pattern.as_deref())
                        .map_err(|e| invalid("test_count_pattern", &e))?;
                    if self.expect_red.is_empty()
                        && self.unreachable.is_none()
                        && self.desk_only.is_none()
                    {
                        return Err(invalid("expect_red", "must name at least one test"));
                    }
                    for id in &self.expect_red {
                        if id.is_empty() || id.chars().any(|c| c.is_whitespace() || c.is_control())
                        {
                            return Err(invalid("expect_red", &format!("invalid test id {id:?}: ids must be nonempty without whitespace or control characters")));
                        }
                    }
                }
            }
            "cargo" | "nextest" => {
                if self.test_count_pattern.is_some() {
                    return Err(invalid(
                        "test_count_pattern",
                        "must be absent for cargo/nextest rows",
                    ));
                }
                if self.command.is_some() {
                    return Err(invalid("command", "must be absent for cargo/nextest rows"));
                }
                if self.package.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    return Err(invalid("package", "is required and must be nonempty"));
                }
                self.targets()?;
                self.feature_args()?;
            }
            _ => return Err(invalid("runner", "must be cargo, nextest, or command")),
        }
        if self.timeout_s == 0 {
            return Err(invalid("timeout_s", "must be positive"));
        }
        if self.runner != "command" && self.build_timeout_s == 0 {
            return Err(invalid("build_timeout_s", "must be positive"));
        }
        Ok(())
    }

    fn validate_hub(&self) -> Result<()> {
        if self.hub.is_none() && self.hub_targets.is_none() {
            return Ok(());
        }
        if self.equivalent.is_some() || self.unreachable.is_some() || self.desk_only.is_some() {
            return Err(format!(
                "{}: hub, equivalent and unreachable are exclusive; desk_only is also exclusive",
                self.id
            ));
        }
        let reason = self
            .hub
            .as_deref()
            .ok_or_else(|| format!("{}: HUB requires a hub reason", self.id))?;
        if reason.trim().chars().count() < 20 {
            return Err(format!(
                "{}: HUB reason must be at least 20 characters after trimming",
                self.id
            ));
        }
        if self.hub_targets.as_ref().is_none_or(|targets| {
            targets.is_empty() || targets.iter().any(|target| target.trim().is_empty())
        }) {
            return Err(format!("{}: HUB requires non-empty hub_targets", self.id));
        }
        Ok(())
    }

    fn recorded_disposition(&self) -> Result<Option<(Outcome, &str)>> {
        self.validate_hub()?;
        if [
            self.equivalent.is_some(),
            self.unreachable.is_some(),
            self.desk_only.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            > 1
        {
            return Err(format!(
                "{}: equivalent, unreachable and desk_only are exclusive",
                self.id
            ));
        }
        let disposition = self
            .unreachable
            .as_deref()
            .map(|r| (Outcome::Unreachable, r))
            .or_else(|| self.desk_only.as_deref().map(|r| (Outcome::DeskOnly, r)));
        if disposition
            .as_ref()
            .is_some_and(|(_, reason)| reason.trim().is_empty())
        {
            return Err(format!(
                "{}: recorded disposition requires a non-empty reason",
                self.id
            ));
        }
        Ok(disposition)
    }

    pub fn edits(&self) -> Result<Vec<Edit>> {
        match (&self.file, &self.old, &self.new, self.edits.is_empty()) {
            (Some(file), Some(old), Some(new), true) => Ok(vec![Edit {
                file: file.clone(),
                old: old.clone(),
                new: new.clone(),
            }]),
            (None, None, None, false) => Ok(self.edits.clone()),
            _ => Err(format!(
                "{}: use file/old/new or edits, exclusively",
                self.id
            )),
        }
    }
    fn targets(&self) -> Result<Vec<&str>> {
        let target = self.target.as_deref().unwrap_or_default();
        let words: Vec<_> = target.split_whitespace().collect();
        let mut i = 0;
        while i < words.len() {
            match words[i] {
                "--lib" | "--tests" | "--bins" | "--examples" | "--all-targets" => i += 1,
                "--test" | "--bin" | "--example" | "--bench"
                    if i + 1 < words.len() && !words[i + 1].starts_with('-') =>
                {
                    i += 2
                }
                _ => {
                    return Err(format!(
                        "{}: invalid cargo target selector {}",
                        self.id, target
                    ))
                }
            }
        }
        Ok(words)
    }

    fn feature_args(&self) -> Result<Vec<String>> {
        if self.all_features == Some(true)
            && (self.features.is_some() || self.no_default_features == Some(true))
        {
            return Err(format!(
                "{}: all_features is mutually exclusive with features and no_default_features",
                self.id
            ));
        }
        let mut args = vec![];
        if let Some(features) = &self.features {
            for name in features {
                if name.is_empty()
                    || name.starts_with('-')
                    || name.contains(',')
                    || name.chars().any(|c| c.is_whitespace() || c.is_control())
                {
                    return Err(format!("{}: invalid features name {name:?}: must be nonempty without whitespace, control characters, leading '-' or commas", self.id));
                }
            }
            if !features.is_empty() {
                args.extend(["--features".into(), features.join(",")]);
            }
        }
        if self.no_default_features == Some(true) {
            args.push("--no-default-features".into());
        }
        if self.all_features == Some(true) {
            args.push("--all-features".into());
        }
        Ok(args)
    }
}

fn known_os(name: &str) -> bool {
    matches!(
        name,
        "aix"
            | "android"
            | "cuda"
            | "dragonfly"
            | "emscripten"
            | "espidf"
            | "freebsd"
            | "fuchsia"
            | "haiku"
            | "hermit"
            | "horizon"
            | "hurd"
            | "illumos"
            | "ios"
            | "l4re"
            | "linux"
            | "macos"
            | "netbsd"
            | "none"
            | "nto"
            | "nuttx"
            | "openbsd"
            | "psp"
            | "psx"
            | "redox"
            | "rtems"
            | "solid_asp3"
            | "solaris"
            | "teeos"
            | "trusty"
            | "tvos"
            | "uefi"
            | "unknown"
            | "vexos"
            | "visionos"
            | "vita"
            | "vxworks"
            | "wasi"
            | "watchos"
            | "windows"
            | "xous"
            | "zkvm"
    )
}

/// A named, shell-free prerequisite run from the repository root. Rebuilding
/// every prerequisite under every mutant avoids trusting stale fixture binaries.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Prebuild {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default = "default_build_timeout")]
    pub timeout_s: u64,
}

fn validate_prebuild(steps: &[Prebuild]) -> Result<()> {
    let mut names = BTreeSet::new();
    for step in steps {
        if step.name.trim().is_empty() || !names.insert(&step.name) {
            return Err(format!(
                "invalid or duplicate prebuild name: {:?}",
                step.name
            ));
        }
        if step.command.first().is_none_or(|arg| arg.trim().is_empty())
            || step.command.iter().any(|arg| arg.contains('\0'))
            || step.timeout_s == 0
        {
            return Err(format!(
                "prebuild {:?}: requires a program, NUL-free argv and positive timeout_s",
                step.name
            ));
        }
        if Path::new(&step.command[0])
            .file_stem()
            .is_some_and(|p| p == "cargo")
            && !step
                .command
                .iter()
                .take_while(|arg| arg.as_str() != "--")
                .any(|arg| arg == "--locked")
        {
            return Err(format!(
                "prebuild {:?}: cargo commands require --locked",
                step.name
            ));
        }
    }
    Ok(())
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalogue {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prebuild: Vec<Prebuild>,
    #[serde(default)]
    pub control: Vec<Control>,
}

pub fn load(path: &Path) -> Result<Catalogue> {
    // Deserialize each row separately so malformed fields still name its id.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Rows {
        #[serde(default)]
        prebuild: Vec<Prebuild>,
        #[serde(default)]
        control: Vec<toml::Value>,
    }
    let rows: Rows = toml::from_str(&fs::read_to_string(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let control = rows
        .control
        .into_iter()
        .map(|row| {
            let id = row
                .get("id")
                .and_then(toml::Value::as_str)
                .unwrap_or("<missing id>")
                .to_owned();
            let c: Control = row.try_into().map_err(|e| format!("{id}: {e}"))?;
            c.validate_runner()?;
            c.recorded_disposition()?;
            Ok(c)
        })
        .collect::<Result<Vec<_>>>()?;
    validate_prebuild(&rows.prebuild)?;
    Ok(Catalogue {
        control,
        prebuild: rows.prebuild,
    })
}

fn target_io_error(name: &str, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("ANCHOR_MISSING {name}: {error}")
    } else {
        format!("{name}: {error}")
    }
}

fn safe_path(root: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("not a repository-relative path: {name}"));
    }
    let full = root.join(path);
    let resolved = full.canonicalize().map_err(|e| target_io_error(name, e))?;
    if !resolved.starts_with(root.canonicalize().map_err(|e| e.to_string())?) || !resolved.is_file()
    {
        return Err(format!("not a regular file inside repository: {name}"));
    }
    if fs::symlink_metadata(&full)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err(format!("symlink target refused: {name}"));
    }
    Ok(full)
}

pub fn validate(root: &Path, catalogue: &Catalogue) -> Result<()> {
    validate_prebuild(&catalogue.prebuild)?;
    let mut ids = BTreeSet::new();
    for c in &catalogue.control {
        if c.id.is_empty()
            || !c
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !ids.insert(&c.id)
        {
            return Err(format!("invalid or duplicate id: {}", c.id));
        }
        if c.guards.trim().is_empty()
            || (c.expect_red.is_empty()
                && c.unreachable.is_none()
                && c.desk_only.is_none()
                && c.equivalent.is_none()
                && c.catch_on.is_none())
            || c.expect_red.iter().any(|n| n.trim().is_empty())
            || c.equivalent.as_ref().is_some_and(|s| s.trim().is_empty())
        {
            return Err(format!("{}: invalid required field", c.id));
        }
        safe_path(root, &c.test_file)?;
        validate_mutant(root, c)?;
    }
    Ok(())
}

/// The checks a control needs before it may be replayed at all: a runnable
/// command (package, runner, timeout, target selector) and well-formed edits.
/// Catalogue validation and `explore` share it; `explore` has no catalogue
/// fields (id, guards, test file, expected names) until it appends a row.
pub fn validate_mutant(root: &Path, c: &Control) -> Result<()> {
    c.recorded_disposition()?;
    c.validate_runner()?;
    for edit in c.edits()? {
        // Deleted edit targets are row outcomes, not catalogue-wide errors.
        // Check mode still rejects them when it verifies the anchors.
        match safe_path(root, &edit.file) {
            Ok(_) => {}
            Err(e) if e.starts_with("ANCHOR_MISSING ") => {}
            Err(e) => return Err(e),
        }
        if edit.old.is_empty() || edit.old == edit.new {
            return Err(format!("{}: empty or unchanged edit", c.id));
        }
    }
    Ok(())
}

/// Append one control to the catalogue file as a new `[[control]]` table.
/// `prove` and `explore --append` both write rows through this function.
pub fn append_control(path: &Path, c: &Control) -> Result<()> {
    use std::io::Write;
    // HUB records a person's review of an observed broad catch, not a discovery
    // that prove or explore may assign automatically.
    if c.hub.is_some() || c.hub_targets.is_some() {
        return Err(format!(
            "{}: cannot append HUB; review a broad catch and edit the catalogue manually",
            c.id
        ));
    }
    let row = toml::to_string(&Catalogue {
        control: vec![c.clone()],
        ..Catalogue::default()
    })
    .map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "\n{row}").map_err(|e| e.to_string())
}

/// Parse `explore --edits`: a JSON array of `{file, old, new}` objects, a JSON
/// or TOML document with an `edits` array, or a bare TOML inline array.
pub fn parse_edits(text: &str) -> Result<Vec<Edit>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Doc {
        edits: Vec<Edit>,
    }
    if let Ok(edits) = serde_json::from_str::<Vec<Edit>>(text) {
        return Ok(edits);
    }
    if let Ok(doc) = serde_json::from_str::<Doc>(text) {
        return Ok(doc.edits);
    }
    toml::from_str::<Doc>(text)
        .or_else(|_| toml::from_str::<Doc>(&format!("edits = {text}")))
        .map(|doc| doc.edits)
        .map_err(|e| format!("--edits is neither JSON nor TOML edits: {e}"))
}

/// Infer only literal, whole-file OS gates: an inner cfg attribute or a direct
/// conventional `mod name;` declaration beside the file. Compound cfgs, macros,
/// path attributes and gates on just one item need an author's platform field.
pub fn infer_platforms(root: &Path, edits: &[Edit]) -> Option<Vec<String>> {
    let mut platforms = BTreeSet::new();
    for edit in edits {
        let file = Path::new(&edit.file);
        if file.extension()?.to_str()? != "rs" {
            return None;
        }
        let text = fs::read_to_string(root.join(file)).ok()?;
        let mut found = visible_platform(&text, None);
        if found.is_none() {
            let parent = file.parent()?;
            let (directory, name) = if file.file_stem()? == "mod" {
                (parent.parent()?, parent.file_name()?.to_str()?)
            } else {
                (parent, file.file_stem()?.to_str()?)
            };
            let candidates = [
                directory.join("mod.rs"),
                directory.join("lib.rs"),
                directory.join("main.rs"),
                directory.with_extension("rs"),
            ];
            for candidate in candidates {
                if candidate == file {
                    continue;
                }
                if let Ok(source) = fs::read_to_string(root.join(candidate)) {
                    if let Some(os) = visible_platform(&source, Some(name)) {
                        if found.as_ref().is_some_and(|previous| previous != &os) {
                            return None;
                        }
                        found = Some(os);
                    }
                }
            }
        }
        platforms.insert(found?);
    }
    if platforms.len() == 1 {
        Some(platforms.into_iter().collect())
    } else {
        None
    }
}

fn visible_platform(text: &str, module: Option<&str>) -> Option<String> {
    let toks = tokens(text);
    // A path attribute can redirect a mod declaration away from its conventional
    // sibling file. Do not infer that file's gate from a potentially redirected
    // declaration, even when another attribute sits between path and cfg.
    if module.is_some()
        && toks.windows(3).any(|slice| {
            slice
                .iter()
                .map(|t| t.1)
                .eq([Tok::Punct(b'#'), Tok::Punct(b'['), Tok::Ident("path")])
        })
    {
        return None;
    }
    let mut braces = 0usize;
    for (i, (_, tok)) in toks.iter().enumerate() {
        match tok {
            Tok::Punct(b'{') => braces += 1,
            Tok::Punct(b'}') => braces = braces.saturating_sub(1),
            _ => {}
        }
        if braces != 0 || *tok != Tok::Punct(b'#') {
            continue;
        }
        let inner = toks.get(i + 1).is_some_and(|t| t.1 == Tok::Punct(b'!'));
        let start = i + 1 + usize::from(inner);
        let shape = [
            Tok::Punct(b'['),
            Tok::Ident("cfg"),
            Tok::Punct(b'('),
            Tok::Ident("target_os"),
            Tok::Punct(b'='),
            Tok::Punct(b')'),
            Tok::Punct(b']'),
        ];
        if !toks
            .get(start..start + shape.len())
            .is_some_and(|slice| slice.iter().map(|t| t.1).eq(shape))
        {
            continue;
        }
        let literal = text[toks[start + 4].0 + 1..toks[start + 5].0].trim();
        let os = literal.strip_prefix('"')?.strip_suffix('"')?;
        if !known_os(os) {
            continue;
        }
        if inner && module.is_none() {
            return Some(os.to_owned());
        }
        if !inner {
            let mut next = start + shape.len();
            if toks.get(next).is_some_and(|t| t.1 == Tok::Ident("pub")) {
                next += 1;
            }
            if let Some(name) = module {
                if toks.get(next..next + 3).is_some_and(|slice| {
                    slice.iter().map(|t| t.1).eq([
                        Tok::Ident("mod"),
                        Tok::Ident(name),
                        Tok::Punct(b';'),
                    ])
                }) {
                    return Some(os.to_owned());
                }
            }
        }
    }
    None
}

pub fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(out.stdout)
}

pub fn repository() -> Result<PathBuf> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let bytes = git(&cwd, &["rev-parse", "--show-toplevel"])?;
    PathBuf::from(String::from_utf8_lossy(&bytes).trim())
        .canonicalize()
        .map_err(|e| e.to_string())
}

pub struct TreeLock {
    file: File,
}
impl TreeLock {
    pub fn acquire(root: &Path) -> Result<Self> {
        let dir = git(root, &["rev-parse", "--git-path", "ck-mutate.lock"])?;
        let path = root.join(String::from_utf8_lossy(&dir).trim());
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.try_lock_exclusive()
            .map_err(|e| format!("mutation tree is locked: {e}"))?;
        Ok(Self { file })
    }
}
impl Drop for TreeLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

struct Saved {
    files: BTreeMap<PathBuf, Vec<u8>>,
    lock: PathBuf,
    lock_bytes: Option<Vec<u8>>,
    active: bool,
}
impl Saved {
    fn unchanged(&self) -> Result<bool> {
        Ok(self
            .files
            .iter()
            .all(|(path, bytes)| fs::read(path).ok().as_ref() == Some(bytes))
            && read_optional(&self.lock)? == self.lock_bytes)
    }

    fn new(root: &Path, edits: &[Edit], allow_dirty: bool) -> Result<Self> {
        let mut files = BTreeMap::new();
        for e in edits {
            let path = safe_path(root, &e.file)?;
            let bytes = fs::read(&path).map_err(|err| target_io_error(&e.file, err))?;
            if !allow_dirty && git(root, &["show", &format!("HEAD:{}", e.file)])? != bytes {
                return Err(format!(
                    "dirty target refused: {} (use --allow-dirty)",
                    e.file
                ));
            }
            files.insert(path, bytes);
        }
        let lock = root.join("Cargo.lock");
        let lock_bytes = read_optional(&lock)?;
        Ok(Self {
            files,
            lock,
            lock_bytes,
            active: true,
        })
    }
    fn restore(&mut self) -> Result<()> {
        // A listed lockfile is restored and verified with the other edit targets.
        // Only an unlisted lockfile change is an unexpected side effect of a row.
        let changed_lock =
            !self.files.contains_key(&self.lock) && read_optional(&self.lock)? != self.lock_bytes;
        let mut errors = Vec::new();
        for (path, bytes) in &self.files {
            if let Err(e) = fs::write(path, bytes).and_then(|()| {
                if fs::read(path)? != *bytes {
                    return Err(std::io::Error::other("byte-for-byte mismatch"));
                }
                Ok(())
            }) {
                errors.push(format!("restore failed {}: {e}", path.display()));
            }
        }
        if changed_lock {
            let result = match &self.lock_bytes {
                Some(b) => fs::write(&self.lock, b),
                None => fs::remove_file(&self.lock),
            };
            if let Err(e) = result {
                errors.push(format!("restore failed Cargo.lock: {e}"));
            }
            if read_optional(&self.lock)? != self.lock_bytes {
                errors.push("restore mismatch Cargo.lock".into());
            }
        }
        if errors.is_empty() {
            self.active = false;
        }
        if changed_lock {
            errors.push("Cargo.lock changed during row".into());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
impl Drop for Saved {
    fn drop(&mut self) {
        if self.active {
            if let Err(e) = self.restore() {
                eprintln!("hard restoration error: {e}");
            }
        }
    }
}

// Apply sequential edits in memory before writing anything. The replacement count
// is the anchor check, including for multi-line anchors and repeated-file edits.
fn replaced(root: &Path, edits: &[Edit]) -> Result<BTreeMap<PathBuf, String>> {
    let mut files = BTreeMap::new();
    for edit in edits {
        let path = safe_path(root, &edit.file)?;
        if !files.contains_key(&path) {
            files.insert(
                path.clone(),
                fs::read_to_string(&path).map_err(|e| e.to_string())?,
            );
        }
        let text = files.get_mut(&path).expect("inserted file");
        let mut count = 0;
        let replacement =
            text.split(&edit.old)
                .enumerate()
                .fold(String::new(), |mut out, (i, part)| {
                    if i > 0 {
                        count += 1;
                        out.push_str(&edit.new);
                    }
                    out.push_str(part);
                    out
                });
        if count != 1 {
            return Err(format!(
                "ANCHOR_MISSING {}: replaced {count} occurrences",
                edit.file
            ));
        }
        *text = replacement;
    }
    Ok(files)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    Caught,
    CaughtBroadly,
    Survived,
    WrongTest,
    RedForAnotherReason,
    NoTestsRan,
    AnchorMissing,
    DidNotCompile,
    TimedOut,
    Equivalent,
    EquivalentCaught,
    Unreachable,
    Hub,
    SkippedPlatform,
    DeskOnly,
    Error,
}
impl Outcome {
    pub fn is_caught(&self) -> bool {
        matches!(self, Self::Caught | Self::CaughtBroadly | Self::Hub)
    }
}
/// Which deadline expired (command rows report timeouts as ERROR).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// The separate `--no-run` build, bounded by `build_timeout_s`.
    Build,
    /// The test run, bounded by `timeout_s`.
    Test,
    /// A declared fixture prerequisite, bounded by that step's timeout_s.
    Prebuild,
}

#[derive(Debug, Clone, Serialize)]
pub struct Collateral {
    pub count: usize,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub id: String,
    pub outcome: Outcome,
    /// The phase whose deadline expired, including command-row ERROR timeouts.
    pub timed_out_phase: Option<Phase>,
    pub red: Vec<String>,
    pub green: Vec<String>,
    /// Each red test's own mutant output, bounded to 16 KiB per entry.
    pub failures: BTreeMap<String, String>,
    /// Each baseline-red test's own output, separate from mutant failures.
    pub baseline_failures: BTreeMap<String, String>,
    #[serde(skip)]
    unexpected_red: BTreeSet<String>,
    pub collateral: Collateral,
    /// True only after an opt-in broad audit produced complete test results.
    /// A normal replay reports collateral but does not observe package breadth.
    pub breadth_observed: bool,
    pub build_ms: u128,
    pub test_ms: u128,
    /// Clean-tree costs, attributed once per shared target selection.
    pub baseline_build_ms: u128,
    pub baseline_test_ms: u128,
    /// Stable target -> baseline-red test names, retained even when now green.
    pub baseline_red: BTreeMap<String, Vec<String>>,
    #[serde(skip)]
    baseline_results: Option<TestResults>,
    #[serde(skip)]
    baseline_stdout: Option<String>,
    /// Per-mutant prerequisites, separate from the normal test-binary build.
    pub prebuild_ms: u128,
    pub prebuild_tail: String,
    /// Session preparation is attributed only to the first executable row.
    pub baseline_prebuild_ms: u128,
    pub baseline_prebuild_tail: String,
    /// Final refresh on the restored tree, attributed to the last reported row.
    pub restore_prebuild_ms: u128,
    pub restore_prebuild_tail: String,
    #[serde(skip)]
    restore_prebuild_needed: bool,
    pub build_tail: String,
    pub test_tail: String,
    pub reason: Option<String>,
}
impl Report {
    fn new(c: &Control) -> Self {
        Self {
            id: c.id.clone(),
            outcome: Outcome::Error,
            timed_out_phase: None,
            red: vec![],
            green: vec![],
            failures: BTreeMap::new(),
            baseline_failures: BTreeMap::new(),
            unexpected_red: BTreeSet::new(),
            collateral: Collateral {
                count: 0,
                targets: vec![],
            },
            breadth_observed: false,
            build_ms: 0,
            test_ms: 0,
            baseline_build_ms: 0,
            baseline_test_ms: 0,
            baseline_red: BTreeMap::new(),
            baseline_results: None,
            baseline_stdout: None,
            prebuild_ms: 0,
            prebuild_tail: String::new(),
            baseline_prebuild_ms: 0,
            baseline_prebuild_tail: String::new(),
            restore_prebuild_ms: 0,
            restore_prebuild_tail: String::new(),
            restore_prebuild_needed: false,
            build_tail: String::new(),
            test_tail: String::new(),
            reason: None,
        }
    }
    pub fn passes(&self) -> bool {
        self.outcome.is_caught()
            || matches!(
                self.outcome,
                Outcome::Equivalent
                    | Outcome::Unreachable
                    | Outcome::SkippedPlatform
                    | Outcome::DeskOnly
            )
    }

    /// Failures needing explanation: collateral reds and message mismatches.
    pub fn unexpected_failures(&self) -> impl Iterator<Item = (&str, &str)> {
        self.unexpected_red.iter().filter_map(|name| {
            self.failures
                .get(name)
                .map(|output| (name.as_str(), output.as_str()))
        })
    }
}

struct Output {
    success: bool,
    code: Option<i32>,
    signal: Option<String>,
    timeout: bool,
    interrupted: bool,
    text: String,
    stdout: String,
    ms: u128,
}
/// Which tests one replay runs and how their per-test results are graded.
/// Every scope goes through the same `replay` body (tree state, build, test,
/// parse, restore); only the command's selection and the grading differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// The row's package and target selector, graded against `expect_red`.
    Row,
    /// Opt-in audit: all package test targets plus the row's explicit targets,
    /// graded against `expect_red`, including cross-target collateral.
    Broad,
    /// The row's whole package without target selection, graded against
    /// `expect_red` (the second replay `prove` makes for a survivor).
    Package,
    /// `explore`: the whole package, or the whole workspace; any red test is a
    /// catch because nobody has named the guarding tests yet.
    Explore { workspace: bool },
}

fn command(c: &Control, mode: &str, scope: Scope) -> Result<Command> {
    let package = c.package.as_deref().unwrap_or_default();
    let mut cmd = Command::new("cargo");
    if c.runner == "nextest" {
        cmd.args(["nextest", if mode == "list" { "list" } else { "run" }]);
    } else {
        cmd.arg("test");
    }
    cmd.arg("--locked");
    match scope {
        Scope::Row => {
            cmd.args(["-p", package]);
            cmd.args(c.targets()?);
        }
        Scope::Broad => {
            cmd.args(["-p", package]);
            let targets = c.targets()?;
            // Keep explicitly selected examples or benches too: their expected
            // tests need not be included by Cargo's `--tests` selector.
            if !targets.contains(&"--tests") {
                cmd.arg("--tests");
            }
            cmd.args(targets);
        }
        Scope::Package | Scope::Explore { workspace: false } => {
            cmd.args(["-p", package]);
        }
        Scope::Explore { workspace: true } => {
            cmd.arg("--workspace");
        }
    }
    cmd.args(c.feature_args()?);
    match mode {
        "build" => {
            cmd.arg("--no-run");
        }
        "list" if c.runner == "cargo" => {
            cmd.args(["--", "--list"]);
        }
        "list" => {
            cmd.args(["--message-format", "json"]);
        }
        _ if c.runner == "cargo" => {
            cmd.args(["--no-fail-fast", "--", "--test-threads=1"]);
        }
        _ => {
            cmd.args(["--no-fail-fast", "--retries", "0"]);
            let help = Command::new("cargo")
                .args(["nextest", "run", "--help"])
                .output()
                .map_err(|e| e.to_string())?;
            if help.status.success()
                && String::from_utf8_lossy(&help.stdout).contains("libtest-json")
            {
                cmd.env("NEXTEST_EXPERIMENTAL_LIBTEST_JSON", "1")
                    .args(["--message-format", "libtest-json"]);
            }
            cmd.args([
                "--status-level",
                "all",
                "--final-status-level",
                "all",
                "--color",
                "never",
            ]);
        }
    }
    Ok(cmd)
}
fn execute(root: &Path, cmd: Command, timeout: u64, stop: &AtomicBool) -> Result<Output> {
    execute_output(root, cmd, timeout, stop, false)
}

fn execute_output(
    root: &Path,
    mut cmd: Command,
    timeout: u64,
    stop: &AtomicBool,
    separate_stdout: bool,
) -> Result<Output> {
    // Regular files cannot deadlock on full pipes or descendants holding pipes open.
    let git_dir = git(root, &["rev-parse", "--git-dir"])?;
    let dir = root.join(String::from_utf8_lossy(&git_dir).trim());
    let stdout = dir.join("ck-mutate.stdout");
    let out_file = File::create(&stdout).map_err(|e| e.to_string())?;
    // Cargo prints binary headers on stderr and test events on stdout. Sharing
    // the file offset preserves their order, so events keep their binary identity.
    let stderr = dir.join("ck-mutate.stderr");
    let err_file = if separate_stdout {
        File::create(&stderr)
    } else {
        out_file.try_clone()
    }
    .map_err(|e| e.to_string())?;
    cmd.current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let start = Instant::now();
    let mut child = RunningChild(cmd.spawn().map_err(|e| e.to_string())?);
    let mut timed_out = false;
    let mut interrupted = false;
    let status = loop {
        if stop.load(Ordering::SeqCst) || start.elapsed() >= Duration::from_secs(timeout) {
            interrupted = stop.load(Ordering::SeqCst);
            timed_out = !interrupted;
            kill_tree(&mut child.0);
            let _ = child.0.wait();
            break None;
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            break Some(status);
        }
        thread::sleep(Duration::from_millis(20));
    };
    drop(child);
    let stdout_text = fs::read_to_string(&stdout).map_err(|e| e.to_string())?;
    let text = if separate_stdout {
        format!(
            "{}{}",
            stdout_text,
            fs::read_to_string(&stderr).map_err(|e| e.to_string())?
        )
    } else {
        stdout_text.clone()
    };
    let _ = fs::remove_file(stdout);
    if separate_stdout {
        let _ = fs::remove_file(stderr);
    }
    Ok(Output {
        success: status.is_some_and(|s| s.success()),
        code: status.and_then(|s| s.code()),
        signal: status.and_then(exit_signal),
        timeout: timed_out,
        interrupted,
        text,
        stdout: stdout_text,
        ms: start.elapsed().as_millis(),
    })
}

fn exit_signal(status: std::process::ExitStatus) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(|number| {
            format!(
                "signal {number} ({:?})",
                rustix::process::Signal::from_named_raw(number)
            )
        })
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

fn message_matches(pattern: &str, output: &str) -> Result<bool> {
    if pattern.trim().is_empty() {
        return Err("must be nonempty".into());
    }
    if let Some(regex) = pattern.strip_prefix('/').and_then(|s| s.strip_suffix('/')) {
        if regex.is_empty() {
            return Err("regex must be nonempty".into());
        }
        regex::Regex::new(regex)
            .map(|regex| regex.is_match(output))
            .map_err(|e| format!("invalid regex: {e}"))
    } else {
        Ok(output.contains(pattern))
    }
}

fn wrong_message(c: &Control, id: &str, output: &str) -> Result<Option<String>> {
    if let Some(pattern) = &c.expect_message {
        if !message_matches(pattern, output)? {
            let first_lines = output.lines().take(8).collect::<Vec<_>>().join("\n");
            return Ok(Some(format!(
                "{id}: failure output did not match expect_message = {pattern:?}; actual failure:\n{}",
                if first_lines.is_empty() { "<no failure output>" } else { &first_lines }
            )));
        }
    }
    Ok(None)
}
struct RunningChild(std::process::Child);
impl Drop for RunningChild {
    fn drop(&mut self) {
        kill_tree(&mut self.0);
        let _ = self.0.wait();
    }
}
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    // The child was placed in its own process group before exec.
    {
        use rustix::process::{kill_process_group, Pid, Signal};
        let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .status();
    }
    let _ = child.kill();
}
fn tail(text: &str) -> String {
    let start = text.char_indices().rev().nth(7999).map_or(0, |(i, _)| i);
    text[start..].to_owned()
}

fn bounded_failure(text: &str) -> String {
    const LIMIT: usize = 16 * 1024;
    const HEAD: usize = 4 * 1024;
    if text.len() <= LIMIT {
        return text.to_owned();
    }
    let mut head_end = HEAD;
    while !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = text.len() - (LIMIT - head_end);
    loop {
        while !text.is_char_boundary(tail_start) {
            tail_start += 1;
        }
        let marker = format!("\n[… {} bytes elided …]\n", tail_start - head_end);
        let size = head_end + marker.len() + text.len() - tail_start;
        if size <= LIMIT {
            return format!("{}{marker}{}", &text[..head_end], &text[tail_start..]);
        }
        // The marker counts toward the cap; reserve its bytes from the tail.
        tail_start += size - LIMIT;
    }
}

fn validate_count_pattern(pattern: Option<&str>) -> Result<(&str, &str)> {
    let pattern = pattern.ok_or("is required for command rows")?;
    if pattern.matches("{count}").count() != 1 || pattern.contains('\0') {
        return Err("requires exactly one {count} placeholder and no NUL".into());
    }
    let (before, after) = pattern.split_once("{count}").unwrap();
    if before.is_empty() || after.is_empty() {
        return Err("requires nonempty literal text before and after {count}".into());
    }
    Ok((before, after))
}

fn command_test_count(text: &str, pattern: Option<&str>) -> Result<u64> {
    let (before, after) = validate_count_pattern(pattern)?;
    let mut counts = Vec::new();
    for (start, _) in text.match_indices(before) {
        let rest = &text[start + before.len()..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(after) {
            counts.push(
                rest[..digits]
                    .parse::<u64>()
                    .map_err(|e| format!("invalid test count: {e}"))?,
            );
        }
    }
    match counts.as_slice() {
        [count] if *count > 0 => Ok(*count),
        [0] => Err("command reported zero tests executed".into()),
        [] => Err("test_count_pattern did not match an executed-test count".into()),
        _ => Err("test_count_pattern matched multiple counts; execution is ambiguous".into()),
    }
}

#[cfg(test)]
mod count_tests {
    use super::*;

    #[test]
    fn count_pattern_requires_one_unambiguous_nonzero_decimal_count() {
        let pattern = Some("Ran {count} tests");
        assert_eq!(
            command_test_count("Ran 2 tests in 0.1s", pattern).unwrap(),
            2
        );
        for text in [
            "Ran 0 tests",
            "Ran two tests",
            "Ran 1 tests\nRan 2 tests",
            "Ran 18446744073709551616 tests",
            "",
        ] {
            assert!(command_test_count(text, pattern).is_err(), "{text}");
        }
    }
}

// Both baseline and mutant commands use the same child supervisor as Cargo.
// A failing baseline or a broken process is not evidence that a test caught a mutant.
fn command_tests(
    root: &Path,
    c: &Control,
    stop: &AtomicBool,
    report: &mut Report,
    baseline: bool,
) -> Result<()> {
    if c.catch_on.is_some() {
        return output_tests(root, c, stop, report, baseline);
    }
    let argv = c.command.as_ref().ok_or("command is required")?;
    for id in &c.expect_red {
        // The error label states a verdict, so it is used only when the run fails;
        // the output heading names the phase and stays neutral for green runs.
        let (label, heading) = if baseline {
            ("baseline was not green", "baseline output")
        } else {
            ("mutated command", "mutated command output")
        };
        let error = |reason: String| format!("{}: {id}: {label}: {reason}", c.id);
        let args: Vec<_> = argv
            .iter()
            .map(|arg| if arg == "{test}" { id } else { arg })
            .collect();
        let mut cmd = Command::new(args[0]);
        cmd.args(&args[1..]);
        let output = execute(root, cmd, c.timeout_s, stop)
            .map_err(|e| error(format!("spawn/execution failed: {e}")))?;
        if baseline {
            report.baseline_test_ms += output.ms;
        } else {
            report.test_ms += output.ms;
        }
        report.test_tail = tail(&format!(
            "{}\n{heading}: {id}\n{}",
            report.test_tail, output.text
        ));
        if output.interrupted {
            return Err(error("interrupted".into()));
        }
        if output.timeout {
            report.timed_out_phase = Some(Phase::Test);
            return Err(error(format!(
                "test command exceeded timeout_s = {}",
                c.timeout_s
            )));
        }
        let code = match output.code {
            Some(code) => code,
            None => {
                let signal = output
                    .signal
                    .as_deref()
                    .unwrap_or("unknown signal (no exit code)");
                if baseline || c.signal_is_catch.is_none() {
                    return Err(error(format!("process died by {signal}")));
                }
                // The opt-in does not waive the executed-test count or message proof.
                1
            }
        };
        if code == 126 {
            return Err(error(
                "exit 126: not executable, or the command refused to run".into(),
            ));
        }
        if code == 127 {
            return Err(error("exit 127: program not found".into()));
        }
        command_test_count(&output.text, c.test_count_pattern.as_deref()).map_err(error)?;
        if code != 0 {
            let failures = if baseline {
                &mut report.baseline_failures
            } else {
                &mut report.failures
            };
            failures.insert(id.clone(), bounded_failure(&output.text));
        }
        if baseline {
            if code != 0 {
                return Err(error(format!("exit {code}")));
            }
        } else if code == 0 {
            report.green.push(id.clone());
        } else {
            report.red.push(id.clone());
            if let Some(reason) = wrong_message(c, id, &output.text)? {
                report.unexpected_red.insert(id.clone());
                report.outcome = Outcome::RedForAnotherReason;
                report.reason.get_or_insert(reason);
            }
        }
    }
    Ok(())
}

fn normalize_output(c: &Control, stdout: &str) -> Result<String> {
    let mut normalized = stdout.to_owned();
    for pattern in &c.output_normalize {
        normalized = regex::Regex::new(pattern)
            .map_err(|e| e.to_string())?
            .replace_all(&normalized, "")
            .into_owned();
    }
    Ok(normalized)
}

fn first_output_difference(left: &str, right: &str) -> String {
    let a: Vec<_> = left.split('\n').collect();
    let b: Vec<_> = right.split('\n').collect();
    let i = (0..a.len().max(b.len()))
        .find(|&i| a.get(i) != b.get(i))
        .unwrap_or(0);
    format!("line {}: {:?} != {:?}", i + 1, a.get(i), b.get(i))
}

fn output_tests(
    root: &Path,
    c: &Control,
    stop: &AtomicBool,
    report: &mut Report,
    baseline: bool,
) -> Result<()> {
    let argv = c.command.as_ref().ok_or("command is required")?;
    let mut first: Option<String> = None;
    for _ in 0..if baseline { 2 } else { 1 } {
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        let output = execute_output(root, cmd, c.timeout_s, stop, true)?;
        if baseline {
            report.baseline_test_ms += output.ms;
        } else {
            report.test_ms += output.ms;
        }
        report.test_tail = tail(&format!(
            "{}\n{} output:\n{}",
            report.test_tail,
            if baseline {
                "baseline"
            } else {
                "mutated command"
            },
            output.text
        ));
        if output.interrupted {
            return Err("interrupted".into());
        }
        if output.timeout {
            report.timed_out_phase = Some(Phase::Test);
            return Err(format!("test command exceeded timeout_s = {}", c.timeout_s));
        }
        let code = output.code.ok_or_else(|| {
            format!(
                "process died by {}",
                output.signal.as_deref().unwrap_or("unknown signal")
            )
        })?;
        if code == 126 || code == 127 {
            return Err(format!(
                "exit {code}: command not executable, refused to run, or not found"
            ));
        }
        if baseline && code != 0 {
            return Err(format!("baseline was not green: exit {code}"));
        }
        let normalized = normalize_output(c, &output.stdout)?;
        if baseline {
            if let Some(first) = &first {
                if first != &normalized {
                    return Err(format!(
                        "baseline output is not deterministic: {}",
                        first_output_difference(first, &normalized)
                    ));
                }
            } else {
                first = Some(normalized);
            }
        } else {
            let clean = report
                .baseline_stdout
                .as_ref()
                .ok_or("missing baseline stdout")?;
            report.outcome = if code != 0 || clean != &normalized {
                Outcome::Caught
            } else {
                Outcome::Survived
            };
        }
    }
    if baseline {
        report.baseline_stdout = first;
    }
    Ok(())
}

fn cargo_baseline(
    root: &Path,
    c: &Control,
    scope: Scope,
    stop: &AtomicBool,
    report: &mut Report,
) -> Result<()> {
    let build = execute(root, command(c, "build", scope)?, c.build_timeout_s, stop)?;
    report.baseline_build_ms += build.ms;
    report.build_tail = tail(&build.text);
    if build.timeout {
        report.timed_out_phase = Some(Phase::Build);
    }
    if build.interrupted || build.timeout || !build.success {
        return Err(format!("baseline build failed: {}", tail(&build.text)));
    }
    let tests = execute(root, command(c, "run", scope)?, c.timeout_s, stop)?;
    report.baseline_test_ms += tests.ms;
    report.test_tail = tail(&tests.text);
    if tests.timeout {
        report.timed_out_phase = Some(Phase::Test);
    }
    if tests.interrupted || tests.timeout || tests.code.is_none() {
        return Err(format!("baseline test run failed: {}", tail(&tests.text)));
    }
    let results = parse_test_results(&tests.text, &c.runner)?;
    report.baseline_failures = results.failures();
    if let Some((name, signal)) = results.signals.iter().next() {
        return Err(format!("baseline {name}: test binary died by {signal}"));
    }
    if results.red.is_empty() && !tests.success {
        return Err("baseline runner exited nonzero without a red test".into());
    }
    for name in &results.red {
        report
            .baseline_red
            .entry(stable_target(&c.runner, &results.targets[name]).to_owned())
            .or_default()
            .push(results.names[name].clone());
    }
    report.baseline_results = Some(results);
    Ok(())
}

fn validate_baseline(c: &Control, report: &mut Report) -> Result<()> {
    if let Some(results) = &report.baseline_results {
        let expected = resolve_expected(c, results)?;
        let red: Vec<_> = expected
            .iter()
            .filter(|name| results.red.contains(name))
            .cloned()
            .collect();
        if !red.is_empty() {
            return Err(format!("baseline red: {}", red.join(", ")));
        }
        if results.red.len() + results.green.len() == 0
            || expected.iter().any(|n| !results.green.contains(n))
        {
            report.outcome = Outcome::NoTestsRan;
        }
    }
    Ok(())
}

fn exclude_baseline_red(c: &Control, report: &Report, results: &mut TestResults) {
    results.red.retain(|name| {
        let target = stable_target(&c.runner, &results.targets[name]);
        !report
            .baseline_red
            .get(target)
            .is_some_and(|names| names.contains(&results.names[name]))
    });
}

pub fn parse_tests(text: &str, runner: &str) -> Result<(Vec<String>, Vec<String>)> {
    let results = parse_test_results(text, runner)?;
    Ok((results.red, results.green))
}

#[derive(Debug, Clone)]
struct TestResults {
    red: Vec<String>,
    green: Vec<String>,
    targets: BTreeMap<String, String>,
    names: BTreeMap<String, String>,
    failure_output: BTreeMap<String, String>,
    signals: BTreeMap<String, String>,
}

impl TestResults {
    fn failures(&self) -> BTreeMap<String, String> {
        self.red
            .iter()
            .map(|name| (name.clone(), bounded_failure(&self.failure_output[name])))
            .collect()
    }
}

// Keep binary identity until all output has been read. Two binaries can compile
// the same test source, and their independent outcomes must never be collapsed.
#[derive(Default)]
struct TestEvents(BTreeMap<(String, String), TestEvent>);

#[derive(Default)]
struct TestEvent {
    failed: bool,
    output: String,
    signal: Option<String>,
}

impl TestEvents {
    fn record(&mut self, target: &str, name: &str, failed: bool) -> Result<()> {
        let key = (target.to_owned(), name.to_owned());
        if self.0.get(&key).is_some_and(|old| old.failed != failed) {
            return Err(format!("conflicting test results: {target}::{name}"));
        }
        self.0.entry(key).or_default().failed = failed;
        Ok(())
    }

    fn finish(self, runner: &str) -> TestResults {
        let mut counts = BTreeMap::new();
        for (_, name) in self.0.keys() {
            *counts.entry(name.clone()).or_insert(0) += 1;
        }
        let mut results = TestResults {
            red: vec![],
            green: vec![],
            targets: BTreeMap::new(),
            names: BTreeMap::new(),
            failure_output: BTreeMap::new(),
            signals: BTreeMap::new(),
        };
        for ((target, name), event) in self.0 {
            let identity = if counts[&name] > 1 {
                format!("{}::{name}", stable_target(runner, &target))
            } else {
                name.clone()
            };
            results.targets.insert(identity.clone(), target);
            results.names.insert(identity.clone(), name);
            results
                .failure_output
                .insert(identity.clone(), event.output);
            if let Some(signal) = event.signal {
                results.signals.insert(identity.clone(), signal);
            }
            if event.failed {
                results.red.push(identity);
            } else {
                results.green.push(identity);
            }
        }
        results.red.sort();
        results.green.sort();
        results
    }
}

fn resolve_expected(c: &Control, results: &TestResults) -> Result<Vec<String>> {
    c.expect_red
        .iter()
        .map(|expected| {
            let candidates: Vec<_> = results
                .names
                .iter()
                .filter(|(_, name)| *name == expected)
                .map(|(identity, _)| identity.clone())
                .collect();
            if candidates.len() > 1 {
                return Err(format!(
                    "{}: ambiguous expect_red name {expected}; choose a qualified name: {}",
                    c.id,
                    candidates.join(", ")
                ));
            }
            if let Some(identity) = candidates.first() {
                return Ok(identity.clone());
            }
            // A qualified expectation is also valid when a scoped run observes only
            // that binary, in which case its report keeps the unique plain name.
            Ok(results
                .names
                .iter()
                .find(|(identity, name)| {
                    format!(
                        "{}::{name}",
                        stable_target(&c.runner, &results.targets[*identity])
                    ) == *expected
                })
                .map_or_else(|| expected.clone(), |(identity, _)| identity.clone()))
        })
        .collect()
}

fn cargo_binary_header(line: &str) -> Result<Option<String>> {
    if let Some(rest) = line.strip_prefix("Running ") {
        let (_, binary) = rest
            .rsplit_once(" (")
            .ok_or("cargo header missing binary")?;
        Ok(Some(
            binary
                .strip_suffix(')')
                .ok_or("invalid cargo binary header")?
                .rsplit(['/', '\\'])
                .next()
                .ok_or("empty cargo binary")?
                .to_owned(),
        ))
    } else {
        Ok(line
            .strip_prefix("Doc-tests ")
            .map(|target| format!("doc:{target}")))
    }
}

fn parse_test_results(text: &str, runner: &str) -> Result<TestResults> {
    if runner == "nextest" && text.lines().any(|l| l.starts_with("{\"type\":\"suite\"")) {
        return parse_nextest_json(text);
    }
    let mut events = TestEvents::default();
    let mut cargo_target = String::new();
    let mut cargo_started = BTreeSet::new();
    let mut cargo_complete = BTreeSet::new();
    let mut cargo_signaled = BTreeSet::new();
    let mut pending = None;
    let mut failure_block: Option<(String, String)> = None;
    let mut summary_total = 0;
    let mut cargo_counts = BTreeMap::<String, usize>::new();
    let mut observed_summary = false;
    let mut nextest_total = None;
    for line in text.lines() {
        let raw_line = line;
        let line = line.trim();
        if runner == "cargo" {
            if let Some(name) = line
                .strip_prefix("---- ")
                .and_then(|s| s.strip_suffix(" stdout ----"))
            {
                failure_block = Some((cargo_target.clone(), name.to_owned()));
                continue;
            }
            if line == "failures:" || line.starts_with("test result:") {
                failure_block = None;
            }
            if let Some(key) = &failure_block {
                let event = events
                    .0
                    .get_mut(key)
                    .ok_or("failure output without test result")?;
                event.output.push_str(raw_line);
                event.output.push('\n');
                continue;
            }
            if let Some(target) = cargo_binary_header(line)? {
                cargo_target = target;
                cargo_started.insert(cargo_target.clone());
                pending = None;
            } else if let Some(rest) = line.strip_prefix("test result:") {
                cargo_complete.insert(cargo_target.clone());
                observed_summary = true;
                for key in ["passed", "failed", "ignored", "measured", "filtered"] {
                    let words: Vec<_> = rest.split_whitespace().collect();
                    let pos = words
                        .iter()
                        .position(|w| w.trim_end_matches([';', '.']) == key)
                        .ok_or_else(|| format!("unrecognized libtest summary: {line}"))?;
                    let count: usize = pos
                        .checked_sub(1)
                        .and_then(|i| words[i].parse().ok())
                        .ok_or_else(|| format!("invalid libtest count: {line}"))?;
                    if key == "passed" || key == "failed" {
                        summary_total += count;
                        *cargo_counts.entry(cargo_target.clone()).or_default() += count;
                    }
                }
            } else if let Some(rest) = line.strip_prefix("test ") {
                if let Some(name) = rest.strip_suffix(" ...") {
                    pending = Some(name.to_owned());
                    continue;
                }
                if let Some((name, status)) = rest.rsplit_once(" ... ") {
                    pending = None;
                    match status {
                        "ok" => {
                            events.record(&cargo_target, name, false)?;
                        }
                        "FAILED" => {
                            events.record(&cargo_target, name, true)?;
                        }
                        status if status == "ignored" || status.starts_with("ignored,") => {}
                        status if status.is_empty() || status.starts_with("error:") => {
                            pending = Some(name.to_owned());
                        }
                        _ => return Err(format!("unrecognized test status: {line}")),
                    }
                }
            } else if line.starts_with("process didn't exit successfully:") {
                if let Some((_, signal)) = line.rsplit_once(" (signal: ") {
                    let signal = signal.trim_end_matches(')').to_owned();
                    let name = pending.take().ok_or_else(|| format!(
                        "{cargo_target}: test binary died by signal {signal} (no running test identified)"
                    ))?;
                    events.record(&cargo_target, &name, true)?;
                    events
                        .0
                        .get_mut(&(cargo_target.clone(), name))
                        .unwrap()
                        .signal = Some(format!("signal {signal}"));
                    cargo_signaled.insert(cargo_target.clone());
                }
            }
        } else {
            // Nextest's stable human status format includes a duration, binary, and
            // full test name. No command exit status is used to infer test results.
            let words: Vec<_> = line.split_whitespace().collect();
            if words.first() == Some(&"Summary") {
                let pos = words
                    .iter()
                    .position(|w| *w == "tests" || *w == "test")
                    .ok_or("unrecognized nextest summary")?;
                nextest_total = Some(
                    pos.checked_sub(1)
                        .and_then(|i| words[i].parse::<usize>().ok())
                        .ok_or("invalid nextest test count")?,
                );
            }
            if let Some((status, target, name)) = nextest_status(line) {
                events.record(target, name, status != "PASS")?;
                let key = (target.to_owned(), name.to_owned());
                if status.starts_with("SIG") {
                    events.0.get_mut(&key).unwrap().signal = Some(status.to_owned());
                }
                failure_block = (status != "PASS").then_some(key);
            } else if let Some(key) = &failure_block {
                // Older nextest emits failure output beneath the status, with
                // stdout/stderr or combined-output headings rather than JSON.
                if line.starts_with("Summary") || line.starts_with('─') {
                    failure_block = None;
                } else {
                    let event = events.0.get_mut(key).unwrap();
                    event.output.push_str(raw_line);
                    event.output.push('\n');
                }
            }
        }
    }
    if runner == "cargo" {
        for target in cargo_started.difference(&cargo_complete) {
            if !cargo_signaled.contains(target) {
                return Err(format!("{target}: libtest binary ended without a summary; running test: {} (signal unknown)", pending.as_deref().unwrap_or("<not reported>")));
            }
        }
        for (target, count) in cargo_counts {
            let observed = events
                .0
                .keys()
                .filter(|(binary, _)| *binary == target)
                .count();
            if count != observed {
                return Err(format!(
                    "{target}: libtest summary counts disagree with per-test output"
                ));
            }
        }
        let aborted_count = events
            .0
            .keys()
            .filter(|(target, _)| cargo_signaled.contains(target))
            .count();
        if (!observed_summary && cargo_signaled.is_empty())
            || summary_total + aborted_count != events.0.len()
        {
            return Err("libtest summary missing or counts disagree with per-test output".into());
        }
    }
    if runner == "nextest" && nextest_total != Some(events.0.len()) {
        if events.0.is_empty() && text.contains("no tests to run") {
            return Ok(events.finish(runner));
        }
        return Err("nextest summary missing or counts disagree with per-test output".into());
    }
    Ok(events.finish(runner))
}

fn nextest_status(line: &str) -> Option<(&str, &str, &str)> {
    let words: Vec<_> = line.split_whitespace().collect();
    let status = *words.first()?;
    if !matches!(status, "PASS" | "FAIL") && !status.starts_with("SIG") {
        return None;
    }
    if !words.get(1)?.starts_with('[') || words.len() < 4 {
        return None;
    }
    Some((status, words[words.len() - 2], words[words.len() - 1]))
}

fn stable_target<'a>(runner: &str, target: &'a str) -> &'a str {
    if runner != "cargo" || target.starts_with("doc:") {
        return target;
    }
    let binary = target.strip_suffix(".exe").unwrap_or(target);
    // Cargo's executable hash changes on rebuild. Catalogue approvals must use
    // the test target's stable name, including on Windows.
    match binary.rsplit_once('-') {
        Some((name, hash)) if hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()) => {
            name
        }
        _ => binary,
    }
}

fn collateral(c: &Control, results: &TestResults) -> Result<(Collateral, Vec<String>)> {
    // A catch must be attributable; names-only snippets remain supported by the
    // public parser, but cannot establish whether a real replay caught broadly.
    if results.targets.values().any(String::is_empty) {
        return Err("test output missing binary attribution".into());
    }
    let expected_targets: BTreeSet<_> = c
        .expect_red
        .iter()
        .filter_map(|name| results.targets.get(name))
        .map(|target| stable_target(&c.runner, target))
        .collect();
    let extra: Vec<_> = results
        .red
        .iter()
        .filter(|name| !c.expect_red.contains(name))
        .collect();
    let targets: BTreeSet<_> = extra
        .iter()
        .filter_map(|name| results.targets.get(*name))
        .map(|target| stable_target(&c.runner, target).to_owned())
        .collect();
    // The report includes all collateral, but breadth grading and HUB approval
    // concern only targets outside every expected test's own target.
    let cross_targets = targets
        .iter()
        .filter(|target| !expected_targets.contains(target.as_str()))
        .cloned()
        .collect();
    Ok((
        Collateral {
            count: extra.len(),
            targets: targets.into_iter().collect(),
        },
        cross_targets,
    ))
}

/// Name-only grading. Replay additionally verifies signals and per-test messages
/// against the original runner output before crediting a catch.
pub fn grade(c: &Control, red: &[String], green: &[String]) -> Outcome {
    if red.len() + green.len() == 0
        || c.expect_red
            .iter()
            .any(|n| !red.contains(n) && !green.contains(n))
    {
        return Outcome::NoTestsRan;
    }
    let extra = red.iter().any(|n| !c.expect_red.contains(n));
    if (c.only && extra) || (extra && c.expect_red.iter().any(|n| green.contains(n))) {
        return Outcome::WrongTest;
    }
    if c.expect_red.iter().any(|n| green.contains(n)) {
        return Outcome::Survived;
    }
    Outcome::Caught
}

/// Grade an `explore` run: nobody has named the guarding tests, so any red test
/// is a catch. Only parsed per-test results count, never the exit status.
pub fn explore_grade(red: &[String], green: &[String]) -> Outcome {
    if red.is_empty() && green.is_empty() {
        Outcome::NoTestsRan
    } else if red.is_empty() {
        Outcome::Survived
    } else {
        Outcome::Caught
    }
}

/// What a SURVIVED `explore` means. It is the survivor diagnosis `prove` gives
/// (scope omitted the guarding test, coverage gap, or equivalent mutant), worded
/// for a run that was already unscoped: explore runs every test in the package
/// or workspace by design, so "outside the scope" means outside that run.
pub fn survivor_diagnosis(package: &str, workspace: bool) -> String {
    let scope = if workspace {
        "the whole workspace".to_owned()
    } else {
        format!("every test in package `{package}`")
    };
    let elsewhere = if workspace {
        "in another repository or workspace, in an ignored test, or behind a feature or cfg this run did not enable"
    } else {
        "in another package (rerun with --workspace), in an ignored test, or behind a feature or cfg this run did not enable"
    };
    format!(
        "No test went red: this run covered {scope}, unscoped by design. Three causes remain:\n\
         1. The mutant is equivalent: it computes exactly the same result. Inspect semantics; use equivalent = <reason> only when justified.\n\
         2. The guarding test lives outside the scope run: {elsewhere}.\n\
         3. The guard is missing: a real coverage gap. Write the test, then prove it."
    )
}

/// Replay one catalogue row: with its target selector, or (`unscoped`) across
/// its whole package. A thin entry point to the shared `replay` path.
pub fn run_row(
    root: &Path,
    c: &Control,
    allow_dirty: bool,
    stop: &AtomicBool,
    unscoped: bool,
) -> Result<Report> {
    let scope = if unscoped { Scope::Package } else { Scope::Row };
    replay(
        root,
        c,
        allow_dirty,
        stop,
        scope,
        &[],
        ReplayStage::Mutant(None),
    )
}

/// Audit one catalogue row across every test target in its package, retaining
/// any explicitly selected targets. Only this opt-in replay grades broad catches.
pub fn run_broad_row(
    root: &Path,
    c: &Control,
    allow_dirty: bool,
    stop: &AtomicBool,
) -> Result<Report> {
    replay(
        root,
        c,
        allow_dirty,
        stop,
        Scope::Broad,
        &[],
        ReplayStage::Mutant(None),
    )
}

/// Apply one mutant and run every test in its package (or the workspace),
/// reporting which tests went red. `c.expect_red` and `c.target` are ignored.
/// A thin entry point to the same `replay` path `run_row` uses.
pub fn explore_row(
    root: &Path,
    c: &Control,
    allow_dirty: bool,
    stop: &AtomicBool,
    workspace: bool,
) -> Result<Report> {
    replay(
        root,
        c,
        allow_dirty,
        stop,
        Scope::Explore { workspace },
        &[],
        ReplayStage::Mutant(None),
    )
}

/// One CLI replay session. Prepare prerequisites, collect every clean baseline,
/// replay mutants, then finish by refreshing fixtures on the restored tree. A plain
/// library run_row has no catalogue prerequisites; catalogue callers use a session.
pub struct ReplaySession {
    prebuild: Vec<Prebuild>,
    baseline: Option<(u128, String)>,
    row_baselines: BTreeMap<String, Report>,
    validated_rows: BTreeSet<String>,
    clean_edits: Vec<Edit>,
    fixtures_dirty: bool,
}

impl ReplaySession {
    pub fn prepare(
        root: &Path,
        prebuild: &[Prebuild],
        rows: &[&Control],
        allow_dirty: bool,
        stop: &AtomicBool,
    ) -> Result<Self> {
        validate_prebuild(prebuild)?;
        let mut session = Self {
            prebuild: prebuild.to_vec(),
            baseline: None,
            row_baselines: BTreeMap::new(),
            validated_rows: BTreeSet::new(),
            clean_edits: vec![],
            fixtures_dirty: false,
        };
        let active: Vec<_> = rows
            .iter()
            .copied()
            .filter(|c| c.matches_platform() && c.unreachable.is_none() && c.desk_only.is_none())
            .collect();
        if prebuild.is_empty() || active.is_empty() {
            return Ok(session);
        }
        let edits = active
            .iter()
            .map(|c| c.edits())
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .filter(|e| root.join(&e.file).exists())
            .collect::<Vec<_>>();
        session.clean_edits = edits;
        session.baseline = Some(
            clean_prebuild(root, prebuild, &session.clean_edits, allow_dirty, stop)
                .map_err(|e| format!("unmutated {e}; replay aborted before grading rows"))?,
        );
        Ok(session)
    }

    /// Collect baselines before the first mutant, sharing Cargo/nextest runs
    /// across rows with the same runner, package, target and feature selection.
    pub fn baselines(
        &mut self,
        root: &Path,
        rows: &[&Control],
        allow_dirty: bool,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.baselines_scoped(root, rows, allow_dirty, stop, Scope::Row)
    }

    pub fn broad_baselines(
        &mut self,
        root: &Path,
        rows: &[&Control],
        allow_dirty: bool,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.baselines_scoped(root, rows, allow_dirty, stop, Scope::Broad)
    }

    /// Prove may diagnose a survivor with a package replay, so prepare that
    /// wider baseline while fixtures are still clean, not after a mutant.
    pub fn package_baselines(
        &mut self,
        root: &Path,
        rows: &[&Control],
        allow_dirty: bool,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.baselines_scoped(root, rows, allow_dirty, stop, Scope::Package)
    }

    fn baselines_scoped(
        &mut self,
        root: &Path,
        rows: &[&Control],
        allow_dirty: bool,
        stop: &AtomicBool,
        scope: Scope,
    ) -> Result<()> {
        if self.fixtures_dirty {
            return Err("baselines must be collected before replaying mutants".into());
        }
        let mut shared: BTreeMap<Vec<String>, Report> = BTreeMap::new();
        let mut listings = BTreeMap::new();
        for c in rows {
            let active = c.matches_platform() && c.unreachable.is_none() && c.desk_only.is_none();
            let key = if active && c.runner != "command" {
                command(c, "build", scope)?
                    .get_args()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect()
            } else {
                vec![c.id.clone()]
            };
            let mut report = if let Some(report) = shared.get(&key) {
                let mut report = report.clone();
                report.id = c.id.clone();
                report.baseline_build_ms = 0;
                report.baseline_test_ms = 0;
                report
            } else {
                let mut selection = (*c).clone();
                // Baseline data is shared, but each row's expected reds are
                // validated independently before its mutant can be applied.
                if c.runner != "command" {
                    selection.expect_red.clear();
                }
                let report = replay(
                    root,
                    &selection,
                    allow_dirty,
                    stop,
                    scope,
                    &[],
                    ReplayStage::Baseline,
                )?;
                if report.outcome != Outcome::AnchorMissing {
                    shared.insert(key.clone(), report.clone());
                }
                report
            };
            if active && report.outcome == Outcome::Survived && c.runner != "command" {
                // Resolve names in exactly the selection that the baseline ran.
                // Package-wide listing can compile unrelated, feature-gated targets.
                let names = listings.entry(key).or_insert_with(|| {
                    let start = Instant::now();
                    let names = list_results(root, c, scope, stop);
                    report.baseline_build_ms += start.elapsed().as_millis();
                    names
                });
                let validation = names
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|names| resolve_expected(c, names).map(|_| ()))
                    .and_then(|()| validate_baseline(c, &mut report));
                if let Err(e) = validation {
                    report.outcome = Outcome::Error;
                    report.reason = Some(e);
                }
            }
            self.row_baselines.insert(c.id.clone(), report);
            if stop.load(Ordering::SeqCst) {
                break;
            }
        }
        Ok(())
    }

    /// Restore fixture outputs once after all mutants, even when Ctrl-C stopped
    /// a mutant. Cleanup has its own deadlines and must ignore the latched stop.
    pub fn finish(&mut self, root: &Path, rows: &mut [Report], allow_dirty: bool) -> Result<()> {
        if !self.fixtures_dirty {
            return Ok(());
        }
        eprintln!("final restore_prebuild");
        let (ms, text) = clean_prebuild(
            root,
            &self.prebuild,
            &self.clean_edits,
            allow_dirty,
            &AtomicBool::new(false),
        )
        .map_err(|e| format!("final restore {e}; tree fixtures are suspect"))?;
        self.fixtures_dirty = false;
        if let Some(row) = rows.last_mut() {
            row.restore_prebuild_ms = ms;
            row.restore_prebuild_tail = text;
        }
        Ok(())
    }

    pub fn run_row(
        &mut self,
        root: &Path,
        c: &Control,
        allow_dirty: bool,
        stop: &AtomicBool,
        unscoped: bool,
    ) -> Result<Report> {
        self.replay(
            root,
            c,
            allow_dirty,
            stop,
            if unscoped { Scope::Package } else { Scope::Row },
        )
    }

    pub fn run_broad_row(
        &mut self,
        root: &Path,
        c: &Control,
        allow_dirty: bool,
        stop: &AtomicBool,
    ) -> Result<Report> {
        self.replay(root, c, allow_dirty, stop, Scope::Broad)
    }

    pub fn explore_row(
        &mut self,
        root: &Path,
        c: &Control,
        allow_dirty: bool,
        stop: &AtomicBool,
        workspace: bool,
    ) -> Result<Report> {
        self.replay(root, c, allow_dirty, stop, Scope::Explore { workspace })
    }

    fn replay(
        &mut self,
        root: &Path,
        c: &Control,
        allow_dirty: bool,
        stop: &AtomicBool,
        scope: Scope,
    ) -> Result<Report> {
        let active = c.matches_platform() && c.unreachable.is_none() && c.desk_only.is_none();
        let baseline = self.row_baselines.get(&c.id).cloned().map(|mut report| {
            if !self.validated_rows.insert(c.id.clone()) {
                report.baseline_build_ms = 0;
                report.baseline_test_ms = 0;
            }
            report
        });
        // A row's clean-tree baseline must not see fixture binaries built from an
        // earlier row's mutant, so rebuild them first when no session-wide
        // baseline has done so. That includes `explore` rows, which have no
        // expected tests yet but still run the baseline.
        if active && baseline.is_none() && self.fixtures_dirty {
            self.finish(root, &mut [], allow_dirty)?;
        }
        let mut row = replay(
            root,
            c,
            allow_dirty,
            stop,
            scope,
            &self.prebuild,
            ReplayStage::Mutant(baseline.map(Box::new)),
        )?;
        self.fixtures_dirty |= row.restore_prebuild_needed;
        if active {
            if let Some((ms, text)) = self.baseline.take() {
                row.baseline_prebuild_ms = ms;
                row.baseline_prebuild_tail = text;
            }
        }
        Ok(row)
    }
}

// Prerequisites may produce fixtures, never rewrite edit targets or the lockfile.
// Apply the same byte checks to both the initial and final clean-tree build.
fn clean_prebuild(
    root: &Path,
    prebuild: &[Prebuild],
    edits: &[Edit],
    allow_dirty: bool,
    stop: &AtomicBool,
) -> Result<(u128, String)> {
    let mut saved = Saved::new(root, edits, allow_dirty)?;
    let mut ms = 0;
    let mut text = String::new();
    let work = run_prebuild(root, prebuild, stop, &mut ms, &mut text, &mut None);
    // Prerequisites may produce fixtures, never rewrite edit targets or the
    // lockfile. Restore on failure, including a failing command's side effects.
    let unchanged = saved.unchanged()?;
    if unchanged {
        saved.active = false;
    } else {
        let restoration = saved.restore();
        return Err(format!(
            "prebuild changed source or Cargo.lock ({:?}); {}; {}",
            prebuild.iter().map(|step| &step.name).collect::<Vec<_>>(),
            work.err().unwrap_or_default(),
            restoration.err().unwrap_or_default()
        ));
    }
    work?;
    Ok((ms, tail(&text)))
}

fn run_prebuild(
    root: &Path,
    steps: &[Prebuild],
    stop: &AtomicBool,
    ms: &mut u128,
    text: &mut String,
    phase: &mut Option<Phase>,
) -> Result<()> {
    for step in steps {
        let label = format!("prebuild {:?}", step.name);
        if stop.load(Ordering::SeqCst) {
            return Err(format!("{label}: interrupted"));
        }
        let mut cmd = Command::new(&step.command[0]);
        cmd.args(&step.command[1..]);
        let output =
            execute(root, cmd, step.timeout_s, stop).map_err(|e| format!("{label}: {e}"))?;
        *ms += output.ms;
        text.push_str(&format!("{label} ({} ms):\n{}\n", output.ms, output.text));
        eprintln!("{label} ({} ms):\n{}", output.ms, output.text);
        if output.timeout {
            *phase = Some(Phase::Prebuild);
            return Err(format!("{label}: exceeded timeout_s = {}", step.timeout_s));
        }
        if !output.success || output.interrupted {
            return Err(format!(
                "{label}: failed (exit {:?}, interrupted {})",
                output.code, output.interrupted
            ));
        }
    }
    Ok(())
}

enum ReplayStage {
    Baseline,
    Mutant(Option<Box<Report>>),
}

// The only code that mutates source: dirty-target refusal, exact-once anchors,
// the separate build, the test run, per-test parsing and byte restoration of
// targets and Cargo.lock all live here, for every command.
fn replay(
    root: &Path,
    c: &Control,
    allow_dirty: bool,
    stop: &AtomicBool,
    scope: Scope,
    prebuild: &[Prebuild],
    stage: ReplayStage,
) -> Result<Report> {
    let (mut report, baseline_only, baseline_ready) = match stage {
        ReplayStage::Baseline => (Report::new(c), true, false),
        ReplayStage::Mutant(Some(report)) => {
            if report.outcome != Outcome::Survived {
                return Ok(*report);
            }
            (*report, false, true)
        }
        ReplayStage::Mutant(None) => (Report::new(c), false, false),
    };
    c.validate_runner()?;
    let disposition = c.recorded_disposition()?;
    if let Some((Outcome::DeskOnly, reason)) = disposition {
        report.outcome = Outcome::DeskOnly;
        report.reason = Some(reason.to_owned());
        return Ok(report);
    }
    if !c.matches_platform() {
        report.outcome = Outcome::SkippedPlatform;
        report.reason = Some(format!(
            "host target_os = {}; platforms = {:?}",
            std::env::consts::OS,
            c.platforms.as_ref().unwrap()
        ));
        return Ok(report);
    }
    if c.runner == "command" && matches!(scope, Scope::Explore { .. }) {
        return Err(format!(
            "{}: explore refuses command rows: only expect_red ids can be observed",
            c.id
        ));
    }
    c.validate_runner()?;
    if let Some((outcome, reason)) = c.recorded_disposition()? {
        report.outcome = outcome;
        report.reason = Some(reason.to_owned());
        return Ok(report);
    }
    let edits = c.edits()?;
    let mut saved = match Saved::new(root, &edits, allow_dirty) {
        Ok(saved) => saved,
        Err(e) if e.starts_with("ANCHOR_MISSING ") => {
            report.outcome = Outcome::AnchorMissing;
            report.reason = Some(e);
            return Ok(report);
        }
        Err(e) => return Err(e),
    };
    let work = (|| -> Result<()> {
        let files = match replaced(root, &edits) {
            Ok(f) => f,
            Err(e) if e.starts_with("ANCHOR_MISSING") => {
                report.outcome = Outcome::AnchorMissing;
                report.reason = Some(e);
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        if stop.load(Ordering::SeqCst) {
            return Err("interrupted".into());
        }
        if !baseline_ready {
            if c.runner == "command" {
                command_tests(root, c, stop, &mut report, true)?;
            } else {
                if !c.expect_red.is_empty() && !matches!(scope, Scope::Explore { .. }) {
                    let start = Instant::now();
                    let names = list_results(root, c, scope, stop)?;
                    report.baseline_build_ms += start.elapsed().as_millis();
                    resolve_expected(c, &names)?;
                }
                cargo_baseline(root, c, scope, stop, &mut report)?;
                let mut baseline_control = c.clone();
                if matches!(scope, Scope::Explore { .. }) {
                    baseline_control.expect_red.clear();
                }
                validate_baseline(&baseline_control, &mut report)?;
                if report.outcome == Outcome::NoTestsRan {
                    return Ok(());
                }
            }
        }
        if baseline_only {
            report.outcome = Outcome::Survived;
            return Ok(());
        }
        // Even an interrupted or failed build may have rewritten fixtures before
        // the declared mutant prebuild starts, so final cleanup is already owed.
        report.restore_prebuild_needed = !prebuild.is_empty();
        for (path, text) in files {
            fs::write(path, text).map_err(|e| e.to_string())?;
        }
        if c.runner == "command" {
            run_prebuild(
                root,
                prebuild,
                stop,
                &mut report.prebuild_ms,
                &mut report.prebuild_tail,
                &mut report.timed_out_phase,
            )?;
            command_tests(root, c, stop, &mut report, false)?;
            if c.catch_on.is_none() && report.outcome != Outcome::RedForAnotherReason {
                report.outcome = grade(c, &report.red, &report.green);
            }
            return Ok(());
        }
        let build = execute(root, command(c, "build", scope)?, c.build_timeout_s, stop)?;
        report.build_ms += build.ms;
        report.build_tail = tail(&build.text);
        if build.interrupted {
            return Err("interrupted".into());
        }
        if build.timeout {
            report.outcome = Outcome::TimedOut;
            report.timed_out_phase = Some(Phase::Build);
            report.reason = Some(format!(
                "build exceeded build_timeout_s = {}",
                c.build_timeout_s
            ));
            return Ok(());
        }
        if !build.success {
            report.outcome = Outcome::DidNotCompile;
            return Ok(());
        }
        run_prebuild(
            root,
            prebuild,
            stop,
            &mut report.prebuild_ms,
            &mut report.prebuild_tail,
            &mut report.timed_out_phase,
        )?;
        let tests = execute(root, command(c, "run", scope)?, c.timeout_s, stop)?;
        report.test_ms += tests.ms;
        report.test_tail = tail(&tests.text);
        if tests.interrupted {
            return Err("interrupted".into());
        }
        if tests.timeout {
            report.outcome = Outcome::TimedOut;
            report.timed_out_phase = Some(Phase::Test);
            report.reason = Some(format!("test run exceeded timeout_s = {}", c.timeout_s));
            return Ok(());
        }
        if tests.code.is_none() {
            return Err(format!(
                "test runner died by {}; expected tests: {}",
                tests.signal.as_deref().unwrap_or("unknown signal"),
                c.expect_red.join(", ")
            ));
        }
        let mut results = parse_test_results(&tests.text, &c.runner)?;
        report.failures = results.failures();
        // Match baseline-red tests by (test binary, plain test name). A `--broad`
        // run prefixes a name with its binary when two binaries share it, so
        // the printed name can differ from the one recorded at baseline.
        exclude_baseline_red(c, &report, &mut results);
        if c.signal_is_catch.is_none() {
            if let Some((name, signal)) = results.signals.iter().next() {
                return Err(format!("{name}: test binary died by {signal}"));
            }
        }
        report.outcome = match scope {
            // Explore has no expected tests: every red name becomes expect_red
            // when recorded, so none is collateral to that discovery.
            Scope::Explore { .. } => explore_grade(&results.red, &results.green),
            Scope::Row | Scope::Package | Scope::Broad => {
                let mut grading = c.clone();
                grading.expect_red = resolve_expected(c, &results)?;
                report.unexpected_red.extend(
                    results
                        .red
                        .iter()
                        .filter(|name| !grading.expect_red.contains(name))
                        .cloned(),
                );
                let (extra, cross_targets) = collateral(&grading, &results)?;
                report.collateral = extra;
                report.breadth_observed = scope == Scope::Broad;
                let mut outcome = grade(&grading, &results.red, &results.green);
                for name in &grading.expect_red {
                    if results.red.contains(name) {
                        if let Some(reason) = wrong_message(c, name, &results.failure_output[name])?
                        {
                            outcome = Outcome::RedForAnotherReason;
                            report.unexpected_red.insert(name.clone());
                            report.reason.get_or_insert(reason);
                        }
                    }
                }
                if outcome == Outcome::Caught && report.breadth_observed {
                    if let Some(reason) = &c.hub {
                        let approved = c.hub_targets.as_deref().unwrap_or_default();
                        let new_targets: Vec<_> = cross_targets
                            .iter()
                            .filter(|target| !approved.contains(target))
                            .cloned()
                            .collect();
                        // A reviewed hub permits only the recorded cross-target
                        // set. Same-target failures never require HUB approval;
                        // fewer other targets are fine, new ones need review.
                        if new_targets.is_empty() {
                            report.reason = Some(reason.clone());
                            Outcome::Hub
                        } else {
                            report.reason = Some(format!(
                                "HUB collateral outside hub_targets: {}",
                                new_targets.join(", ")
                            ));
                            Outcome::CaughtBroadly
                        }
                    } else if !cross_targets.is_empty() {
                        Outcome::CaughtBroadly
                    } else {
                        outcome
                    }
                } else {
                    outcome
                }
            }
        };
        report.red = results.red;
        report.green = results.green;
        Ok(())
    })();
    if !baseline_only
        && c.equivalent.is_some()
        && (report.outcome == Outcome::Survived
            || report.outcome == Outcome::Caught
            || !report.red.is_empty())
    {
        report.outcome = if !report.red.is_empty()
            || (c.catch_on.is_some() && report.outcome == Outcome::Caught)
        {
            Outcome::EquivalentCaught
        } else {
            Outcome::Equivalent
        };
        report.reason = Some(format!(
            "{}; guard: {}",
            c.equivalent.as_deref().unwrap(),
            c.equivalent_guard.as_deref().unwrap()
        ));
    }
    let restoration = if baseline_only && saved.unchanged()? {
        saved.active = false;
        Ok(())
    } else {
        saved.restore()
    };
    if let Err(e) = work {
        report.outcome = Outcome::Error;
        report.reason = Some(e);
    }
    if let Err(e) = restoration {
        report.outcome = Outcome::Error;
        report.timed_out_phase = None;
        report.reason = Some(e);
    }
    report.prebuild_tail = tail(&report.prebuild_tail);
    Ok(report)
}

fn list_results(root: &Path, c: &Control, scope: Scope, stop: &AtomicBool) -> Result<TestResults> {
    // List mode compiles the test binaries, so the build deadline bounds it.
    let output = execute(root, command(c, "list", scope)?, c.build_timeout_s, stop)?;
    if !output.success || output.timeout || output.interrupted {
        return Err(format!("{}: list failed: {}", c.id, tail(&output.text)));
    }
    let mut events = TestEvents::default();
    if c.runner == "cargo" {
        let mut target = String::new();
        for line in output.text.lines().map(str::trim) {
            if let Some(binary) = cargo_binary_header(line)? {
                target = binary;
            } else if let Some(name) = line.strip_suffix(": test") {
                events.record(&target, name, false)?;
            }
        }
    } else {
        let json: serde_json::Value = serde_json::from_str(
            output
                .text
                .lines()
                .find(|l| l.starts_with('{'))
                .ok_or("missing nextest list JSON")?,
        )
        .map_err(|e| e.to_string())?;
        let suites = json
            .get("rust-suites")
            .and_then(|v| v.as_object())
            .ok_or("missing nextest rust-suites")?;
        for (target, suite) in suites {
            let cases = suite
                .get("testcases")
                .and_then(|v| v.as_object())
                .ok_or("missing nextest testcases")?;
            for name in cases.keys() {
                events.record(target, name, false)?;
            }
        }
    }
    Ok(events.finish(&c.runner))
}

pub fn check(root: &Path, catalogue: &Catalogue, stop: &AtomicBool) -> Result<()> {
    validate(root, catalogue)?;
    let rows: Vec<_> = catalogue.control.iter().collect();
    let _session = ReplaySession::prepare(root, &catalogue.prebuild, &rows, false, stop)?;
    for c in &catalogue.control {
        replaced(root, &c.edits()?)?;
        // Unreachable rows have no names to discover. Command rows have no list
        // protocol: replay verifies their expected ids with a fresh baseline.
        if !c.matches_platform()
            || c.unreachable.is_some()
            || c.desk_only.is_some()
            || c.runner == "command"
        {
            continue;
        }
        let selected = list_results(root, c, Scope::Row, stop)?;
        for expected in resolve_expected(c, &selected)? {
            if !selected.names.contains_key(&expected) {
                return Err(format!(
                    "{}: expect_red name no longer exists: {expected}",
                    c.id
                ));
            }
        }
    }
    Ok(())
}

pub fn select_diff(
    root: &Path,
    catalogue_path: &str,
    catalogue: &Catalogue,
    base: &str,
) -> Result<BTreeSet<String>> {
    let changed = git(root, &["diff", "--name-only", "-z", base, "HEAD", "--"])?;
    let paths: BTreeSet<_> = changed
        .split(|b| *b == 0)
        .filter_map(|b| std::str::from_utf8(b).ok())
        .collect();
    let old: Catalogue = match git(root, &["show", &format!("{base}:{catalogue_path}")]) {
        Ok(b) => toml::from_str(std::str::from_utf8(&b).map_err(|e| e.to_string())?)
            .map_err(|e| format!("base catalogue is invalid: {e}"))?,
        Err(_) if paths.contains(catalogue_path) => Catalogue::default(),
        Err(_) => return Err("base catalogue missing and catalogue not added in diff".into()),
    };
    let mut selected = BTreeSet::new();
    for c in &catalogue.control {
        if paths.contains(c.test_file.as_str())
            || old.prebuild != catalogue.prebuild
            || c.edits()?.iter().any(|e| paths.contains(e.file.as_str()))
            || !old.control.iter().any(|o| o == c)
        {
            selected.insert(c.id.clone());
        }
    }
    Ok(selected)
}

/// The order `run` executes a shard's rows in, as indices into `rows`: grouped
/// by package, then by the first edited file, keeping the given order within a
/// group. Building the mutant dominates a row's time. When rows alternate
/// between packages, every row on a package that depends on another one
/// recompiles that dependency, because the previous row restored (rewrote) its
/// source; grouped, the dependency is recompiled once per group instead. The
/// order is internal: `run` still reports rows in their sorted-ID order.
pub fn execution_order(rows: &[&Control]) -> Vec<usize> {
    let first_file = |c: &Control| {
        c.file
            .clone()
            .or_else(|| c.edits.first().map(|e| e.file.clone()))
            .unwrap_or_default()
    };
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_cached_key(|&i| (rows[i].package.clone(), first_file(rows[i]), i));
    order
}

/// One token of the small Rust lexer behind the `prove` hint.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok<'a> {
    Ident(&'a str),
    Punct(u8),
}

/// Skip a string body starting just after its opening quote; returns the index
/// just past the closing quote (or the end of the text if it never closes).
fn skip_string(b: &[u8], mut i: usize) -> usize {
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// Identifiers and punctuation with their byte offsets. Comments and string,
/// raw-string and char literals are skipped so that braces and parentheses
/// inside them never count. This is not a full Rust lexer: it only has to be
/// good enough to find enclosing function bodies and call sites in source that
/// already compiles.
fn tokens(text: &str) -> Vec<(usize, Tok<'_>)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0usize;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c == b'"' {
            i = skip_string(b, i + 1);
        } else if c == b'\'' {
            // A char literal ('x', '\n', '\'') or a lifetime ('a).
            if b.get(i + 1) == Some(&b'\\') {
                i += 3;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            } else {
                let len = text[i + 1..].chars().next().map_or(0, char::len_utf8);
                i += if len > 0 && b.get(i + 1 + len) == Some(&b'\'') {
                    2 + len
                } else {
                    1
                };
            }
        } else if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
                i += 1;
            }
            let word = &text[start..i];
            let mut hashes = 0;
            while matches!(word, "r" | "br" | "cr") && b.get(i + hashes) == Some(&b'#') {
                hashes += 1;
            }
            if matches!(word, "r" | "br" | "cr") && b.get(i + hashes) == Some(&b'"') {
                let mut j = i + hashes + 1;
                i = b.len();
                while j < b.len() {
                    if b[j] == b'"'
                        && b.len() > j + hashes
                        && b[j + 1..=j + hashes].iter().all(|&x| x == b'#')
                    {
                        i = j + 1 + hashes;
                        break;
                    }
                    j += 1;
                }
            } else if matches!(word, "b" | "c") && b.get(i) == Some(&b'"') {
                i = skip_string(b, i + 1);
            } else {
                out.push((start, Tok::Ident(word)));
            }
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
        } else {
            if !c.is_ascii_whitespace() {
                out.push((i, Tok::Punct(c)));
            }
            i += 1;
        }
    }
    out
}

/// The innermost function whose body is open at byte `pos` of `text`.
fn enclosing_fn(text: &str, pos: usize) -> Option<&str> {
    // One entry per open brace: the function name when it opened a body.
    let mut open: Vec<Option<&str>> = Vec::new();
    // A `fn NAME` seen but whose body has not opened yet, and the parenthesis
    // and bracket depth inside its signature (`;` there is not a declaration end).
    let mut pending: Option<&str> = None;
    let mut nesting = 0usize;
    let toks = tokens(text);
    let mut k = 0;
    while k < toks.len() && toks[k].0 < pos {
        match toks[k].1 {
            Tok::Ident("fn") => {
                if let Some((_, Tok::Ident(name))) = toks.get(k + 1) {
                    pending = Some(name);
                    nesting = 0;
                    k += 1;
                }
            }
            Tok::Punct(b'(' | b'[') => nesting += 1,
            Tok::Punct(b')' | b']') => nesting = nesting.saturating_sub(1),
            Tok::Punct(b';') if nesting == 0 => pending = None,
            Tok::Punct(b'{') => open.push(if nesting == 0 { pending.take() } else { None }),
            Tok::Punct(b'}') => {
                open.pop();
            }
            _ => {}
        }
        k += 1;
    }
    open.into_iter().rev().flatten().next()
}

/// The first function the anchor text itself defines (`fn NAME`), when the row
/// rewrites a whole function rather than a line inside one.
fn defined_fn(text: &str) -> Option<&str> {
    tokens(text)
        .windows(2)
        .find_map(|w| match (w[0].1, w[1].1) {
            (Tok::Ident("fn"), Tok::Ident(name)) => Some(name),
            _ => None,
        })
}

/// Function and method calls in `text`, counted by callee name: an identifier
/// followed by `(`, directly or through a turbofish. Macros are not calls;
/// capitalised names (`Some(..)`, `Err(..)`, `Point(..)`) are constructors.
fn calls(text: &str) -> BTreeMap<&str, usize> {
    const KEYWORDS: &[&str] = &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "fn", "for",
        "if", "impl", "in", "let", "loop", "match", "move", "mut", "pub", "ref", "return", "self",
        "static", "super", "unsafe", "where", "while", "yield",
    ];
    let toks = tokens(text);
    let mut found = BTreeMap::new();
    for (k, &(_, tok)) in toks.iter().enumerate() {
        let Tok::Ident(name) = tok else { continue };
        if KEYWORDS.contains(&name)
            || name.starts_with(|c: char| c.is_ascii_uppercase())
            || (k > 0 && toks[k - 1].1 == Tok::Ident("fn"))
        {
            continue;
        }
        let mut next = k + 1;
        let turbofish = [Tok::Punct(b':'), Tok::Punct(b':'), Tok::Punct(b'<')];
        if toks
            .get(next..next + 3)
            .is_some_and(|t| t.iter().map(|(_, tok)| *tok).eq(turbofish.iter().copied()))
        {
            let mut depth = 0usize;
            next += 2;
            while let Some((_, tok)) = toks.get(next) {
                next += 1;
                match tok {
                    Tok::Punct(b'<') => depth += 1,
                    Tok::Punct(b'>') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        if toks.get(next).map(|(_, tok)| *tok) == Some(Tok::Punct(b'(')) {
            *found.entry(name).or_insert(0) += 1;
        }
    }
    found
}

/// Whether an edit removes a call site: some call in `old` loses an occurrence
/// in `new` and no call is added in its place. Swapping one call for another
/// (`to_ascii_uppercase()` for `to_ascii_lowercase()`) is a body mutation.
fn removes_call(edit: &Edit) -> bool {
    let old = calls(&edit.old);
    let new = calls(&edit.new);
    let fewer = |a: &BTreeMap<&str, usize>, b: &BTreeMap<&str, usize>| {
        a.iter()
            .any(|(name, n)| b.get(name).copied().unwrap_or(0) < *n)
    };
    fewer(&old, &new) && !fewer(&new, &old)
}

/// What `prove` suggests after a CAUGHT row, given the unmutated text of the
/// edited file. A mutation inside a function body proves the function's tests
/// notice it, not that any caller still reaches the function, so the hint names
/// that function and asks for a row removing a call to it. A row that already
/// removes a call site, or that mutates something outside every function body
/// (a constant, a type, an attribute), gets no hint: there is no call site of
/// it to remove.
pub fn call_site_hint(file_text: &str, edit: &Edit) -> Option<String> {
    if removes_call(edit) {
        return None;
    }
    let pos = file_text.find(&edit.old)?;
    let name = defined_fn(&edit.old).or_else(|| enclosing_fn(file_text, pos))?;
    // Nothing in the program calls `main`; there is no call site to remove.
    if name == "main" {
        return None;
    }
    Some(format!(
        "This row mutates the body of `{name}`, which proves nothing about callers reaching it. Add a second row removing a call to `{name}`."
    ))
}

pub fn signal_flag() -> Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    let handler = flag.clone();
    ctrlc::set_handler(move || {
        handler.store(true, Ordering::SeqCst);
    })
    .map_err(|e| e.to_string())?;
    Ok(flag)
}

fn parse_nextest_json(text: &str) -> Result<TestResults> {
    let mut events = TestEvents::default();
    let mut total = 0u64;
    let mut summaries = 0;
    for line in text.lines().filter(|l| l.starts_with('{')) {
        let event: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let kind = event["type"].as_str().ok_or("nextest event missing type")?;
        let status = event["event"]
            .as_str()
            .ok_or("nextest event missing event")?;
        if kind == "test" && matches!(status, "ok" | "failed") {
            let qualified = event["name"].as_str().ok_or("nextest event missing name")?;
            // Nextest prefixes libtest names with crate::binary$; the suffix is
            // the exact libtest path used by catalogue rows and list mode.
            let (target, name) = qualified
                .split_once('$')
                .ok_or("nextest name missing binary prefix")?;
            if events.0.contains_key(&(target.to_owned(), name.to_owned())) {
                return Err(format!("repeated test name: {target}::{name}"));
            }
            events.record(target, name, status == "failed")?;
            let output = &mut events
                .0
                .get_mut(&(target.to_owned(), name.to_owned()))
                .unwrap()
                .output;
            output.push_str(event["stdout"].as_str().unwrap_or_default());
            if let Some(stderr) = event["stderr"].as_str().filter(|text| !text.is_empty()) {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(stderr);
            }
        } else if kind == "suite" && matches!(status, "ok" | "failed") {
            summaries += 1;
            total += event["passed"]
                .as_u64()
                .ok_or("nextest summary missing passed")?
                + event["failed"]
                    .as_u64()
                    .ok_or("nextest summary missing failed")?;
        } else if !matches!(status, "started" | "ignored") {
            return Err(format!("unrecognized nextest event: {line}"));
        }
    }
    if summaries == 0 || total as usize != events.0.len() {
        return Err("nextest summary missing or inconsistent".into());
    }
    // Current nextest libtest-json marks an abort as just `failed` with empty
    // stdout. Its simultaneous human status is the authoritative signal name.
    // Match both binary and test, never a signal-looking word in panic output.
    for (signal, human_target, name) in text
        .lines()
        .filter_map(|line| nextest_status(line.trim()))
        .filter(|(status, _, _)| status.starts_with("SIG"))
    {
        let keys: Vec<_> = events
            .0
            .keys()
            .filter(|(target, test)| {
                test == name
                    && (target == human_target
                        || target.split_once("::").is_some_and(|(package, binary)| {
                            package == human_target && binary == package.replace('-', "_")
                        }))
            })
            .cloned()
            .collect();
        if keys.len() != 1 {
            return Err(format!(
                "{human_target}::{name}: {signal} status missing unambiguous JSON test attribution"
            ));
        }
        let event = events.0.get_mut(&keys[0]).unwrap();
        if !event.failed {
            return Err(format!(
                "{human_target}::{name}: {signal} disagrees with green JSON result"
            ));
        }
        event.signal = Some(signal.to_owned());
    }
    Ok(events.finish("nextest"))
}

#[cfg(test)]
mod broad_command_tests {
    use super::*;

    #[test]
    fn feature_flags_reach_every_runner_mode_and_scope_before_the_harness() {
        let mut c =
            toml::from_str::<Catalogue>(include_str!("../tests/fixture/mutations-v0.1.toml"))
                .unwrap()
                .control
                .remove(0);
        for runner in ["cargo", "nextest"] {
            c.runner = runner.into();
            for scope in [
                Scope::Row,
                Scope::Broad,
                Scope::Package,
                Scope::Explore { workspace: false },
                Scope::Explore { workspace: true },
            ] {
                for mode in ["build", "list", "run"] {
                    for all_features in [false, true] {
                        c.features =
                            (!all_features).then(|| vec!["test-support".into(), "dep/seam".into()]);
                        c.no_default_features = (!all_features).then_some(true);
                        c.all_features = all_features.then_some(true);
                        let cmd = command(&c, mode, scope).unwrap();
                        let args: Vec<_> = cmd.get_args().map(|s| s.to_str().unwrap()).collect();
                        let cargo_args =
                            &args[..args.iter().position(|a| *a == "--").unwrap_or(args.len())];
                        if all_features {
                            assert!(cargo_args.contains(&"--all-features"), "{args:?}");
                            assert!(!args.contains(&"--features"));
                            assert!(!args.contains(&"--no-default-features"));
                        } else {
                            assert!(
                                cargo_args
                                    .windows(2)
                                    .any(|w| w == ["--features", "test-support,dep/seam"]),
                                "{args:?}"
                            );
                            assert!(cargo_args.contains(&"--no-default-features"), "{args:?}");
                            assert!(!args.contains(&"--all-features"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn broad_commands_select_package_tests_and_preserve_explicit_targets() {
        let mut c =
            toml::from_str::<Catalogue>(include_str!("../tests/fixture/mutations-v0.1.toml"))
                .unwrap()
                .control
                .remove(0);
        for runner in ["cargo", "nextest"] {
            c.runner = runner.into();
            for target in ["--lib", "--tests", "--example contract", "--bench contract"] {
                c.target = Some(target.into());
                // Both the separate build and execution must select all test
                // targets, rather than compile them and then run only the row.
                for mode in ["build", "run"] {
                    let cmd = command(&c, mode, Scope::Broad).unwrap();
                    let args: Vec<_> = cmd.get_args().map(|s| s.to_str().unwrap()).collect();
                    assert!(args.windows(2).any(|w| w == ["-p", "mutation-fixture"]));
                    assert_eq!(args.iter().filter(|a| **a == "--tests").count(), 1);
                    let targets = c.targets().unwrap();
                    assert!(args.windows(targets.len()).any(|w| w == targets));
                }
            }
            c.target = Some("--lib".into());
            let row = command(&c, "build", Scope::Row).unwrap();
            assert!(!row.get_args().any(|a| a == "--tests"));
        }
    }
}

#[cfg(test)]
mod failure_report_tests {
    use super::*;

    #[test]
    fn failure_cap_keeps_head_tail_and_counts_elided_bytes() {
        for text in [
            format!("SETUP\n{}\nPANIC: required assertion", "x".repeat(40_000)),
            format!("SETUP\n{}\nPANIC: required assertion", "🦀é".repeat(8_000)),
        ] {
            let bounded = bounded_failure(&text);
            assert!(bounded.len() <= 16 * 1024);
            let (head, rest) = bounded.split_once("\n[… ").unwrap();
            let (elided, end) = rest.split_once(" bytes elided …]\n").unwrap();
            assert!(text.starts_with(head));
            assert!((4093..=4096).contains(&head.len()));
            assert!(text.ends_with(end));
            assert!((12_200..=12_288).contains(&end.len()));
            assert_eq!(
                elided.parse::<usize>().unwrap(),
                text.len() - head.len() - end.len()
            );
            assert!(bounded.starts_with("SETUP\n"));
            assert!(bounded.ends_with("PANIC: required assertion"));
        }
        for text in [String::new(), "é".repeat(8192)] {
            assert_eq!(bounded_failure(&text), text);
        }
    }

    #[test]
    fn nextest_human_failure_survives_hundreds_of_later_passes() {
        let mut text = String::from("FAIL [ 0.001s] fixture collateral\n  stdout ───\nsetup for collateral\n  stderr ───\nthread panicked: collateral invariant\n");
        for i in 0..350 {
            text.push_str(&format!("PASS [ 0.001s] fixture passing_test_{i:03}\n"));
        }
        text.push_str("Summary [ 1.0s] 351 tests run: 350 passed, 1 failed\nFAIL [ 0.001s] fixture collateral\n");
        assert!(!tail(&text).contains("collateral invariant"));
        let results = parse_test_results(&text, "nextest").unwrap();
        assert_eq!(results.green.len(), 350);
        let failures = results.failures();
        assert_eq!(failures.len(), 1);
        assert!(failures["collateral"].contains("setup for collateral"));
        assert!(failures["collateral"].contains("collateral invariant"));
        assert!(!failures["collateral"].contains("passing_test"));
    }

    #[test]
    fn nextest_json_failure_retains_stdout_and_stderr_only_for_red_tests() {
        let text = r#"{"type":"suite","event":"started","test_count":3}
{"type":"test","event":"failed","name":"fixture::one$shared","stdout":"setup one","stderr":"panic one"}
{"type":"test","event":"failed","name":"fixture::two$shared","stdout":"setup two\n","stderr":"panic two"}
{"type":"test","event":"ok","name":"fixture::two$passes","stdout":"green output","stderr":"green error"}
{"type":"suite","event":"failed","passed":1,"failed":2}"#;
        let failures = parse_test_results(text, "nextest").unwrap().failures();
        assert_eq!(failures.len(), 2);
        assert_eq!(failures["fixture::one::shared"], "setup one\npanic one");
        assert_eq!(failures["fixture::two::shared"], "setup two\npanic two");
    }
}

#[cfg(test)]
mod target_parser_tests {
    use super::*;

    #[test]
    fn recorded_abort_outputs_identify_the_test_and_signal() {
        for (runner, text) in [
            ("cargo", include_str!("../tests/fixture/cargo-abort.txt")),
            (
                "nextest",
                include_str!("../tests/fixture/nextest-abort.txt"),
            ),
        ] {
            let results = parse_test_results(text, runner).unwrap();
            assert_eq!(results.red, ["tests::waits"]);
            assert!(results.signals["tests::waits"].contains("SIGABRT"));
            let segfault = parse_test_results(&text.replace("SIGABRT", "SIGSEGV"), runner).unwrap();
            assert!(segfault.signals["tests::waits"].contains("SIGSEGV"));
        }
    }

    #[test]
    fn cargo_missing_binary_summary_is_not_hidden_by_another_summary() {
        let incomplete = "Running tests/one.rs (target/debug/deps/one-0123456789abcdef)\ntest first ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\nRunning tests/two.rs (target/debug/deps/two-0123456789abcdef)\ntest second ... ";
        let error = parse_test_results(incomplete, "cargo").unwrap_err();
        assert!(
            error.contains("two-")
                && error.contains("second")
                && error.contains("without a summary"),
            "{error}"
        );
    }

    #[test]
    fn cargo_summary_counts_cannot_cancel_out_between_binaries() {
        let output = "Running tests/one.rs (target/debug/deps/one-0123456789abcdef)\ntest first ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\nRunning tests/two.rs (target/debug/deps/two-0123456789abcdef)\ntest second ... ok\ntest third ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;";
        assert!(parse_test_results(output, "cargo")
            .unwrap_err()
            .contains("counts disagree"));
    }

    #[test]
    fn nextest_human_fallback_preserves_signal_and_failure_output() {
        let results = parse_test_results("SIGABRT [ 0.01s] fixture tests::abort\n  output ───\n (test aborted with signal 6: SIGABRT)\nFAIL [ 0.01s] fixture tests::panic\n  output ───\nthread panicked: required assertion\nSummary [ 0.02s] 2 tests run: 0 passed, 2 failed", "nextest").unwrap();
        assert_eq!(results.signals["tests::abort"], "SIGABRT");
        assert!(results.failure_output["tests::panic"].contains("required assertion"));
        assert!(!results.failure_output["tests::abort"].contains("required assertion"));
    }

    #[test]
    fn repeated_test_names_keep_their_own_failure_messages() {
        let results = parse_test_results(r#"{"type":"test","event":"failed","name":"fixture::one$tests::shared","stdout":"first assertion"}
{"type":"test","event":"failed","name":"fixture::two$tests::shared","stdout":"second assertion"}
{"type":"suite","event":"failed","passed":0,"failed":2}"#, "nextest").unwrap();
        assert_eq!(
            results.failure_output["fixture::one::tests::shared"],
            "first assertion"
        );
        assert_eq!(
            results.failure_output["fixture::two::tests::shared"],
            "second assertion"
        );
        let cargo = parse_test_results("Running tests/one.rs (target/debug/deps/one-0123456789abcdef)\ntest shared ... FAILED\nfailures:\n---- shared stdout ----\nfirst assertion\nfailures:\n shared\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out;\nRunning tests/two.rs (target/debug/deps/two-0123456789abcdef)\ntest shared ... FAILED\nfailures:\n---- shared stdout ----\nsecond assertion\nfailures:\n shared\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out;", "cargo").unwrap();
        assert!(cargo.failure_output["one::shared"].contains("first assertion"));
        assert!(!cargo.failure_output["one::shared"].contains("second assertion"));
        assert!(cargo.failure_output["two::shared"].contains("second assertion"));
    }

    #[test]
    fn hub_cargo_target_names_are_stable_across_rebuilds() {
        let mut c =
            toml::from_str::<Catalogue>(include_str!("../tests/fixture/mutations-v0.1.toml"))
                .unwrap()
                .control
                .remove(0);
        c.expect_red = vec!["guard".into()];
        for hash in ["0123456789abcdef", "fedcba9876543210"] {
            let results = parse_test_results(
                &format!(
                    "Running unittests src/lib.rs (target/debug/deps/fixture-{hash})\n\
                     test guard ... FAILED\n\
                     test same_target ... FAILED\n\
                     test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out;\n\
                     Running tests/encoder.rs (C:\\repo\\target\\debug\\deps\\encoder_e2e-{hash}.exe)\n\
                     test encoder_contract ... FAILED\n\
                     test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out;"
                ),
                "cargo",
            )
            .unwrap();
            let (extra, cross_targets) = collateral(&c, &results).unwrap();
            assert_eq!(extra.targets, ["encoder_e2e", "fixture"]);
            assert_eq!(extra.count, 2);
            assert_eq!(cross_targets, ["encoder_e2e"]);
        }
        assert_eq!(stable_target("cargo", "doc:fixture"), "doc:fixture");
        assert_eq!(stable_target("cargo", "my-target"), "my-target");
        assert_eq!(
            stable_target("nextest", "crate::encoder_e2e"),
            "crate::encoder_e2e"
        );
    }

    #[test]
    fn repeated_names_are_qualified_in_nextest_human_and_json_results() {
        for text in [
            "FAIL [ 0.001s] fixture::one tests::shared\nPASS [ 0.001s] fixture::two tests::shared\nSummary [ 0.01s] 2 tests run: 1 passed, 1 failed",
            "{\"type\":\"test\",\"event\":\"failed\",\"name\":\"fixture::one$tests::shared\"}\n{\"type\":\"test\",\"event\":\"ok\",\"name\":\"fixture::two$tests::shared\"}\n{\"type\":\"suite\",\"event\":\"failed\",\"passed\":1,\"failed\":1}",
        ] {
            let results = parse_test_results(text, "nextest").unwrap();
            assert_eq!(results.red, ["fixture::one::tests::shared"]);
            assert_eq!(results.green, ["fixture::two::tests::shared"]);
            assert_eq!(results.targets.len(), 2);
        }
    }

    #[test]
    fn cargo_parser_attributes_each_test_to_running_binary() {
        let results = parse_test_results(
            "Running unittests src/lib.rs (target/debug/deps/fixture-abc123)\n\
             test tests::guard ... FAILED\n\
             test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out;\n\
             Running tests/capacity.rs (C:\\repo\\target\\debug\\deps\\capacity-def456.exe)\n\
             test capacity_contract ... FAILED\n\
             test accepts ... ok\n\
             test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out;",
            "cargo",
        )
        .unwrap();
        assert_eq!(results.targets["tests::guard"], "fixture-abc123");
        assert_eq!(results.targets["capacity_contract"], "capacity-def456.exe");
        assert_eq!(results.targets["accepts"], "capacity-def456.exe");
    }

    #[test]
    fn nextest_human_parser_attributes_each_test_to_binary_token() {
        let results = parse_test_results(
            "FAIL [ 0.001s] fixture tests::guard\n\
             FAIL [ 0.001s] fixture::capacity capacity_contract\n\
             PASS [ 0.001s] fixture::capacity accepts\n\
             FAIL [ 0.001s] fixture::capacity capacity_contract\n\
             Summary [ 0.01s] 3 tests run: 1 passed, 2 failed",
            "nextest",
        )
        .unwrap();
        assert_eq!(results.targets["tests::guard"], "fixture");
        assert_eq!(results.targets["capacity_contract"], "fixture::capacity");
        assert_eq!(results.targets["accepts"], "fixture::capacity");
    }

    #[test]
    fn nextest_json_parser_attributes_each_test_to_qualified_prefix() {
        let results = parse_test_results(
            r#"{"type":"suite","event":"started"}
{"type":"test","event":"failed","name":"fixture::fixture$tests::guard"}
{"type":"test","event":"failed","name":"fixture::capacity$capacity_contract"}
{"type":"test","event":"ok","name":"fixture::capacity$accepts"}
{"type":"suite","event":"failed","passed":1,"failed":2}"#,
            "nextest",
        )
        .unwrap();
        assert_eq!(results.targets["tests::guard"], "fixture::fixture");
        assert_eq!(results.targets["capacity_contract"], "fixture::capacity");
        assert_eq!(results.targets["accepts"], "fixture::capacity");
    }
}

#[cfg(test)]
mod restoration_tests {
    use super::*;

    #[test]
    fn panic_unwinds_saved_bytes_before_releasing_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("lib.rs");
        fs::write(&file, b"original\r\nbytes\n").unwrap();
        let edit = Edit {
            file: "lib.rs".into(),
            old: "original".into(),
            new: "mutated".into(),
        };
        let caught = std::panic::catch_unwind(|| {
            let _saved = Saved::new(&root, &[edit], true).unwrap();
            fs::write(&file, b"mutated").unwrap();
            panic!("fixture panic after mutation");
        });
        assert!(caught.is_err());
        assert_eq!(fs::read(file).unwrap(), b"original\r\nbytes\n");
    }
}

#[cfg(test)]
mod hint_tests {
    use super::*;

    const SOURCE: &str = r#"const MAX_HOPS: u32 = 40;
pub struct Names { room: String }
/// Doc comment with a brace { that must not count.
pub fn guarded(value: i32) -> bool {
    let _ = "a string with } and fn fake() {";
    let _ = '}';
    value > 0
}
impl Names {
    fn derive(account: &str) -> Self {
        let upper = account.to_ascii_uppercase();
        Self { room: format!("CK_{upper}_ROOM") }
    }
}
pub fn caller() -> bool {
    guarded(1) && check::<u8>(2)
}
fn check<T>(_: i32) -> bool { true }
fn main() { let _ = caller() || 1 > 0; }
"#;

    fn edit(old: &str, new: &str) -> Edit {
        Edit {
            file: "src/lib.rs".into(),
            old: old.into(),
            new: new.into(),
        }
    }

    #[test]
    fn body_mutation_names_the_enclosing_function() {
        let hint = call_site_hint(SOURCE, &edit("value > 0", "value >= 0")).unwrap();
        assert!(hint.contains("removing a call to `guarded`"), "{hint}");
    }

    #[test]
    fn method_body_mutation_and_call_swap_name_the_method() {
        let hint = call_site_hint(
            SOURCE,
            &edit(
                "account.to_ascii_uppercase()",
                "account.to_ascii_lowercase()",
            ),
        )
        .unwrap();
        assert!(hint.contains("`derive`"), "{hint}");
        let hint = call_site_hint(SOURCE, &edit("_ROOM\")", "_MUTATED_ROOM\")")).unwrap();
        assert!(hint.contains("`derive`"), "{hint}");
    }

    #[test]
    fn whole_function_rewrite_names_the_rewritten_function() {
        let old = "fn check<T>(_: i32) -> bool { true }";
        let hint =
            call_site_hint(SOURCE, &edit(old, "fn check<T>(_: i32) -> bool { false }")).unwrap();
        assert!(hint.contains("`check`"), "{hint}");
    }

    #[test]
    fn call_site_removal_gets_no_hint() {
        assert_eq!(call_site_hint(SOURCE, &edit("guarded(1)", "true")), None);
        assert_eq!(
            call_site_hint(SOURCE, &edit("check::<u8>(2)", "true")),
            None
        );
    }

    #[test]
    fn mutation_outside_any_function_body_gets_no_hint() {
        assert_eq!(
            call_site_hint(SOURCE, &edit("MAX_HOPS: u32 = 40", "MAX_HOPS: u32 = 0")),
            None
        );
        assert_eq!(
            call_site_hint(SOURCE, &edit("room: String", "room: Box<str>")),
            None
        );
    }

    #[test]
    fn main_has_no_call_site_to_remove() {
        assert_eq!(call_site_hint(SOURCE, &edit("1 > 0", "1 < 0")), None);
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    fn row(id: &str, package: &str, file: &str) -> Control {
        Control {
            id: id.into(),
            guards: "g".into(),
            file: Some(file.into()),
            old: Some("a".into()),
            new: Some("b".into()),
            edits: vec![],
            test_file: file.into(),
            runner: "cargo".into(),
            package: Some(package.into()),
            target: None,
            features: None,
            no_default_features: None,
            all_features: None,
            command: None,
            test_count_pattern: None,
            catch_on: None,
            output_normalize: vec![],
            expect_message: None,
            signal_is_catch: None,
            expect_red: vec!["t".into()],
            only: false,
            equivalent: None,
            equivalent_guard: None,
            unreachable: None,
            desk_only: None,
            hub: None,
            hub_targets: None,
            platforms: None,
            timeout_s: 1,
            build_timeout_s: 1,
        }
    }

    #[test]
    fn rows_execute_grouped_by_package_then_file_stably() {
        let rows = [
            row("a", "pkg-b", "b/src/lib.rs"),
            row("b", "pkg-a", "a/src/z.rs"),
            row("c", "pkg-b", "b/src/lib.rs"),
            row("d", "pkg-a", "a/src/lib.rs"),
            row("e", "pkg-a", "a/src/z.rs"),
        ];
        let mut multi = row("f", "pkg-a", "unused");
        multi.file = None;
        multi.old = None;
        multi.new = None;
        multi.edits = vec![Edit {
            file: "a/src/lib.rs".into(),
            old: "a".into(),
            new: "b".into(),
        }];
        let refs: Vec<&Control> = rows.iter().chain([&multi]).collect();
        let ids: Vec<&str> = execution_order(&refs)
            .into_iter()
            .map(|i| refs[i].id.as_str())
            .collect();
        assert_eq!(ids, ["d", "f", "b", "e", "a", "c"]);
    }
}

#[cfg(test)]
mod baseline_identity_tests {
    use super::*;

    #[test]
    fn baseline_exclusion_matches_binary_and_name_with_either_path_separator() {
        let c: Control = toml::from_str(
            r#"
id = "guard"
guards = "a binary-specific baseline"
file = "src/lib.rs"
old = "true"
new = "false"
test_file = "src/lib.rs"
runner = "cargo"
package = "fixture"
expect_red = ["other::same_name"]
"#,
        )
        .unwrap();
        let mut report = Report::new(&c);
        report
            .baseline_red
            .insert("fixture".into(), vec!["same_name".into()]);
        for (separator, extension) in [("/", ""), ("\\", ".exe")] {
            let mut results = parse_test_results(&format!(
                "Running unittests src{separator}lib.rs (target{separator}debug{separator}deps{separator}fixture-123abc123abc123a{extension})\n\
                 test same_name ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out;\n\
                 Running tests{separator}other.rs (target{separator}debug{separator}deps{separator}other-456def456def456d{extension})\n\
                 test same_name ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out;"
            ), "cargo").unwrap();
            exclude_baseline_red(&c, &report, &mut results);
            assert_eq!(results.red, ["other::same_name"]);
        }
    }
}
