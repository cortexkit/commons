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
    pub package: String,
    #[serde(default)]
    pub target: String,
    pub expect_red: Vec<String>,
    #[serde(default)]
    pub only: bool,
    pub equivalent: Option<String>,
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
        let words: Vec<_> = self.target.split_whitespace().collect();
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
                        self.id, self.target
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
    toml::from_str(&fs::read_to_string(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
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
            || c.expect_red.is_empty()
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
    if c.package.trim().is_empty()
        || !matches!(c.runner.as_str(), "cargo" | "nextest")
        || c.timeout_s == 0
        || c.build_timeout_s == 0
    {
        return Err(format!("{}: invalid required field", c.id));
    }
    c.targets()?;
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
        let changed_lock = read_optional(&self.lock)? != self.lock_bytes;
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
    Survived,
    WrongTest,
    NoTestsRan,
    AnchorMissing,
    DidNotCompile,
    TimedOut,
    Equivalent,
    Error,
}
/// Which deadline a TIMED_OUT row hit.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// The separate `--no-run` build, bounded by `build_timeout_s`.
    Build,
    /// The test run, bounded by `timeout_s`.
    Test,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub id: String,
    pub outcome: Outcome,
    /// Set only on TIMED_OUT: the phase whose deadline expired.
    pub timed_out_phase: Option<Phase>,
    pub red: Vec<String>,
    pub green: Vec<String>,
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
            build_ms: 0,
            test_ms: 0,
            build_tail: String::new(),
            test_tail: String::new(),
            reason: None,
        }
    }
    pub fn passes(&self) -> bool {
        matches!(self.outcome, Outcome::Caught | Outcome::Equivalent)
    }
}

struct Output {
    success: bool,
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
    /// The row's whole package without target selection, graded against
    /// `expect_red` (the second replay `prove` makes for a survivor).
    Package,
    /// `explore`: the whole package, or the whole workspace; any red test is a
    /// catch because nobody has named the guarding tests yet.
    Explore { workspace: bool },
}

fn command(c: &Control, mode: &str, scope: Scope) -> Result<Command> {
    let mut cmd = Command::new("cargo");
    if c.runner == "nextest" {
        cmd.args(["nextest", if mode == "list" { "list" } else { "run" }]);
    } else {
        cmd.arg("test");
    }
    cmd.arg("--locked");
    match scope {
        Scope::Row => {
            cmd.args(["-p", &c.package]);
            cmd.args(c.targets()?);
        }
        Scope::Package | Scope::Explore { workspace: false } => {
            cmd.args(["-p", &c.package]);
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
    let stderr = dir.join("ck-mutate.stderr");
    let out_file = File::create(&stdout).map_err(|e| e.to_string())?;
    let err_file = File::create(&stderr).map_err(|e| e.to_string())?;
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
    let success = loop {
        if stop.load(Ordering::SeqCst) || start.elapsed() >= Duration::from_secs(timeout) {
            interrupted = stop.load(Ordering::SeqCst);
            timed_out = !interrupted;
            kill_tree(&mut child.0);
            let _ = child.0.wait();
            break false;
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            break status.success();
        }
        thread::sleep(Duration::from_millis(20));
    };
    drop(child);
    let text = format!(
        "{}\n{}",
        fs::read_to_string(&stdout).map_err(|e| e.to_string())?,
        fs::read_to_string(&stderr).map_err(|e| e.to_string())?
    );
    let _ = fs::remove_file(stdout);
    let _ = fs::remove_file(stderr);
    Ok(Output {
        success,
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

pub fn parse_tests(text: &str, runner: &str) -> Result<(Vec<String>, Vec<String>)> {
    if runner == "nextest" && text.lines().any(|l| l.starts_with("{\"type\":\"suite\"")) {
        return parse_nextest_json(text);
    }
    let mut red = BTreeSet::new();
    let mut green = BTreeSet::new();
    let mut summary_total = 0;
    let mut observed_summary = false;
    let mut nextest_total = None;
    for line in text.lines() {
        let line = line.trim();
        if runner == "cargo" {
            if let Some(rest) = line.strip_prefix("test result:") {
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
                            green.insert(name.to_owned());
                        }
                        "FAILED" => {
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
            return Ok((vec![], vec![]));
        }
        return Err("nextest summary missing or counts disagree with per-test output".into());
    }
    if !red.is_disjoint(&green) {
        return Err("ambiguous test name across test binaries".into());
    }
    Ok((red.into_iter().collect(), green.into_iter().collect()))
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
    if let Some(reason) = &c.equivalent {
        report.outcome = Outcome::Equivalent;
        report.reason = Some(reason.clone());
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
        for (path, text) in files {
            fs::write(path, text).map_err(|e| e.to_string())?;
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
        let (red, green) = parse_tests(&tests.text, &c.runner)?;
        report.outcome = match scope {
            Scope::Explore { .. } => explore_grade(&red, &green),
            Scope::Row | Scope::Package => grade(c, &red, &green),
        };
        report.red = red;
        report.green = green;
        Ok(())
    })();
    let restoration = saved.restore();
    if let Err(e) = work {
        report.outcome = Outcome::Error;
        report.timed_out_phase = None;
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

pub fn signal_flag() -> Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    let handler = flag.clone();
    ctrlc::set_handler(move || {
        handler.store(true, Ordering::SeqCst);
    })
    .map_err(|e| e.to_string())?;
    Ok(flag)
}

fn parse_nextest_json(text: &str) -> Result<(Vec<String>, Vec<String>)> {
    let mut red = BTreeSet::new();
    let mut green = BTreeSet::new();
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
            let name = qualified
                .split_once('$')
                .ok_or("nextest name missing binary prefix")?
                .1
                .to_owned();
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
    Ok((red.into_iter().collect(), green.into_iter().collect()))
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
