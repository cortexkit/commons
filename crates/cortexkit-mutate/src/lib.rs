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
    /// An argv template, not a shell string; substitutes one expected test id.
    pub command: Option<Vec<String>>,
    pub expect_red: Vec<String>,
    #[serde(default)]
    pub only: bool,
    pub equivalent: Option<String>,
    /// A person's explanation of why the mutated code has no production caller.
    pub unreachable: Option<String>,
    /// A reviewed explanation of why multiple test targets guard this property.
    pub hub: Option<String>,
    /// Stable collateral target names approved by the reviewer of a broad catch.
    pub hub_targets: Option<Vec<String>>,
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
    fn validate_runner(&self) -> Result<()> {
        let invalid = |field: &str, reason: &str| format!("{}: {field} {reason}", self.id);
        match self.runner.as_str() {
            "command" => {
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
                if self.expect_red.is_empty() && self.unreachable.is_none() {
                    return Err(invalid("expect_red", "must name at least one test"));
                }
                for id in &self.expect_red {
                    if id.is_empty() || id.chars().any(|c| c.is_whitespace() || c.is_control()) {
                        return Err(invalid("expect_red", &format!("invalid test id {id:?}: ids must be nonempty without whitespace or control characters")));
                    }
                }
            }
            "cargo" | "nextest" => {
                if self.command.is_some() {
                    return Err(invalid("command", "must be absent for cargo/nextest rows"));
                }
                if self.package.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    return Err(invalid("package", "is required and must be nonempty"));
                }
                self.targets()?;
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
        if self.equivalent.is_some() || self.unreachable.is_some() {
            return Err(format!(
                "{}: hub, equivalent and unreachable are exclusive",
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
        if self.equivalent.is_some() && self.unreachable.is_some() {
            return Err(format!(
                "{}: equivalent and unreachable are exclusive",
                self.id
            ));
        }
        let disposition = self
            .equivalent
            .as_deref()
            .map(|r| (Outcome::Equivalent, r))
            .or_else(|| {
                self.unreachable
                    .as_deref()
                    .map(|r| (Outcome::Unreachable, r))
            });
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
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalogue {
    #[serde(default)]
    pub control: Vec<Control>,
}

pub fn load(path: &Path) -> Result<Catalogue> {
    // Deserialize each row separately so malformed fields still name its id.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Rows {
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
            c.validate_hub()?;
            Ok(c)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Catalogue { control })
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
            || (c.expect_red.is_empty() && c.unreachable.is_none())
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
    NoTestsRan,
    AnchorMissing,
    DidNotCompile,
    TimedOut,
    Equivalent,
    Unreachable,
    Hub,
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
}

#[derive(Debug, Serialize)]
pub struct Collateral {
    pub count: usize,
    pub targets: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub id: String,
    pub outcome: Outcome,
    /// The phase whose deadline expired, including command-row ERROR timeouts.
    pub timed_out_phase: Option<Phase>,
    pub red: Vec<String>,
    pub green: Vec<String>,
    pub collateral: Collateral,
    /// True only after an opt-in broad audit produced complete test results.
    /// A normal replay reports collateral but does not observe package breadth.
    pub breadth_observed: bool,
    pub build_ms: u128,
    pub test_ms: u128,
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
            collateral: Collateral {
                count: 0,
                targets: vec![],
            },
            breadth_observed: false,
            build_ms: 0,
            test_ms: 0,
            build_tail: String::new(),
            test_tail: String::new(),
            reason: None,
        }
    }
    pub fn passes(&self) -> bool {
        self.outcome.is_caught()
            || matches!(self.outcome, Outcome::Equivalent | Outcome::Unreachable)
    }
}

struct Output {
    success: bool,
    code: Option<i32>,
    timeout: bool,
    interrupted: bool,
    text: String,
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
fn execute(root: &Path, mut cmd: Command, timeout: u64, stop: &AtomicBool) -> Result<Output> {
    // Regular files cannot deadlock on full pipes or descendants holding pipes open.
    let git_dir = git(root, &["rev-parse", "--git-dir"])?;
    let dir = root.join(String::from_utf8_lossy(&git_dir).trim());
    let stdout = dir.join("ck-mutate.stdout");
    let out_file = File::create(&stdout).map_err(|e| e.to_string())?;
    // Cargo prints binary headers on stderr and test events on stdout. Sharing
    // the file offset preserves their order, so events keep their binary identity.
    let err_file = out_file.try_clone().map_err(|e| e.to_string())?;
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
    let text = fs::read_to_string(&stdout).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(stdout);
    Ok(Output {
        success: status.is_some_and(|s| s.success()),
        code: status.and_then(|s| s.code()),
        timeout: timed_out,
        interrupted,
        text,
        ms: start.elapsed().as_millis(),
    })
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

// Both baseline and mutant commands use the same child supervisor as Cargo.
// A failing baseline or a broken process is not evidence that a test caught a mutant.
fn command_tests(
    root: &Path,
    c: &Control,
    stop: &AtomicBool,
    report: &mut Report,
    baseline: bool,
) -> Result<()> {
    let argv = c.command.as_ref().ok_or("command is required")?;
    for id in &c.expect_red {
        let label = if baseline {
            "baseline was not green"
        } else {
            "mutated command"
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
        report.test_ms += output.ms;
        report.test_tail = tail(&format!(
            "{}\n{label}: {id}\n{}",
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
        let code = output
            .code
            .ok_or_else(|| error("process died by signal (no exit code)".into()))?;
        if matches!(code, 126 | 127) {
            return Err(error(format!(
                "exit {code}: program not executable or not found"
            )));
        }
        if baseline {
            if code != 0 {
                return Err(error(format!("exit {code}")));
            }
        } else if code == 0 {
            report.green.push(id.clone());
        } else {
            report.red.push(id.clone());
        }
    }
    Ok(())
}

pub fn parse_tests(text: &str, runner: &str) -> Result<(Vec<String>, Vec<String>)> {
    let results = parse_test_results(text, runner)?;
    Ok((results.red, results.green))
}

#[derive(Debug)]
struct TestResults {
    red: Vec<String>,
    green: Vec<String>,
    targets: BTreeMap<String, String>,
}

fn attribute(targets: &mut BTreeMap<String, String>, name: &str, target: &str) -> Result<()> {
    if targets.get(name).is_some_and(|old| old != target) {
        return Err(format!("ambiguous test name across test binaries: {name}"));
    }
    targets.insert(name.to_owned(), target.to_owned());
    Ok(())
}

fn parse_test_results(text: &str, runner: &str) -> Result<TestResults> {
    if runner == "nextest" && text.lines().any(|l| l.starts_with("{\"type\":\"suite\"")) {
        return parse_nextest_json(text);
    }
    let mut red = BTreeSet::new();
    let mut green = BTreeSet::new();
    let mut targets = BTreeMap::new();
    let mut cargo_target = String::new();
    let mut summary_total = 0;
    let mut observed_summary = false;
    let mut nextest_total = None;
    for line in text.lines() {
        let line = line.trim();
        if runner == "cargo" {
            if let Some(rest) = line.strip_prefix("Running ") {
                let (_, binary) = rest
                    .rsplit_once(" (")
                    .ok_or("cargo header missing binary")?;
                cargo_target = binary
                    .strip_suffix(')')
                    .ok_or("invalid cargo binary header")?
                    .rsplit(['/', '\\'])
                    .next()
                    .ok_or("empty cargo binary")?
                    .to_owned();
            } else if let Some(target) = line.strip_prefix("Doc-tests ") {
                cargo_target = format!("doc:{target}");
            } else if let Some(rest) = line.strip_prefix("test result:") {
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
                    }
                }
            } else if let Some(rest) = line.strip_prefix("test ") {
                if let Some((name, status)) = rest.rsplit_once(" ... ") {
                    match status {
                        "ok" => {
                            attribute(&mut targets, name, &cargo_target)?;
                            green.insert(name.to_owned());
                        }
                        "FAILED" => {
                            attribute(&mut targets, name, &cargo_target)?;
                            red.insert(name.to_owned());
                        }
                        status if status == "ignored" || status.starts_with("ignored,") => {}
                        _ => return Err(format!("unrecognized test status: {line}")),
                    }
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
            if matches!(words.first(), Some(&"PASS") | Some(&"FAIL")) {
                let name = words.last().ok_or("empty nextest status")?.to_string();
                let target = words
                    .get(words.len().checked_sub(2).ok_or("missing nextest binary")?)
                    .ok_or("missing nextest binary")?;
                attribute(&mut targets, &name, target)?;
                if words[0] == "PASS" {
                    green.insert(name);
                } else {
                    red.insert(name);
                }
            }
        }
    }
    if runner == "cargo" && (!observed_summary || summary_total != red.len() + green.len()) {
        return Err("libtest summary missing or counts disagree with per-test output".into());
    }
    if runner == "nextest" && nextest_total != Some(red.len() + green.len()) {
        if red.is_empty() && green.is_empty() && text.contains("no tests to run") {
            return Ok(TestResults {
                red: vec![],
                green: vec![],
                targets,
            });
        }
        return Err("nextest summary missing or counts disagree with per-test output".into());
    }
    if !red.is_disjoint(&green) {
        return Err("ambiguous test name across test binaries".into());
    }
    Ok(TestResults {
        red: red.into_iter().collect(),
        green: green.into_iter().collect(),
        targets,
    })
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

fn collateral(c: &Control, results: &TestResults) -> Result<(Collateral, bool)> {
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
    let broad = targets
        .iter()
        .any(|target| !expected_targets.contains(target.as_str()));
    Ok((
        Collateral {
            count: extra.len(),
            targets: targets.into_iter().collect(),
        },
        broad,
    ))
}

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
    replay(root, c, allow_dirty, stop, scope)
}

/// Audit one catalogue row across every test target in its package, retaining
/// any explicitly selected targets. Only this opt-in replay grades broad catches.
pub fn run_broad_row(
    root: &Path,
    c: &Control,
    allow_dirty: bool,
    stop: &AtomicBool,
) -> Result<Report> {
    replay(root, c, allow_dirty, stop, Scope::Broad)
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
    replay(root, c, allow_dirty, stop, Scope::Explore { workspace })
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
) -> Result<Report> {
    let mut report = Report::new(c);
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
        if c.runner == "command" {
            command_tests(root, c, stop, &mut report, true)?;
        }
        for (path, text) in files {
            fs::write(path, text).map_err(|e| e.to_string())?;
        }
        if c.runner == "command" {
            command_tests(root, c, stop, &mut report, false)?;
            report.outcome = grade(c, &report.red, &report.green);
            return Ok(());
        }
        let build = execute(root, command(c, "build", scope)?, c.build_timeout_s, stop)?;
        report.build_ms = build.ms;
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
        let tests = execute(root, command(c, "run", scope)?, c.timeout_s, stop)?;
        report.test_ms = tests.ms;
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
        let results = parse_test_results(&tests.text, &c.runner)?;
        report.outcome = match scope {
            // Explore has no expected tests: every red name becomes expect_red
            // when recorded, so none is collateral to that discovery.
            Scope::Explore { .. } => explore_grade(&results.red, &results.green),
            Scope::Row | Scope::Package | Scope::Broad => {
                let (extra, broad) = collateral(c, &results)?;
                report.collateral = extra;
                report.breadth_observed = scope == Scope::Broad;
                let outcome = grade(c, &results.red, &results.green);
                if outcome == Outcome::Caught && report.breadth_observed {
                    if let Some(reason) = &c.hub {
                        let approved = c.hub_targets.as_deref().unwrap_or_default();
                        let new_targets: Vec<_> = report
                            .collateral
                            .targets
                            .iter()
                            .filter(|target| !approved.contains(target))
                            .cloned()
                            .collect();
                        // A reviewed hub permits only the recorded target set;
                        // fewer collateral targets are fine, new ones need review.
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
                    } else if broad {
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
    let restoration = saved.restore();
    if let Err(e) = work {
        report.outcome = Outcome::Error;
        report.reason = Some(e);
    }
    if let Err(e) = restoration {
        report.outcome = Outcome::Error;
        report.timed_out_phase = None;
        report.reason = Some(e);
    }
    Ok(report)
}

pub fn check(root: &Path, catalogue: &Catalogue, stop: &AtomicBool) -> Result<()> {
    validate(root, catalogue)?;
    for c in &catalogue.control {
        replaced(root, &c.edits()?)?;
        // Unreachable rows have no names to discover. Command rows have no list
        // protocol: replay verifies their expected ids with a fresh baseline.
        if c.unreachable.is_some() || c.runner == "command" {
            continue;
        }
        // List mode compiles the test binaries, so the build deadline bounds it.
        let output = execute(
            root,
            command(c, "list", Scope::Row)?,
            c.build_timeout_s,
            stop,
        )?;
        if !output.success || output.timeout || output.interrupted {
            return Err(format!("{}: list failed: {}", c.id, tail(&output.text)));
        }
        let names: BTreeSet<String> = if c.runner == "cargo" {
            output
                .text
                .lines()
                .filter_map(|l| l.strip_suffix(": test").map(str::to_owned))
                .collect()
        } else {
            let json: serde_json::Value = serde_json::from_str(
                output
                    .text
                    .lines()
                    .find(|l| l.starts_with('{'))
                    .ok_or("missing nextest list JSON")?,
            )
            .map_err(|e| e.to_string())?;
            json.get("rust-suites")
                .and_then(|v| v.as_object())
                .ok_or("missing nextest rust-suites")?
                .values()
                .flat_map(|s| {
                    s.get("testcases")
                        .and_then(|v| v.as_object())
                        .into_iter()
                        .flat_map(|m| m.keys().cloned())
                })
                .collect()
        };
        for expected in &c.expect_red {
            if !names.contains(expected) {
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
    let mut red = BTreeSet::new();
    let mut green = BTreeSet::new();
    let mut targets = BTreeMap::new();
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
            attribute(&mut targets, name, target)?;
            let name = name.to_owned();
            if red.contains(&name) || green.contains(&name) {
                return Err(format!("ambiguous or repeated test name: {name}"));
            }
            if status == "ok" {
                green.insert(name);
            } else {
                red.insert(name);
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
    if summaries == 0 || total as usize != red.len() + green.len() {
        return Err("nextest summary missing or inconsistent".into());
    }
    Ok(TestResults {
        red: red.into_iter().collect(),
        green: green.into_iter().collect(),
        targets,
    })
}

#[cfg(test)]
mod broad_command_tests {
    use super::*;

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
mod target_parser_tests {
    use super::*;

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
            let (extra, broad) = collateral(&c, &results).unwrap();
            assert_eq!(extra.targets, ["encoder_e2e", "fixture"]);
            assert_eq!(extra.count, 2);
            assert!(broad);
        }
        assert_eq!(stable_target("cargo", "doc:fixture"), "doc:fixture");
        assert_eq!(stable_target("cargo", "my-target"), "my-target");
        assert_eq!(
            stable_target("nextest", "crate::encoder_e2e"),
            "crate::encoder_e2e"
        );
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
            command: None,
            expect_red: vec!["t".into()],
            only: false,
            equivalent: None,
            unreachable: None,
            hub: None,
            hub_targets: None,
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
