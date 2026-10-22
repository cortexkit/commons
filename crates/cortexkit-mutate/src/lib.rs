#![forbid(unsafe_code)]

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct RowSource {
    pub file: String,
    pub line: usize,
}

impl RowSource {
    fn location(&self) -> String {
        if self.file.is_empty() {
            "<unrecorded row>".into()
        } else if self.line == 0 {
            self.file.clone()
        } else {
            format!("{}:{}", self.file, self.line)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub file: String,
    pub old: String,
    pub new: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AnchorScope {
    pub after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub id: String,
    /// Load-time provenance is not part of the catalogue's TOML schema.
    #[serde(skip)]
    pub source: RowSource,
    pub guards: String,
    pub file: Option<String>,
    pub old: Option<String>,
    pub new: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<AnchorScope>,
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
    /// Whether every list and test run in this row also runs `#[ignore]`d
    /// tests ("include") or runs only them ("only"). Absent runs only
    /// tests that are not ignored, as Cargo does by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored: Option<IgnoredSelection>,
    /// Narrow the target to expected names, a substring, or an exact run set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<TestSelection>,
    /// An argv template, not a shell string; substitutes one expected test id.
    pub command: Option<Vec<String>>,
    /// Lists the names selected by the same {test} argument used at replay.
    /// Stdout must be one JSON array of strings, with diagnostics on stderr.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_command: Option<Vec<String>>,
    /// Package-wide argv run only during a breadth audit, after named tests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broad_command: Option<Vec<String>>,
    /// Repository-relative JUnit XML output; removed before the audit command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broad_report: Option<String>,
    /// Maps JUnit testcase attributes to exact catalogue ids; `{file}` uses the
    /// testcase file or its enclosing testsuite file, and `{suite}` the names of
    /// the enclosing testsuites below the file-level one, outermost first,
    /// joined with ` > ` (see [`JunitClass::Suites`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broad_id: Option<String>,
    /// Where a JUnit breadth audit reads each testcase's target from: its
    /// `classname` attribute ("classname", the default when absent) or the
    /// enclosing testsuite names ("suites"). Only valid on rows that set
    /// `broad_command`, `broad_report` and `broad_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub junit_class: Option<JunitClass>,
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
    /// without Cargo's executable hash; command-row targets use the nonempty
    /// JUnit classname (or, with `junit_class = "suites"`, the nonempty suite
    /// path), then testcase file, then enclosing testsuite file.
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
/// Which `#[ignore]`d tests a cargo or nextest row selects. Typed rather than
/// free-form so a catalogue row can never inject arbitrary harness arguments.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum IgnoredSelection {
    /// Run ignored tests as well as the ordinary ones.
    Include,
    /// Run only the ignored tests.
    Only,
}

/// Which JUnit testcase identity a command row's breadth audit records as the
/// testcase's target. `hub_targets` and CAUGHT_BROADLY grading compare targets.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JunitClass {
    /// The testcase's `classname` attribute as written by the runner.
    #[default]
    Classname,
    /// The `name` attributes of the enclosing `<testsuite>` elements,
    /// outermost first, joined with ` > `. The file-level suite is left out:
    /// that is a `<testsuite>` with a `file` attribute and no enclosing
    /// `<testsuite>`, as Bun writes one per test file. Use this for runners
    /// such as Bun that nest one testsuite per `describe` but write nested
    /// classnames innermost first.
    Suites,
}

impl JunitClass {
    /// The catalogue spelling, as written in a row's `junit_class` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Classname => "classname",
            Self::Suites => "suites",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestSelection {
    Expected,
    Filter(String),
    Tests(Vec<String>),
}

/// What `run --narrow` did to one catalogue row. Narrowing runs only the tests a
/// row lists in `expect_red` (the tests its mutant must turn red) instead of the
/// row's whole test target; every other variant names why the row was left as
/// written.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NarrowSelection {
    Narrowed,
    OwnSelect,
    Only,
    CommandRunner,
    Hub,
    NotRun,
}

impl TestSelection {
    fn exact_names<'a>(&'a self, expected: &'a [String]) -> Option<&'a [String]> {
        match self {
            Self::Expected => Some(expected),
            Self::Tests(names) => Some(names),
            Self::Filter(_) => None,
        }
    }

    fn contains(&self, name: &str, expected: &[String]) -> bool {
        match self {
            Self::Filter(filter) => name.contains(filter),
            Self::Expected => expected.iter().any(|test| test == name),
            Self::Tests(names) => names.iter().any(|test| test == name),
        }
    }

    fn key(&self, expected: &[String]) -> String {
        // Include the variant and string boundaries, not just joined names.
        serde_json::to_string(&(self, self.exact_names(expected))).unwrap()
    }
}

impl IgnoredSelection {
    /// The libtest harness option, passed after Cargo's `--`.
    fn libtest_arg(self) -> &'static str {
        match self {
            Self::Include => "--include-ignored",
            Self::Only => "--ignored",
        }
    }

    /// The value of nextest's `--run-ignored` option: `all` or `only`. Older
    /// nextest releases that spell the second one `ignored-only` are not
    /// supported.
    fn nextest_value(self) -> &'static str {
        match self {
            Self::Include => "all",
            Self::Only => "only",
        }
    }

    /// The catalogue spelling, as written in a row's `ignored` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Include => "include",
            Self::Only => "only",
        }
    }
}

fn default_timeout() -> u64 {
    600
}
pub fn default_build_timeout() -> u64 {
    1800
}

impl Control {
    /// Narrow this row to `select = expected` unless doing so would change what the
    /// row asserts. Left as written: rows with their own `select`; `only = true`
    /// rows, which assert that every other test in the target stays green; command
    /// rows, which have no test selection to narrow; and `hub` rows, which assert
    /// that the mutant breaks tests across the listed `hub_targets`. Rows that will
    /// be skipped may list no expected tests, so they are left unchanged too.
    pub fn narrow(&mut self) -> NarrowSelection {
        if !self.matches_platform() || self.unreachable.is_some() || self.desk_only.is_some() {
            NarrowSelection::NotRun
        } else if self.select.is_some() {
            NarrowSelection::OwnSelect
        } else if self.only {
            NarrowSelection::Only
        } else if self.runner == "command" {
            NarrowSelection::CommandRunner
        } else if self.hub.is_some() || self.hub_targets.is_some() {
            NarrowSelection::Hub
        } else {
            self.select = Some(TestSelection::Expected);
            NarrowSelection::Narrowed
        }
    }

    pub fn label(&self) -> String {
        format!("{} ({})", self.id, self.source.location())
    }

    pub fn matches_platform(&self) -> bool {
        self.matches_platform_on(std::env::consts::OS)
    }

    fn matches_platform_on(&self, platform: &str) -> bool {
        self.platforms
            .as_ref()
            .is_none_or(|names| names.iter().any(|name| name == platform))
    }

    fn validate_platforms(&self) -> Result<()> {
        if let Some(names) = &self.platforms {
            if names.is_empty() {
                return Err(format!(
                    "{}: platforms must be nonempty when present",
                    self.label()
                ));
            }
            for name in names {
                if !known_os(name) {
                    return Err(format!(
                        "{}: unknown target_os in platforms: {name:?}",
                        self.label()
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_runner(&self) -> Result<()> {
        self.validate_platforms()?;
        let invalid = |field: &str, reason: &str| format!("{}: {field} {reason}", self.label());
        if let Some(argv) = &self.list_command {
            if self.runner != "command" || self.catch_on.is_some() {
                return Err(invalid(
                    "list_command",
                    "is supported only for named-test command rows",
                ));
            }
            if argv.first().is_none_or(|s| s.trim().is_empty())
                || argv.iter().any(|s| s.contains('\0'))
                || argv
                    .iter()
                    .map(|s| s.matches("{test}").count())
                    .sum::<usize>()
                    != 1
                || !argv.iter().any(|s| s == "{test}")
            {
                return Err(invalid("list_command", "requires a nonempty program, NUL-free argv and {test} exactly once as a complete argv element"));
            }
        }
        if self.select.is_some() {
            if !matches!(self.runner.as_str(), "cargo" | "nextest") {
                return Err(invalid(
                    "select",
                    "is supported only for cargo/nextest rows",
                ));
            }
            if self.select == Some(TestSelection::Expected)
                && (self.only || self.hub.is_some() || self.hub_targets.is_some())
            {
                return Err(invalid("select = expected", "cannot be combined with only = true, hub or hub_targets: breadth is not observed"));
            }
            if self.hub.is_some() || self.hub_targets.is_some() {
                return Err(invalid(
                    "select",
                    "cannot be combined with hub or hub_targets: breadth is not observed",
                ));
            }
            if self.expect_red.is_empty() {
                let field = if self.select == Some(TestSelection::Expected) {
                    "select = expected"
                } else {
                    "select"
                };
                return Err(invalid(field, "requires nonempty expect_red"));
            }
        }
        let broad_fields = [
            self.broad_command.is_some(),
            self.broad_report.is_some(),
            self.broad_id.is_some(),
        ];
        if broad_fields.iter().any(|present| *present) {
            if self.runner != "command" || broad_fields.iter().any(|present| !*present) {
                return Err(invalid(
                    "broad_command/broad_report/broad_id",
                    "are required together on command rows only",
                ));
            }
            let argv = self.broad_command.as_ref().unwrap();
            if argv.first().is_none_or(|s| s.trim().is_empty())
                || argv
                    .iter()
                    .any(|s| s.contains('\0') || s.contains("{test}"))
            {
                return Err(invalid(
                    "broad_command",
                    "requires a nonempty program, NUL-free argv and no {test}",
                ));
            }
            let template = self.broad_id.as_deref().unwrap();
            let literals = template
                .replace("{classname}", "")
                .replace("{name}", "")
                .replace("{file}", "")
                .replace("{suite}", "");
            if !template.contains("{name}")
                || literals.contains(['{', '}'])
                || template.chars().any(char::is_control)
            {
                return Err(invalid(
                    "broad_id",
                    "requires {name} and permits only {classname}, {name}, {file} and {suite} placeholders",
                ));
            }
            repository_relative(self.broad_report.as_deref().unwrap())
                .map_err(|e| invalid("broad_report", &e))?;
        }
        if self.junit_class.is_some() && self.broad_id.is_none() {
            return Err(invalid(
                "junit_class",
                "requires broad_command, broad_report and broad_id: only a JUnit breadth audit reads it",
            ));
        }
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
                    ("ignored", self.ignored.is_some()),
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
                        validate_command_id(id).map_err(|e| invalid("expect_red", &e))?;
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
                self.label()
            ));
        }
        let reason = self
            .hub
            .as_deref()
            .ok_or_else(|| format!("{}: HUB requires a hub reason", self.label()))?;
        if reason.trim().chars().count() < 20 {
            return Err(format!(
                "{}: HUB reason must be at least 20 characters after trimming",
                self.label()
            ));
        }
        if self.hub_targets.as_ref().is_none_or(|targets| {
            targets.is_empty() || targets.iter().any(|target| target.trim().is_empty())
        }) {
            return Err(format!(
                "{}: HUB requires non-empty hub_targets",
                self.label()
            ));
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
                self.label()
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
                self.label()
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
                self.label()
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
                        self.label(),
                        target
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
                self.label()
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
                    return Err(format!("{}: invalid features name {name:?}: must be nonempty without whitespace, control characters, leading '-' or commas", self.label()));
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

/// Resolve the default directory without concealing a half-finished migration.
pub fn catalogue_path(path: &Path) -> Result<PathBuf> {
    let (file, directory) = match path.file_name().and_then(|n| n.to_str()) {
        Some("mutations.toml") => (path.to_path_buf(), path.with_file_name("mutations.d")),
        Some("mutations.d") => (path.with_file_name("mutations.toml"), path.to_path_buf()),
        _ => return Ok(path.to_path_buf()),
    };
    if file.exists() && directory.is_dir() {
        return Err(format!(
            "both {} and {} exist; finish the catalogue migration before continuing",
            file.display(),
            directory.display()
        ));
    }
    Ok(if directory.is_dir() {
        directory
    } else {
        path.to_path_buf()
    })
}

fn source_file(path: &Path) -> String {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if let Ok(absolute) = fs::canonicalize(parent) {
        for ancestor in absolute.ancestors() {
            if let Ok(bytes) = git(ancestor, &["rev-parse", "--show-toplevel"]) {
                // Canonicalize git's answer too: on Windows it prints `C:/...` while
                // `canonicalize` gives `\\?\C:\...` (and expands short names such
                // as `RUNNER~1`), so the two only compare equal in the same form.
                let root = PathBuf::from(String::from_utf8_lossy(&bytes).trim());
                let root = fs::canonicalize(&root).unwrap_or(root);
                if let Some(name) = path.file_name() {
                    if let Ok(relative) = absolute.join(name).strip_prefix(&root) {
                        // Reports name files the same way on every OS.
                        let parts: Vec<_> = relative
                            .components()
                            .map(|part| part.as_os_str().to_string_lossy())
                            .collect();
                        return parts.join("/");
                    }
                }
            }
        }
    }
    path.to_string_lossy().into_owned()
}

fn fragment_paths(path: &Path) -> Result<Vec<PathBuf>> {
    if !path.is_dir() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", path.display()))?;
        let name = entry.file_name();
        if !name.as_encoded_bytes().starts_with(b".")
            && entry.path().extension().is_some_and(|e| e == "toml")
            && entry.path().is_file()
        {
            paths.push(entry.path());
        }
    }
    paths.sort_by(|a, b| {
        a.file_name()
            .unwrap()
            .as_encoded_bytes()
            .cmp(b.file_name().unwrap().as_encoded_bytes())
    });
    Ok(paths)
}

pub fn load(path: &Path) -> Result<Catalogue> {
    let path = catalogue_path(path)?;
    let fragments = fragment_paths(&path)?
        .into_iter()
        .map(|file| {
            let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            Ok((source_file(&file), text))
        })
        .collect::<Result<Vec<_>>>()?;
    load_fragments(fragments)
}

fn load_fragments(fragments: Vec<(String, String)>) -> Result<Catalogue> {
    // Deserialize each row separately so malformed fields still name its id.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Rows {
        #[serde(default)]
        prebuild: Vec<Prebuild>,
        #[serde(default)]
        control: Vec<toml::Value>,
    }
    #[derive(Deserialize)]
    struct Locations {
        #[serde(default)]
        control: Vec<IdLocation>,
    }
    #[derive(Deserialize)]
    struct IdLocation {
        id: Option<toml::Spanned<String>>,
    }
    let mut catalogue = Catalogue::default();
    let mut settings = BTreeMap::new();
    for (file, text) in fragments {
        let table: toml::Table = toml::from_str(&text).map_err(|e| format!("{file}: {e}"))?;
        for key in table.keys().filter(|key| key.as_str() != "control") {
            if let Some(previous) = settings.insert(key.clone(), file.clone()) {
                return Err(format!(
                    "top-level setting {key:?} defined in both {previous} and {file}"
                ));
            }
        }
        let locations: Locations = toml::from_str(&text).map_err(|e| format!("{file}: {e}"))?;
        let rows: Rows = toml::from_str(&text).map_err(|e| format!("{file}: {e}"))?;
        catalogue.prebuild.extend(rows.prebuild);
        for (row, location) in rows.control.into_iter().zip(locations.control) {
            let id = row
                .get("id")
                .and_then(toml::Value::as_str)
                .unwrap_or("<missing id>")
                .to_owned();
            let line = location.id.as_ref().map_or(1, |id| {
                text[..id.span().start]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    + 1
            });
            let mut c: Control = row
                .try_into()
                .map_err(|e| format!("{id} ({file}:{line}): {e}"))?;
            c.source = RowSource {
                file: file.clone(),
                line,
            };
            catalogue.control.push(c);
        }
    }
    unique_ids(&catalogue)?;
    for c in &catalogue.control {
        c.validate_runner()?;
        c.recorded_disposition()?;
    }
    validate_prebuild(&catalogue.prebuild)?;
    Ok(catalogue)
}

fn unique_ids(catalogue: &Catalogue) -> Result<()> {
    let mut ids: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for c in &catalogue.control {
        ids.entry(&c.id).or_default().push(c.source.location());
    }
    let duplicates: Vec<_> = ids
        .into_iter()
        .filter(|(_, sources)| sources.len() > 1)
        .map(|(id, sources)| format!("duplicate id {id:?}: {}", sources.join(", ")))
        .collect();
    if duplicates.is_empty() {
        Ok(())
    } else {
        Err(duplicates.join("\n"))
    }
}

fn target_io_error(name: &str, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("ANCHOR_MISSING {name}: {error}")
    } else {
        format!("{name}: {error}")
    }
}

fn safe_path(root: &Path, name: &str) -> Result<PathBuf> {
    let path = repository_relative(name)?;
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

fn repository_relative(name: &str) -> Result<&Path> {
    let path = Path::new(name);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("not a repository-relative path: {name}"));
    }
    Ok(path)
}

// Unlike edit targets, reports need not exist yet. Refuse symlinks in every
// component so neither stale-file deletion nor a new output can escape the tree.
fn report_path(root: &Path, name: &str) -> Result<PathBuf> {
    let mut full = root.canonicalize().map_err(|e| e.to_string())?;
    for component in repository_relative(name)?.components() {
        full.push(component);
        match fs::symlink_metadata(&full) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("symlink broad_report refused: {name}"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("broad_report {name}: {e}")),
        }
    }
    Ok(full)
}

fn validate_command_id(id: &str) -> Result<()> {
    if id.is_empty() || id.trim() != id || id.chars().any(char::is_control) {
        return Err(format!("invalid test id {id:?}: ids must be nonempty without leading/trailing whitespace or control characters (including NUL)"));
    }
    Ok(())
}

pub fn validate(root: &Path, catalogue: &Catalogue) -> Result<()> {
    validate_prebuild(&catalogue.prebuild)?;
    unique_ids(catalogue)?;
    for c in &catalogue.control {
        if c.id.is_empty()
            || !c
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!("invalid id: {}", c.label()));
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
            return Err(format!("{}: invalid required field", c.label()));
        }
        safe_path(root, &c.test_file).map_err(|e| format!("{}: {e}", c.label()))?;
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
    if let Some(name) = &c.broad_report {
        report_path(root, name).map_err(|e| format!("{}: {e}", c.label()))?;
    }
    for edit in c.edits()? {
        // Deleted edit targets are row outcomes, not catalogue-wide errors.
        // Check mode still rejects them when it verifies the anchors.
        match safe_path(root, &edit.file) {
            Ok(_) => {}
            Err(e) if e.starts_with("ANCHOR_MISSING ") => {}
            Err(e) => return Err(format!("{}: {e}", c.label())),
        }
        if edit.old.is_empty() || edit.old == edit.new {
            return Err(format!("{}: empty or unchanged edit", c.label()));
        }
    }
    Ok(())
}

/// Choose an id-named fragment so independent new rows do not share an append point.
pub fn control_destination(path: &Path, id: &str) -> Result<PathBuf> {
    let path = catalogue_path(path)?;
    if path.is_dir() {
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!("invalid id for catalogue fragment: {id}"));
        }
        Ok(path.join(format!("{id}.toml")))
    } else {
        Ok(path)
    }
}

pub fn control_source(path: &Path, id: &str) -> Result<RowSource> {
    Ok(RowSource {
        file: source_file(&control_destination(path, id)?),
        line: 0,
    })
}

/// Append a table to a file, or create an id-named fragment in a directory.
/// `prove` and `explore --append` both write rows through this function.
pub fn append_control(path: &Path, c: &Control) -> Result<()> {
    use std::io::Write;
    let catalogue_path = catalogue_path(path)?;
    let destination = control_destination(&catalogue_path, &c.id)?;
    // HUB records a person's review of an observed broad catch, not a discovery
    // that prove or explore may assign automatically.
    if c.hub.is_some() || c.hub_targets.is_some() {
        return Err(format!(
            "{}: cannot append HUB; review a broad catch and edit the catalogue manually",
            c.label()
        ));
    }
    if catalogue_path.exists() {
        let mut catalogue = load(&catalogue_path)?;
        catalogue.control.push(c.clone());
        unique_ids(&catalogue)?;
    }
    let row = toml::to_string(&Catalogue {
        control: vec![c.clone()],
        ..Catalogue::default()
    })
    .map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .create(!catalogue_path.is_dir())
        .create_new(catalogue_path.is_dir())
        .append(true)
        .open(&destination)
        .map_err(|e| format!("{}: {e}", destination.display()))?;
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
        // Hash native path bytes after resolving aliases so one worktree has
        // one lock, without needing write access to Git's metadata directory.
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
        let path = std::env::temp_dir().join(format!("ck-mutate-{digest:x}.lock"));
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
            // Git applies line-ending and filter conversions when checking both
            // index and worktree changes. Optional locks stay off for read-only Git metadata.
            if !allow_dirty
                && !git(
                    root,
                    &[
                        "--no-optional-locks",
                        "--literal-pathspecs",
                        "status",
                        "--porcelain=v1",
                        "-z",
                        "--",
                        &e.file,
                    ],
                )?
                .is_empty()
            {
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
struct Replacements {
    files: BTreeMap<PathBuf, String>,
    used_crlf: bool,
}

fn anchor_form(text: &str, anchor: &str) -> (String, bool) {
    if text.contains(anchor)
        || !text.contains("\r\n")
        || !anchor.contains('\n')
        || anchor.contains('\r')
    {
        (anchor.to_owned(), false)
    } else {
        (anchor.replace('\n', "\r\n"), true)
    }
}

fn region(text: &str, file: &str, scope: &AnchorScope) -> Result<(usize, usize, bool)> {
    let (after, after_crlf) = anchor_form(text, &scope.after);
    let after_matches = text.match_indices(&after).collect::<Vec<_>>();
    if after_matches.is_empty() {
        return Err(format!(
            "ANCHOR_SCOPE_AFTER_MISSING {file}: scope.after did not match"
        ));
    }
    if after_matches.len() != 1 {
        return Err(format!(
            "ANCHOR_SCOPE_AFTER_AMBIGUOUS {file}: scope.after matched {} occurrences",
            after_matches.len()
        ));
    }
    let start = after_matches[0].0 + after.len();
    let mut used_crlf = after_crlf;
    let end = if let Some(before) = &scope.before {
        let (mut before_form, mut before_crlf) = (before.clone(), false);
        if !text[start..].contains(&before_form)
            && text.contains("\r\n")
            && before.contains('\n')
            && !before.contains('\r')
        {
            before_form = before.replace('\n', "\r\n");
            before_crlf = true;
        }
        let Some(offset) = text[start..].find(&before_form) else {
            return Err(format!(
                "ANCHOR_SCOPE_BEFORE_MISSING {file}: scope.before did not occur after scope.after"
            ));
        };
        used_crlf |= before_crlf;
        start + offset
    } else {
        text.len()
    };
    Ok((start, end, used_crlf))
}

fn replaced_scoped(
    root: &Path,
    edits: &[Edit],
    scope: Option<&AnchorScope>,
) -> Result<Replacements> {
    // A row has one scope, and its boundaries are text inside one file, so a
    // scoped row whose edits touch several files has no single meaning.
    if scope.is_some() {
        let mut names = edits.iter().map(|edit| edit.file.as_str());
        if let Some(first) = names.next() {
            if let Some(other) = names.find(|name| *name != first) {
                return Err(format!(
                    "ANCHOR_SCOPE_MULTI_FILE {first}: scope applies to one file, but the row also edits {other}"
                ));
            }
        }
    }
    let mut files = BTreeMap::new();
    let mut used_crlf = false;
    for edit in edits {
        let path = safe_path(root, &edit.file)?;
        if !files.contains_key(&path) {
            files.insert(
                path.clone(),
                fs::read_to_string(&path).map_err(|e| e.to_string())?,
            );
        }
        let text = files.get_mut(&path).expect("inserted file");
        let (region_start, region_end, region_crlf) = match scope {
            Some(scope) => region(text, &edit.file, scope)?,
            None => (0, text.len(), false),
        };
        used_crlf |= region_crlf;
        let search = &text[region_start..region_end];
        let mut old = edit.old.clone();
        let mut new = edit.new.clone();
        let mut positions = search.match_indices(&old).collect::<Vec<_>>();
        // Git may check out LF-authored sources as CRLF. Prefer byte-exact
        // anchors, even in mixed-ending files; never combine match counts.
        let mut old_crlf = false;
        if positions.is_empty()
            && text.contains("\r\n")
            && old.contains('\n')
            && !old.contains('\r')
        {
            old = old.replace('\n', "\r\n");
            new = new.replace('\n', "\r\n");
            positions = search.match_indices(&old).collect();
            old_crlf = true;
        }
        if positions.len() != 1 {
            if scope.is_some() {
                let reason = if positions.is_empty() {
                    "ANCHOR_SCOPE_OLD_MISSING"
                } else {
                    "ANCHOR_SCOPE_OLD_AMBIGUOUS"
                };
                return Err(format!(
                    "{reason} {}: old anchor matched {} occurrences inside scope",
                    edit.file,
                    positions.len()
                ));
            } else {
                return Err(format!(
                    "ANCHOR_MISSING {}: replaced {} occurrences",
                    edit.file,
                    positions.len()
                ));
            }
        }
        used_crlf |= old_crlf;
        let start = region_start + positions[0].0;
        text.replace_range(start..start + old.len(), &new);
    }
    Ok(Replacements { files, used_crlf })
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

    /// The outcome as reports spell it, such as `TIMED_OUT`.
    pub fn name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
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
    pub source: RowSource,
    pub outcome: Outcome,
    /// The row's optional text boundaries, retained as evidence for the edit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<AnchorScope>,
    /// "crlf" when an old anchor or scope boundary used the LF-to-CRLF fallback.
    /// Absent when every anchor matched byte-exactly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_endings: Option<String>,
    /// The phase whose deadline expired, including command-row ERROR timeouts.
    pub timed_out_phase: Option<Phase>,
    pub red: Vec<String>,
    pub green: Vec<String>,
    /// Each red test's own mutant output, bounded to 16 KiB per entry.
    pub failures: BTreeMap<String, String>,
    /// Each baseline-red test's own output, separate from mutant failures.
    pub baseline_failures: BTreeMap<String, String>,
    /// What ran and how it ended when the clean-tree baseline was not green
    /// and the row is ERROR for that reason. Absent when the baseline was green.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_failure: Option<BaselineFailure>,
    /// The latest clean-tree run that exited non-green even though the baseline
    /// may still pass (Cargo baseline-red tests outside `expect_red` are
    /// tolerated). Promoted to `baseline_failure` only if a later baseline
    /// check rejects the row.
    #[serde(skip)]
    baseline_run: Option<BaselineFailure>,
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
    pub fn label(&self) -> String {
        format!("{} ({})", self.id, self.source.location())
    }

    fn new(c: &Control) -> Self {
        Self {
            id: c.id.clone(),
            source: c.source.clone(),
            outcome: Outcome::Error,
            scope: c.scope.clone(),
            line_endings: None,
            timed_out_phase: None,
            red: vec![],
            green: vec![],
            failures: BTreeMap::new(),
            baseline_failures: BTreeMap::new(),
            baseline_failure: None,
            baseline_run: None,
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

/// A clean-tree run that did not finish green: the command, how it ended and
/// the end of its output. Environment variables are never recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineFailure {
    /// Program and arguments exactly as spawned. Every runner starts the
    /// process in the repository root, so no working directory is recorded.
    pub command: Vec<String>,
    /// `exit N`, the signal that killed the process, or the expired timeout.
    pub exit: String,
    /// The last [`BASELINE_TAIL_LINES`] lines of output, at most
    /// [`BASELINE_TAIL_BYTES`] bytes. Streams captured together keep their
    /// write order; streams captured apart are labelled `--- stdout ---` and
    /// `--- stderr ---`.
    pub output_tail: String,
    /// True when earlier output was dropped to fit those bounds.
    pub truncated: bool,
}

pub const BASELINE_TAIL_LINES: usize = 40;
pub const BASELINE_TAIL_BYTES: usize = 16 * 1024;

impl BaselineFailure {
    fn from_output(output: &Output) -> Self {
        let exit = if output.timeout {
            format!("timed out: killed after timeout_s = {}", output.timeout_s)
        } else if let Some(signal) = &output.signal {
            format!("killed by {signal}")
        } else if let Some(code) = output.code {
            format!("exit {code}")
        } else {
            "no exit status".into()
        };
        let (output_tail, truncated) = match &output.stderr {
            None => {
                let (tail, truncated) =
                    output_tail(&output.stdout, BASELINE_TAIL_LINES, BASELINE_TAIL_BYTES);
                (tail.to_owned(), truncated)
            }
            Some(stderr) => labelled_tails(&output.stdout, stderr),
        };
        Self {
            command: output.argv.clone(),
            exit,
            output_tail,
            truncated,
        }
    }
}

/// The end of `text`: at most `lines` lines and `bytes` bytes (cut at a
/// character boundary), plus whether anything before it was dropped.
fn output_tail(text: &str, lines: usize, bytes: usize) -> (&str, bool) {
    let kept: usize = text
        .split_inclusive('\n')
        .rev()
        .take(lines)
        .map(str::len)
        .sum();
    let mut start = text.len() - kept.min(bytes);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    (&text[start..], start > 0)
}

/// Tails of separately captured stdout and stderr under one shared line and
/// byte budget. Each stream gets half, and a stream that needs less than its
/// half leaves the rest to the other. The labels count toward the byte cap.
fn labelled_tails(stdout: &str, stderr: &str) -> (String, bool) {
    const LABELS: usize = "--- stdout (earlier output dropped) ---\n".len()
        + "--- stderr (earlier output dropped) ---\n".len()
        + 2;
    let line_count = |text: &str| text.split_inclusive('\n').count();
    let bytes = BASELINE_TAIL_BYTES - LABELS;
    let err_lines = line_count(stderr)
        .min((BASELINE_TAIL_LINES / 2).max(BASELINE_TAIL_LINES.saturating_sub(line_count(stdout))));
    let err_bytes = stderr
        .len()
        .min((bytes / 2).max(bytes.saturating_sub(stdout.len())));
    let mut rendered = String::new();
    let mut truncated = false;
    for (name, text, lines, bytes) in [
        (
            "stdout",
            stdout,
            BASELINE_TAIL_LINES - err_lines,
            bytes - err_bytes,
        ),
        ("stderr", stderr, err_lines, err_bytes),
    ] {
        let (tail, dropped) = output_tail(text, lines, bytes);
        truncated |= dropped;
        let note = if dropped {
            " (earlier output dropped)"
        } else {
            ""
        };
        rendered.push_str(&format!("--- {name}{note} ---\n{tail}"));
        if !tail.is_empty() && !tail.ends_with('\n') {
            rendered.push('\n');
        }
    }
    (rendered, truncated)
}

#[cfg(test)]
mod baseline_tail_tests {
    use super::*;

    #[test]
    fn tail_keeps_the_last_lines_and_reports_only_a_real_drop() {
        let forty: String = (1..=40).map(|i| format!("{i}\n")).collect();
        assert_eq!(output_tail(&forty, 40, 1024), (forty.as_str(), false));
        let more = format!("0\n{forty}");
        assert_eq!(output_tail(&more, 40, 1024), (forty.as_str(), true));
        // An unterminated last line still counts as one line.
        assert_eq!(output_tail("a\nb\nc", 2, 1024), ("b\nc", true));
        assert_eq!(output_tail("", 40, 1024), ("", false));
    }

    #[test]
    fn tail_byte_cap_keeps_the_end_on_a_character_boundary() {
        assert_eq!(output_tail("abcdef", 40, 3), ("def", true));
        // Cutting three bytes from the end would split the two-byte "é".
        assert_eq!(output_tail("aéz\n", 40, 3), ("z\n", true));
    }

    #[test]
    fn labelled_tails_share_one_budget_and_stay_under_the_cap() {
        assert_eq!(
            labelled_tails("out", "err\n"),
            (
                "--- stdout ---\nout\n--- stderr ---\nerr\n".to_owned(),
                false
            )
        );
        let many: String = (0..100).map(|i| format!("{i}\n")).collect();
        let (rendered, truncated) = labelled_tails(&many, "only\n");
        assert!(truncated);
        // One stderr line leaves the other 39 lines to stdout.
        assert_eq!(rendered.lines().count(), 2 + 39 + 1, "{rendered}");
        assert!(rendered.starts_with("--- stdout (earlier output dropped) ---\n61\n"));
        let wide = "x".repeat(20 * 1024);
        let (rendered, truncated) = labelled_tails(&wide, &wide);
        assert!(truncated);
        assert!(rendered.len() <= BASELINE_TAIL_BYTES, "{}", rendered.len());
        assert!(rendered.contains("--- stderr (earlier output dropped) ---\n"));
    }
}

struct Output {
    /// Program and arguments as spawned; never the environment.
    argv: Vec<String>,
    success: bool,
    code: Option<i32>,
    signal: Option<String>,
    timeout: bool,
    /// The deadline the run was given, named when it expired.
    timeout_s: u64,
    interrupted: bool,
    /// Processes under the command that were still alive when its deadline
    /// expired or the run was interrupted, as `program args (pid N)`. Empty
    /// when the command exited on its own.
    running_at_deadline: Vec<String>,
    text: String,
    stdout: String,
    /// Present only when stderr was captured apart from stdout; otherwise both
    /// streams share `stdout` in the order they were written.
    stderr: Option<String>,
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
    // Selected rows never widen their target or named-test selection,
    // including when a caller asks for an expensive breadth pass.
    let scope = if c.select.is_some() {
        Scope::Row
    } else {
        scope
    };
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
            // A build selects no tests, so `ignored` reaches only list and run
            // modes; baseline sharing adds it to its key separately.
            cmd.arg("--no-run");
        }
        "list" if c.runner == "cargo" => {
            cmd.args(["--", "--list"]);
            cmd.args(c.ignored.map(IgnoredSelection::libtest_arg));
        }
        "list" => {
            cmd.args(["--message-format", "json"]);
            if let Some(ignored) = c.ignored {
                cmd.args(["--run-ignored", ignored.nextest_value()]);
            }
        }
        _ if c.runner == "cargo" => {
            cmd.args(["--no-fail-fast", "--", "--test-threads=1"]);
            cmd.args(c.ignored.map(IgnoredSelection::libtest_arg));
            if let Some(selection) = &c.select {
                if let Some(names) = selection.exact_names(&c.expect_red) {
                    cmd.arg("--exact").args(names);
                } else if let TestSelection::Filter(filter) = selection {
                    // End harness options so a leading '-' remains filter text.
                    cmd.arg("--").arg(filter);
                }
            }
        }
        _ => {
            cmd.args(["--no-fail-fast", "--retries", "0"]);
            if let Some(ignored) = c.ignored {
                cmd.args(["--run-ignored", ignored.nextest_value()]);
            }
            if let Some(selection) = &c.select {
                let escape = |name: &str| {
                    let mut escaped = String::new();
                    for ch in name.chars() {
                        match ch {
                            '\\' | '/' | ')' | ',' => {
                                escaped.push('\\');
                                escaped.push(ch);
                            }
                            '\n' => escaped.push_str("\\n"),
                            '\r' => escaped.push_str("\\r"),
                            '\t' => escaped.push_str("\\t"),
                            c if c.is_control() => {
                                escaped.push_str(&format!("\\u{{{:X}}}", c as u32))
                            }
                            _ => escaped.push(ch),
                        }
                    }
                    escaped
                };
                let expression = if let Some(names) = selection.exact_names(&c.expect_red) {
                    names
                        .iter()
                        .map(|name| format!("test(={})", escape(name)))
                        .collect::<Vec<_>>()
                        .join(" | ")
                } else if let TestSelection::Filter(filter) = selection {
                    format!("test(~{})", escape(filter))
                } else {
                    unreachable!()
                };
                cmd.args(["-E", &expression]);
            }
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
    // Keep captures alive until all output has been read. RAII also removes
    // them on spawn, wait or read errors, without writing into Git metadata.
    let dir = tempfile::Builder::new()
        .prefix("ck-mutate-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let stdout = dir.path().join("stdout");
    let out_file = File::create(&stdout).map_err(|e| e.to_string())?;
    // Cargo prints binary headers on stderr and test events on stdout. Sharing
    // the file offset preserves their order, so events keep their binary identity.
    let stderr = dir.path().join("stderr");
    let err_file = if separate_stdout {
        File::create(&stderr)
    } else {
        out_file.try_clone()
    }
    .map_err(|e| e.to_string())?;
    let argv = std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
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
    let child = cmd.spawn();
    // Close the command's capture handles before cleanup, including when
    // spawning fails (Windows cannot remove files with live handles).
    drop(cmd);
    let mut child = RunningChild {
        child: child.map_err(|e| e.to_string())?,
        exited: false,
    };
    let mut timed_out = false;
    let mut interrupted = false;
    let mut running_at_deadline = vec![];
    let status = loop {
        if stop.load(Ordering::SeqCst) || start.elapsed() >= Duration::from_secs(timeout) {
            interrupted = stop.load(Ordering::SeqCst);
            timed_out = !interrupted;
            running_at_deadline = terminate_tree(&mut child.child);
            child.exited = true;
            break None;
        }
        if let Some(status) = child.child.try_wait().map_err(|e| e.to_string())? {
            child.exited = true;
            break Some(status);
        }
        thread::sleep(Duration::from_millis(20));
    };
    drop(child);
    // Test tools do not always write UTF-8 (Python on Windows writes the
    // console code page when redirected to a file). Decode lossily: an output
    // byte the runner cannot decode must not turn a finished test run into a
    // spawn error. Grades never depend on decoding; they come from exit
    // status or from test lines, which still match exactly when valid.
    let read_text = |path: &Path| {
        fs::read(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|e| e.to_string())
    };
    let stdout_text = read_text(&stdout)?;
    let stderr_text = if separate_stdout {
        Some(read_text(&stderr)?)
    } else {
        None
    };
    let text = match &stderr_text {
        Some(stderr_text) => format!("{stdout_text}{stderr_text}"),
        None => stdout_text.clone(),
    };
    Ok(Output {
        argv,
        success: status.is_some_and(|s| s.success()),
        code: status.and_then(|s| s.code()),
        signal: status.and_then(exit_signal),
        timeout: timed_out,
        timeout_s: timeout,
        interrupted,
        running_at_deadline,
        text,
        stdout: stdout_text,
        stderr: stderr_text,
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
/// A spawned command. Dropped before the command exited (an error while
/// waiting for it), it stops the command's whole process tree; dropped after
/// an exit, it only kills what is left in the command's own process group.
struct RunningChild {
    child: std::process::Child,
    /// True once the command exited on its own or `terminate_tree` reaped it.
    exited: bool,
}
impl Drop for RunningChild {
    fn drop(&mut self) {
        if self.exited {
            kill_tree(&mut self.child);
            let _ = self.child.wait();
        } else {
            terminate_tree(&mut self.child);
        }
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

/// How long a timed-out or interrupted command's processes get to exit after
/// SIGTERM before they are sent SIGKILL. A test runner such as nextest uses
/// this time to stop the tests it started.
#[cfg(unix)]
const TERMINATE_GRACE: Duration = Duration::from_secs(2);

/// Stop a command that is still running, and every process it started, then
/// reap it. Returns the processes under the command that were alive when this
/// was called, as `program args (pid N)`.
///
/// The command runs in its own process group, but signalling only that group
/// is not enough: nextest starts each test in a process group of its own, and
/// a test may do the same for helpers it spawns. Those processes would outlive
/// the deadline and keep running while later rows replay. So the process tree
/// under the command is read before any signal is sent (a process whose parent
/// dies is re-parented and can no longer be found from the command), and every
/// process in it, and its process group, gets SIGTERM, then SIGKILL after
/// [`TERMINATE_GRACE`]. The tree is read again while waiting, so processes
/// started in the meantime are stopped too. A process that left the tree
/// before the deadline, such as a double-forked daemon, is out of reach.
#[cfg(unix)]
fn terminate_tree(child: &mut std::process::Child) -> Vec<String> {
    use rustix::process::Signal;
    let root = child.id();
    let mut tracked = BTreeMap::new();
    track_tree(root, &mut tracked);
    let running = tracked
        .iter()
        .filter(|(pid, _)| is_running(**pid))
        .map(|(pid, process)| format!("{} (pid {pid})", short_command(&process.command)))
        .collect();
    signal_tree(root, &tracked, Signal::TERM);
    let grace = Instant::now();
    loop {
        let exited = matches!(child.try_wait(), Ok(Some(_)));
        let known = tracked.len();
        track_tree(root, &mut tracked);
        if tracked.len() > known {
            signal_tree(root, &tracked, Signal::TERM);
        }
        if (exited && !tracked.keys().any(|pid| is_running(*pid)))
            || grace.elapsed() >= TERMINATE_GRACE
        {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    track_tree(root, &mut tracked);
    signal_tree(root, &tracked, Signal::KILL);
    let _ = child.kill();
    let _ = child.wait();
    // Delivery of SIGKILL is asynchronous. Wait briefly until the killed
    // processes are gone, so the next command never runs alongside them.
    let reap = Instant::now();
    while reap.elapsed() < Duration::from_secs(1) && tracked.keys().any(|pid| is_running(*pid)) {
        thread::sleep(Duration::from_millis(20));
    }
    running
}

#[cfg(not(unix))]
fn terminate_tree(child: &mut std::process::Child) -> Vec<String> {
    // `taskkill /T` walks the process tree from the command itself.
    kill_tree(child);
    let _ = child.wait();
    vec![]
}

/// A process found under a command that is being stopped.
#[cfg(unix)]
struct TreeProcess {
    /// Its process group when found, or 0 when that could not be read.
    pgid: u32,
    /// Program and arguments, or the short process name when those are unreadable.
    command: String,
}

/// Add to `tracked` every process whose chain of parents reaches `root` or a
/// process tracked earlier. The second case keeps the children of a tracked
/// process whose own parent already died and was replaced by init.
#[cfg(unix)]
fn track_tree(root: u32, tracked: &mut BTreeMap<u32, TreeProcess>) {
    let mut parents: BTreeSet<u32> = tracked.keys().copied().collect();
    parents.insert(root);
    loop {
        let found: Vec<(u32, String)> = children_of(&parents)
            .into_iter()
            .filter(|(pid, _)| *pid != root && !tracked.contains_key(pid))
            .collect();
        if found.is_empty() {
            return;
        }
        for (pid, command) in found {
            let pgid = process_group(Some(pid));
            tracked.insert(pid, TreeProcess { pgid, command });
            parents.insert(pid);
        }
    }
}

/// Send `signal` to the command's process group, every process group seen in
/// its tree, and every tracked process. Never signals this runner's own group
/// or init, whatever a descendant's group is.
#[cfg(unix)]
fn signal_tree(root: u32, tracked: &BTreeMap<u32, TreeProcess>, signal: rustix::process::Signal) {
    use rustix::process::{kill_process, kill_process_group, Pid};
    let own_group = process_group(None);
    let mut groups: BTreeSet<u32> = tracked.values().map(|process| process.pgid).collect();
    groups.insert(root);
    for group in groups {
        if group > 1 && group != own_group {
            if let Some(group) = Pid::from_raw(group as i32) {
                let _ = kill_process_group(group, signal);
            }
        }
    }
    for &pid in tracked.keys() {
        if pid > 1 {
            if let Some(pid) = Pid::from_raw(pid as i32) {
                let _ = kill_process(pid, signal);
            }
        }
    }
}

/// The live processes whose parent is in `parents`, with their commands, read
/// from `/proc` so that Linux needs no extra program.
#[cfg(target_os = "linux")]
fn children_of(parents: &BTreeSet<u32>) -> Vec<(u32, String)> {
    let Ok(dir) = fs::read_dir("/proc") else {
        return vec![];
    };
    let mut children = vec![];
    for entry in dir.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Some(stat) = proc_stat(&pid.to_string()) else {
            continue;
        };
        if !parents.contains(&stat.ppid) || stat.state == 'Z' || stat.state == 'X' {
            continue;
        }
        let argv = fs::read(entry.path().join("cmdline")).unwrap_or_default();
        let command = argv
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(String::from_utf8_lossy)
            .collect::<Vec<_>>()
            .join(" ");
        children.push((
            pid,
            if command.is_empty() {
                stat.name
            } else {
                command
            },
        ));
    }
    children
}

/// The fields of `/proc/<pid>/stat` this runner reads.
#[cfg(target_os = "linux")]
struct ProcStat {
    name: String,
    state: char,
    ppid: u32,
    pgid: u32,
}

/// Read `/proc/<pid>/stat`, where `pid` is a number or `self`.
#[cfg(target_os = "linux")]
fn proc_stat(pid: &str) -> Option<ProcStat> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (name) state ppid pgrp ...`; the name may hold spaces or ')'.
    let (open, close) = (stat.find('(')?, stat.rfind(')')?);
    let mut fields = stat[close + 1..].split_whitespace();
    let state = fields.next()?.chars().next()?;
    let ppid = fields.next()?.parse().ok()?;
    let pgid = fields.next()?.parse().ok()?;
    Some(ProcStat {
        name: stat[open + 1..close].to_owned(),
        state,
        ppid,
        pgid,
    })
}

/// The process group of `pid`, or of this process for `None`; 0 when unknown.
/// Read from `/proc` rather than with getpgid(2): inside a PID namespace
/// whose process group leader lives outside it (as in some containers) the
/// group id is 0, which rustix's `getpgrp` treats as impossible.
#[cfg(target_os = "linux")]
fn process_group(pid: Option<u32>) -> u32 {
    let pid = pid.map_or_else(|| "self".to_owned(), |pid| pid.to_string());
    proc_stat(&pid).map_or(0, |stat| stat.pgid)
}

/// True while `pid` exists and has not exited; a zombie counts as exited.
#[cfg(target_os = "linux")]
fn is_running(pid: u32) -> bool {
    proc_stat(&pid.to_string()).is_some_and(|stat| stat.state != 'Z' && stat.state != 'X')
}

/// The processes whose parent is in `parents`, with their commands, from
/// `pgrep`. `ps` would also work, but it is setuid on macOS, and sandboxes
/// that forbid running setuid programs (as agent sandboxes do) refuse it.
/// Empty when `pgrep` cannot run, which leaves only the command's own process
/// group to stop.
#[cfg(all(unix, not(target_os = "linux")))]
fn children_of(parents: &BTreeSet<u32>) -> Vec<(u32, String)> {
    let list = parents
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let Ok(output) = Command::new("pgrep")
        .args(["-l", "-f", "-P", &list])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return vec![];
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
            Some((pid.parse().ok()?, command.to_owned()))
        })
        .collect()
}

/// True while `pid` exists. A zombie also counts, but the processes waited on
/// here are reaped promptly by their parent or, once orphaned, by init.
#[cfg(all(unix, not(target_os = "linux")))]
fn is_running(pid: u32) -> bool {
    use rustix::process::{test_kill_process, Pid};
    Pid::from_raw(pid as i32).is_some_and(|pid| test_kill_process(pid).is_ok())
}

/// The process group of `pid`, or of this process for `None`; 0 when unknown.
#[cfg(all(unix, not(target_os = "linux")))]
fn process_group(pid: Option<u32>) -> u32 {
    use rustix::process::{getpgid, Pid};
    let group = match pid {
        Some(pid) => Pid::from_raw(pid as i32).and_then(|pid| getpgid(Some(pid)).ok()),
        None => getpgid(None).ok(),
    };
    group.map_or(0, |group| Pid::as_raw(Some(group)) as u32)
}

/// `program args` without the program's directory, at most 120 characters.
#[cfg(unix)]
fn short_command(command: &str) -> String {
    let (program, args) = command.split_once(' ').unwrap_or((command, ""));
    let program = program.rsplit('/').next().unwrap_or(program);
    let text = if args.is_empty() {
        program.to_owned()
    } else {
        format!("{program} {args}")
    };
    match text.char_indices().nth(120) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

/// What a command was doing when its deadline expired, appended to a
/// timeout reason: for a Cargo test run, the test target it started last;
/// for any command, the processes under it that were still running (at most
/// eight). Empty when neither is known.
fn deadline_note(output: &Output, cargo_test_run: bool) -> String {
    let mut parts = vec![];
    if cargo_test_run {
        // Cargo prints `Running <target> (<binary>)` before each test binary.
        if let Some(target) = output
            .text
            .lines()
            .rev()
            .find_map(|line| line.trim().strip_prefix("Running "))
        {
            parts.push(format!("last test target started: {target}"));
        }
    }
    let running = &output.running_at_deadline;
    if !running.is_empty() {
        let mut shown = running.iter().take(8).cloned().collect::<Vec<_>>();
        if running.len() > 8 {
            shown.push(format!("{} more", running.len() - 8));
        }
        parts.push(format!("still running: {}", shown.join(", ")));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join("; "))
    }
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

fn named_command(argv: &[String], id: &str) -> Command {
    let args: Vec<_> = argv
        .iter()
        .map(|arg| if arg == "{test}" { id } else { arg })
        .collect();
    let mut cmd = Command::new(args[0]);
    cmd.args(&args[1..]);
    cmd
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
        let error = |reason: String| format!("{}: {id}: {label}: {reason}", c.label());
        let cmd = named_command(argv, id);
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
        // Every non-green clean run below ends the row as ERROR, so record what
        // ran before any of those returns.
        if baseline && !output.success {
            report.baseline_failure = Some(BaselineFailure::from_output(&output));
        }
        if output.timeout {
            report.timed_out_phase = Some(Phase::Test);
            return Err(error(format!(
                "test command exceeded timeout_s = {}{}",
                c.timeout_s,
                deadline_note(&output, false)
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

fn command_breadth(
    root: &Path,
    c: &Control,
    stop: &AtomicBool,
    report: &mut Report,
    baseline: bool,
) -> Result<()> {
    let name = c
        .broad_report
        .as_deref()
        .ok_or("broad_report is required")?;
    let audit = (|| -> Result<()> {
        let path = report_path(root, name)?;
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot delete stale report: {e}")),
        }
        let argv = c
            .broad_command
            .as_ref()
            .ok_or("broad_command is required")?;
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        let output = execute(root, cmd, c.timeout_s, stop)?;
        if baseline {
            report.baseline_run = (!output.success && !output.interrupted)
                .then(|| BaselineFailure::from_output(&output));
        }
        if baseline {
            report.baseline_test_ms += output.ms;
        } else {
            report.test_ms += output.ms;
        }
        report.test_tail = tail(&format!(
            "{}\nbreadth output:\n{}",
            report.test_tail, output.text
        ));
        if output.timeout {
            report.timed_out_phase = Some(Phase::Test);
            return Err(format!(
                "broad command exceeded timeout_s = {}{}",
                c.timeout_s,
                deadline_note(&output, false)
            ));
        }
        if output.interrupted {
            return Err("broad command interrupted".into());
        }
        if output.code.is_none() || matches!(output.code, Some(126 | 127)) {
            return Err(format!(
                "broad command failed to run: exit {:?}, signal {:?}",
                output.code, output.signal
            ));
        }
        // Recheck after execution: a report writer must not replace an output
        // directory or file with a link to somewhere outside the repository.
        let path = report_path(root, name)?;
        let xml = fs::read_to_string(&path).map_err(|e| format!("cannot read report: {e}"))?;
        let mut results = junit_results(
            &xml,
            c.broad_id.as_deref().ok_or("broad_id is required")?,
            c.junit_class.unwrap_or_default(),
        )?;
        if output.code != Some(0) && results.red.is_empty() {
            return Err(format!(
                "broad command exited {:?} without failed testcases",
                output.code
            ));
        }
        if baseline {
            report.baseline_failures.extend(results.failures());
            for id in &results.red {
                report
                    .baseline_red
                    .entry(results.targets[id].clone())
                    .or_default()
                    .push(id.clone());
            }
            report.baseline_results = Some(results);
            return Ok(());
        }
        validate_command_broad_ids(c, &results)?;
        // Keep diagnostics for every failure under the mutant, including tests
        // that already failed on the unmutated tree. Only tests that passed
        // before the mutation and fail after it count as the mutant's collateral.
        for (id, output) in results.failures() {
            if !c.expect_red.contains(&id) {
                report.failures.insert(id, output);
            }
        }
        exclude_baseline_red(c, report, &mut results);
        let (extra, cross_targets) = collateral(c, &results)?;
        report.collateral = extra;
        for id in results.red {
            if !c.expect_red.contains(&id) {
                report.unexpected_red.insert(id.clone());
                report.red.push(id);
            }
        }
        report.green.extend(
            results
                .green
                .into_iter()
                .filter(|id| !c.expect_red.contains(id)),
        );
        report.breadth_observed = true;
        report.outcome = breadth_outcome(c, report, &cross_targets);
        Ok(())
    })();
    if audit.is_err() && baseline {
        report.baseline_failure = report.baseline_run.clone();
    }
    audit.map_err(|e| format!("{}: broad_report {name}: {e}", c.label()))
}

fn validate_command_broad_ids(c: &Control, results: &TestResults) -> Result<()> {
    for id in &c.expect_red {
        if !results.targets.contains_key(id) {
            return Err(format!(
                "expected test id {id:?} missing from report; check broad_id"
            ));
        }
    }
    Ok(())
}

/// Rejects a package-wide JUnit baseline that does not show every expected id
/// green, recording the clean run that produced it on the row when it failed.
fn validate_command_broad_baseline(c: &Control, report: &mut Report) -> Result<()> {
    let validation = command_broad_baseline_green(c, report);
    if validation.is_err() {
        report.baseline_failure = report.baseline_run.clone();
    }
    validation
}

fn command_broad_baseline_green(c: &Control, report: &Report) -> Result<()> {
    if let Some(results) = &report.baseline_results {
        validate_command_broad_ids(c, results)?;
        for id in &c.expect_red {
            if !results.green.contains(id) {
                return Err(format!(
                    "{}: broad_report {}: baseline test {id:?} was not green",
                    c.label(),
                    c.broad_report.as_deref().unwrap_or_default()
                ));
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct JunitNode {
    tag: String,
    attributes: BTreeMap<String, String>,
    children: Vec<JunitNode>,
    text: String,
}

// A real XML reader handles quoted attributes, entities, CDATA and nested suites.
// Build a small tree so only direct failure/error children mark a testcase red.
fn junit_document(xml: &str) -> Result<JunitNode> {
    use quick_xml::{events::Event, Reader, XmlVersion};
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    reader.config_mut().check_comments = true;
    let mut stack: Vec<JunitNode> = Vec::new();
    let mut document = None;
    let mut version = XmlVersion::default();
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(element) => {
                if stack.len() >= 128 || (stack.is_empty() && document.is_some()) {
                    return Err("invalid XML document structure".into());
                }
                let mut node = JunitNode {
                    tag: element.name().as_ref().to_owned(),
                    ..JunitNode::default()
                };
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|e| e.to_string())?;
                    let key = attribute.key.as_ref().to_owned();
                    let value = attribute
                        .normalized_value(version)
                        .map_err(|e| e.to_string())?;
                    node.attributes.insert(key, value.into_owned());
                }
                stack.push(node);
            }
            Event::End(_) => {
                let node = stack.pop().ok_or("unexpected closing tag")?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    document = Some(node);
                }
            }
            Event::Text(text) => {
                let text = text.xml_content(version);
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&text);
                } else if !text.trim().is_empty() {
                    return Err("text outside XML root".into());
                }
            }
            Event::CData(text) => {
                stack
                    .last_mut()
                    .ok_or("CDATA outside XML root")?
                    .text
                    .push_str(&text.xml_content(version));
            }
            Event::GeneralRef(reference) => {
                let entity = format!("&{};", reference.as_ref());
                let text = quick_xml::escape::unescape(&entity).map_err(|e| e.to_string())?;
                stack
                    .last_mut()
                    .ok_or("entity outside XML root")?
                    .text
                    .push_str(&text);
            }
            Event::DocType(_) => return Err("JUnit report must not contain a DOCTYPE".into()),
            Event::Decl(declaration) => {
                if !stack.is_empty() || document.is_some() {
                    return Err("XML declaration after root".into());
                }
                version = declaration.xml_version().map_err(|e| e.to_string())?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("unclosed XML element".into());
    }
    let document = document.ok_or("empty XML document")?;
    if !matches!(document.tag.as_str(), "testsuites" | "testsuite") {
        return Err("JUnit root must be testsuites or testsuite".into());
    }
    Ok(document)
}

/// Expands a `broad_id` template in one left-to-right pass. Only the template
/// is scanned for placeholders; substituted attribute values are copied as they
/// are, so a literal `{name}` or `{suite}` inside a test or describe name
/// survives byte-for-byte.
fn expand_broad_id(template: &str, values: &[(&str, &str)]) -> String {
    let mut id = String::new();
    let mut rest = template;
    'scan: while let Some(c) = rest.chars().next() {
        for (placeholder, value) in values {
            if let Some(after) = rest.strip_prefix(placeholder) {
                id.push_str(value);
                rest = after;
                continue 'scan;
            }
        }
        id.push(c);
        rest = &rest[c.len_utf8()..];
    }
    id
}

fn junit_results(xml: &str, template: &str, class: JunitClass) -> Result<TestResults> {
    struct Context<'a> {
        template: &'a str,
        class: JunitClass,
    }
    /// `suites` holds the names of the enclosing describe-level testsuites,
    /// outermost first; `None` marks a suite without a `name` attribute.
    fn collect<'a>(
        node: &'a JunitNode,
        enclosing_suite_file: Option<&'a str>,
        mut suites: Vec<Option<&'a str>>,
        inside_suite: bool,
        context: &Context,
        results: &mut TestResults,
    ) -> Result<()> {
        let template = context.template;
        let suite_file = if node.tag == "testsuite" {
            node.attributes
                .get("file")
                .map(String::as_str)
                .or(enclosing_suite_file)
        } else {
            enclosing_suite_file
        };
        // The file-level suite is the outermost testsuite that names a file:
        // Bun writes one per test file, with one nested testsuite per describe,
        // each of which also carries the file. Matching on structure rather than
        // on name == file keeps a describe that happens to be named like its
        // file in the path.
        if node.tag == "testsuite" && (inside_suite || !node.attributes.contains_key("file")) {
            suites.push(node.attributes.get("name").map(String::as_str));
        }
        let inside_suite = inside_suite || node.tag == "testsuite";
        for child in &node.children {
            if child.tag == "testsuite" {
                collect(
                    child,
                    suite_file,
                    suites.clone(),
                    inside_suite,
                    context,
                    results,
                )?;
            } else if child.tag == "testcase" && node.tag == "testsuite" {
                let name = child
                    .attributes
                    .get("name")
                    .ok_or("testcase missing name")?;
                let classname = child
                    .attributes
                    .get("classname")
                    .ok_or("testcase missing classname")?;
                let file = child
                    .attributes
                    .get("file")
                    .map(String::as_str)
                    .or(suite_file);
                if template.contains("{file}") && file.is_none() {
                    return Err(format!(
                        "testcase {name:?} uses {{file}} but has no file attribute on the testcase or enclosing testsuite"
                    ));
                }
                // Replace placeholders in the template, never in attribute values:
                // a literal {name} in a class name must survive byte-for-byte.
                // Nothing is split either, so a describe name containing " > "
                // stays one path element.
                let suite = if template.contains("{suite}") || context.class == JunitClass::Suites {
                    suites
                        .iter()
                        .map(|suite| suite.ok_or_else(|| format!(
                            "testcase {name:?} needs its suite path but an enclosing testsuite has no name attribute"
                        )))
                        .collect::<Result<Vec<_>>>()?
                        .join(" > ")
                } else {
                    String::new()
                };
                let id = expand_broad_id(
                    template,
                    &[
                        ("{classname}", classname),
                        ("{name}", name),
                        ("{file}", file.unwrap_or_default()),
                        ("{suite}", &suite),
                    ],
                );
                validate_command_id(&id)?;
                let class_target = match context.class {
                    JunitClass::Classname => classname.as_str(),
                    JunitClass::Suites => suite.as_str(),
                };
                let target = if class_target.is_empty() {
                    file.unwrap_or_default()
                } else {
                    class_target
                };
                if results
                    .targets
                    .insert(id.clone(), target.to_owned())
                    .is_some()
                {
                    return Err(format!(
                        "duplicate JUnit id {id:?}; use a more specific broad_id"
                    ));
                }
                results.names.insert(id.clone(), id.clone());
                let failures: Vec<_> = child
                    .children
                    .iter()
                    .filter(|node| matches!(node.tag.as_str(), "failure" | "error"))
                    .collect();
                if !failures.is_empty() {
                    let output = failures
                        .iter()
                        .map(|failure| {
                            format!(
                                "{}\n{}",
                                failure.attributes.get("message").map_or("", String::as_str),
                                failure.text
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    results.failure_output.insert(id.clone(), output);
                    results.red.push(id);
                } else if !child.children.iter().any(|node| node.tag == "skipped") {
                    results.green.push(id);
                }
            }
        }
        Ok(())
    }
    let document = junit_document(xml)?;
    let mut results = TestResults {
        red: vec![],
        green: vec![],
        targets: BTreeMap::new(),
        names: BTreeMap::new(),
        failure_output: BTreeMap::new(),
        signals: BTreeMap::new(),
    };
    collect(
        &document,
        None,
        Vec::new(),
        false,
        &Context { template, class },
        &mut results,
    )?;
    if results.targets.is_empty() {
        return Err("JUnit report contains zero testcases".into());
    }
    Ok(results)
}

#[cfg(test)]
mod junit_tests {
    use super::*;

    #[test]
    fn templates_preserve_decoded_names_and_classnames_without_recursive_substitution() {
        let xml = r#"<testsuites><testsuite><testsuite>
            <testcase classname="routes.{name}" name="scoped route &amp; café"><failure message="why &amp; how">trace &lt;here&gt;</failure></testcase>
            <testcase classname="" name="top level name"/>
            </testsuite></testsuite></testsuites>"#;
        let results = junit_results(xml, "{classname}.{name}", JunitClass::Classname).unwrap();
        assert_eq!(results.red, ["routes.{name}.scoped route & café"]);
        assert_eq!(results.targets[&results.red[0]], "routes.{name}");
        assert_eq!(
            results.failure_output[&results.red[0]],
            "why & how\ntrace <here>"
        );
        assert_eq!(results.green, [".top level name"]);
        let results = junit_results(xml, "{name}", JunitClass::Classname).unwrap();
        assert_eq!(results.red, ["scoped route & café"]);
        assert_eq!(results.green, ["top level name"]);
    }

    #[test]
    fn bun_junit_empty_classnames_use_testcase_and_suite_file_targets() {
        let xml = r#"<testsuite name="src/tests/quota-surfaces.test.ts" file="src/tests/quota-surfaces.test.ts">
            <testcase name="top-level test" classname="" file="src/tests/quota-surfaces.test.ts"/>
            <testcase name="suite-file test" classname=""/>
            <testcase name="named suite test" classname="quota surface normalization" file="other.test.ts"/>
        </testsuite>"#;
        let results = junit_results(xml, "{name}", JunitClass::Classname).unwrap();
        assert_eq!(
            results.targets["top-level test"],
            "src/tests/quota-surfaces.test.ts"
        );
        assert_eq!(
            results.targets["suite-file test"],
            "src/tests/quota-surfaces.test.ts"
        );
        assert_eq!(
            results.targets["named suite test"],
            "quota surface normalization"
        );
    }

    #[test]
    fn broad_id_file_uses_testcase_then_enclosing_suite_file() {
        let xml = r#"<testsuites><testsuite file="outer.test.ts"><testsuite file="suite.test.ts">
            <testcase classname="" name="own file" file="case.test.ts"/>
            <testcase classname="" name="suite file"/>
        </testsuite></testsuite></testsuites>"#;
        let results = junit_results(xml, "{file}:{name}", JunitClass::Classname).unwrap();
        assert!(results.targets.contains_key("case.test.ts:own file"));
        assert!(results.targets.contains_key("suite.test.ts:suite file"));
    }

    #[test]
    fn broad_id_file_without_a_file_names_the_testcase_in_error() {
        let xml = r#"<testsuite><testcase classname="" name="fileless case"/></testsuite>"#;
        let error = junit_results(xml, "{file}:{name}", JunitClass::Classname).unwrap_err();
        assert!(error.contains("fileless case"), "{error}");
        assert!(error.contains("{file}"), "{error}");
    }

    /// Bun 1.3.14's shape for a test nested three describe levels deep: one file-level
    /// testsuite, one nested testsuite per describe carrying its name, and a
    /// classname written innermost first with the joining ` > ` escaped twice.
    const BUN_NESTED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="bun test" tests="3">
  <testsuite name="src/tests/a.test.ts" file="src/tests/a.test.ts" tests="3">
    <testcase name="top" classname="" file="src/tests/a.test.ts" line="2"/>
    <testcase name="suite file only" classname=""/>
    <testsuite name="outer" file="src/tests/a.test.ts" line="3">
      <testcase name="one-level" classname="outer" file="src/tests/a.test.ts" line="4"/>
      <testsuite name="inner" file="src/tests/a.test.ts" line="5">
        <testsuite name="deepest" file="src/tests/a.test.ts" line="6">
          <testcase name="three-level" classname="deepest &amp;gt; inner &amp;gt; outer" file="src/tests/a.test.ts" line="7"><failure message="boom"/></testcase>
        </testsuite>
      </testsuite>
    </testsuite>
  </testsuite>
</testsuites>"#;

    #[test]
    fn bun_nested_suites_give_outermost_first_ids_and_opt_in_targets() {
        let results = junit_results(BUN_NESTED, "{suite}.{name}", JunitClass::Suites).unwrap();
        assert_eq!(results.red, ["outer > inner > deepest.three-level"]);
        assert_eq!(
            results.targets["outer > inner > deepest.three-level"],
            "outer > inner > deepest"
        );
        // Tests directly in the file-level suite have an empty suite path, so the
        // target falls back to the testcase file, then the suite file.
        assert_eq!(results.targets[".top"], "src/tests/a.test.ts");
        assert_eq!(results.targets[".suite file only"], "src/tests/a.test.ts");
        assert_eq!(results.targets["outer.one-level"], "outer");
        // The default still reads the classname exactly as Bun wrote it, decoded
        // once, for both the target and the {classname} placeholder.
        let results = junit_results(BUN_NESTED, "{suite}.{name}", JunitClass::Classname).unwrap();
        assert_eq!(
            results.targets["outer > inner > deepest.three-level"],
            "deepest &gt; inner &gt; outer"
        );
        let results =
            junit_results(BUN_NESTED, "{classname}.{name}", JunitClass::Classname).unwrap();
        assert_eq!(results.red, ["deepest &gt; inner &gt; outer.three-level"]);
        assert_eq!(results.targets[".top"], "src/tests/a.test.ts");
    }

    #[test]
    fn one_describe_deep_targets_agree_under_both_sources() {
        for class in [JunitClass::Classname, JunitClass::Suites] {
            let results = junit_results(BUN_NESTED, "{name}", class).unwrap();
            assert_eq!(results.targets["one-level"], "outer", "{class:?}");
        }
    }

    #[test]
    fn suite_paths_are_never_split_or_recursively_expanded() {
        // Bun double-escapes the describe names inside the classname as well, so
        // the classname alone cannot tell which ` > ` joins two describes.
        let xml = r#"<testsuites><testsuite name="a.test.ts" file="a.test.ts">
            <testsuite name="outer" file="a.test.ts">
              <testsuite name="in &gt; ner {name}" file="a.test.ts">
                <testsuite name="deepest" file="a.test.ts">
                  <testcase name="leaf {suite}" classname="deepest &amp;gt; in &amp;gt; ner {name} &amp;gt; outer"/>
                </testsuite>
              </testsuite>
            </testsuite>
        </testsuite></testsuites>"#;
        let results = junit_results(xml, "{suite}/{name}", JunitClass::Suites).unwrap();
        assert_eq!(
            results.green,
            ["outer > in > ner {name} > deepest/leaf {suite}"]
        );
        assert_eq!(
            results.targets[&results.green[0]],
            "outer > in > ner {name} > deepest"
        );
    }

    #[test]
    fn a_describe_named_like_its_file_stays_in_the_suite_path() {
        // Only the outermost testsuite naming a file is the file-level one.
        let xml = r#"<testsuite name="a.test.ts" file="a.test.ts">
            <testsuite name="a.test.ts" file="a.test.ts">
              <testcase name="leaf" classname="a.test.ts"/>
            </testsuite>
        </testsuite>"#;
        let results = junit_results(xml, "{suite}|{name}", JunitClass::Suites).unwrap();
        assert_eq!(results.green, ["a.test.ts|leaf"]);
        // An outermost testsuite without a file attribute is a named suite.
        let xml = r#"<testsuite name="package"><testcase name="leaf" classname=""/></testsuite>"#;
        let results = junit_results(xml, "{suite}|{name}", JunitClass::Suites).unwrap();
        assert_eq!(results.targets["package|leaf"], "package");
    }

    #[test]
    fn a_needed_suite_path_through_a_nameless_suite_is_an_error() {
        let xml = r#"<testsuites><testsuite><testcase name="leaf" classname="c"/></testsuite></testsuites>"#;
        for (template, class) in [
            ("{suite}.{name}", JunitClass::Classname),
            ("{name}", JunitClass::Suites),
        ] {
            let error = junit_results(xml, template, class).unwrap_err();
            assert!(
                error.contains("leaf") && error.contains("no name attribute"),
                "{error}"
            );
        }
        assert_eq!(
            junit_results(xml, "{name}", JunitClass::Classname)
                .unwrap()
                .targets["leaf"],
            "c"
        );
    }

    #[test]
    fn malformed_or_ambiguous_junit_never_establishes_breadth() {
        for xml in [
            "", "garbage", "<testsuite>", "<testsuite><testcase></testsuite>",
            "<testsuite/><testsuite/>", "<testsuite/>trailing garbage",
            "<testsuite><testcase classname='a' name='b'/><testcase classname='c' name='b'/></testsuite>",
            "<testsuite><testcase classname='a' name='&bogus;'/></testsuite>",
            "<testsuite><testcase name='missing class'/></testsuite>",
            "<!DOCTYPE testsuite><testsuite/>",
        ] {
            assert!(junit_results(xml, "{name}", JunitClass::Classname).is_err(), "{xml}");
        }
        let error = junit_results(
            "<testsuite><testcase name='missing class'/></testsuite>",
            "{name}",
            JunitClass::Classname,
        )
        .unwrap_err();
        assert_eq!(error, "testcase missing classname");
        let xml = "<testsuite><testcase classname='a' name='b'><system-out><failure>not a failure child</failure></system-out></testcase></testsuite>";
        assert!(junit_results(xml, "{name}", JunitClass::Classname)
            .unwrap()
            .red
            .is_empty());
    }
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
        // A clean (unmutated) baseline that fails says the row cannot be graded:
        // every branch below ends it as ERROR, so record what that run printed
        // before returning.
        if baseline && !output.success {
            report.baseline_failure = Some(BaselineFailure::from_output(&output));
        }
        if output.timeout {
            report.timed_out_phase = Some(Phase::Test);
            return Err(format!(
                "test command exceeded timeout_s = {}{}",
                c.timeout_s,
                deadline_note(&output, false)
            ));
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

/// Run a Cargo/nextest row's clean-tree build and tests. A deadline that
/// expires on either grades the row TIMED_OUT here and returns Ok, so the
/// caller stops before any mutant is applied; other failures are errors.
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
        report.baseline_failure = Some(BaselineFailure::from_output(&build));
        report.outcome = Outcome::TimedOut;
        report.reason = Some(format!(
            "baseline build exceeded build_timeout_s = {}{}",
            c.build_timeout_s,
            deadline_note(&build, false)
        ));
        return Ok(());
    }
    if build.interrupted || !build.success {
        if !build.interrupted {
            report.baseline_failure = Some(BaselineFailure::from_output(&build));
        }
        return Err(format!("baseline build failed: {}", tail(&build.text)));
    }
    let tests = execute(root, command(c, "run", scope)?, c.timeout_s, stop)?;
    report.baseline_test_ms += tests.ms;
    report.test_tail = tail(&tests.text);
    // A non-green run can still be a usable baseline when only tests outside
    // `expect_red` failed, so keep it aside and report it only on rejection.
    report.baseline_run =
        (!tests.success && !tests.interrupted).then(|| BaselineFailure::from_output(&tests));
    if tests.timeout {
        report.timed_out_phase = Some(Phase::Test);
        report.baseline_failure = report.baseline_run.clone();
        report.outcome = Outcome::TimedOut;
        report.reason = Some(format!(
            "baseline test run exceeded timeout_s = {}{}",
            c.timeout_s,
            deadline_note(&tests, c.runner == "cargo")
        ));
        return Ok(());
    }
    let checked = cargo_baseline_results(c, report, &tests);
    if checked.is_err() {
        report.baseline_failure = report.baseline_run.clone();
    }
    checked
}

fn cargo_baseline_results(c: &Control, report: &mut Report, tests: &Output) -> Result<()> {
    if tests.interrupted || tests.code.is_none() {
        return Err(format!("baseline test run failed: {}", tail(&tests.text)));
    }
    let results = parse_test_results(&tests.text, &c.runner)?;
    validate_selected_execution(c, &results)?;
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
            report.baseline_failure = report.baseline_run.clone();
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

fn validate_run_set(c: &Control, results: &TestResults) -> Result<()> {
    let Some(selection @ (TestSelection::Filter(_) | TestSelection::Tests(_))) = &c.select else {
        return Ok(());
    };
    for name in &c.expect_red {
        if !selection.contains(name, &c.expect_red) {
            return Err(format!(
                "{}: expect_red test {name:?} is outside the selected run set",
                c.label()
            ));
        }
        if !results.names.values().any(|actual| actual == name) {
            return Err(format!(
                "{}: expect_red test {name:?} does not exist or is skipped in the run set",
                c.label()
            ));
        }
    }
    if let TestSelection::Tests(names) = selection {
        for name in names {
            if !results.names.values().any(|actual| actual == name) {
                return Err(format!(
                    "{}: selected run-set test {name:?} does not exist or is skipped",
                    c.label()
                ));
            }
        }
    }
    let selected: Vec<_> = results
        .names
        .values()
        .filter(|name| selection.contains(name, &c.expect_red))
        .collect();
    if selected.is_empty() {
        return Err(format!(
            "{}: selected run set has zero matching tests",
            c.label()
        ));
    }
    if !selected.iter().any(|name| !c.expect_red.contains(name)) {
        return Err(format!(
            "{}: run set must include a test outside expect_red; use select = \"expected\"",
            c.label()
        ));
    }
    Ok(())
}

fn validate_selected_execution(c: &Control, results: &TestResults) -> Result<()> {
    let Some(selection) = &c.select else {
        return Ok(());
    };
    let label = if *selection == TestSelection::Expected {
        "select = expected"
    } else {
        "selected run-set"
    };
    for name in &c.expect_red {
        // Match each expected name against the name the test harness itself
        // printed, not the report id that is prefixed with the test binary. A
        // name that matched no test, or a test the harness skipped, is an error.
        if !results.names.iter().any(|(id, actual)| {
            actual == name && (results.red.contains(id) || results.green.contains(id))
        }) {
            return Err(format!(
                "{}: {label} test {name:?} did not execute (zero matching tests)",
                c.label()
            ));
        }
    }
    if let Some(name) = results
        .names
        .values()
        .find(|name| !selection.contains(name, &c.expect_red))
    {
        return Err(format!("{}: {label} ran unlisted test {name:?}", c.label()));
    }
    // Skipped events do not establish that an expected-green sibling ran.
    let mut executed = results.clone();
    executed
        .names
        .retain(|id, _| results.red.contains(id) || results.green.contains(id));
    validate_run_set(c, &executed)
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
                    c.label(),
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

fn libtest_output_header(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("---- ")?;
    rest.strip_suffix(" stdout ----")
        .or_else(|| rest.strip_suffix(" stderr ----"))
}

struct LibtestBounds {
    results_start: usize,
    results_end: usize,
    summary: Option<usize>,
    failure_list: Option<usize>,
}

fn libtest_bounds(lines: &[&str], start: usize) -> LibtestBounds {
    let end = (start..lines.len())
        .find(|&i| {
            let line = lines[i].trim();
            line.starts_with("Running ") || line.starts_with("Doc-tests ")
        })
        .unwrap_or(lines.len());
    // Captured output is unescaped: it can contain another complete libtest
    // report, including identical block headers. Within each Cargo binary,
    // use the first running header and the last summary, never a nested run.
    let results_start = (start..end)
        .find(|&i| {
            lines[i]
                .trim()
                .strip_prefix("running ")
                .and_then(|s| s.strip_suffix(" tests").or_else(|| s.strip_suffix(" test")))
                .is_some_and(|n| n.parse::<usize>().is_ok())
        })
        .unwrap_or(start);
    let summary = (results_start..end).rfind(|&i| lines[i].trim().starts_with("test result:"));
    let report_end = summary.unwrap_or(end);
    // All per-test events precede the first failure/success report. A nested
    // failures: or ---- name stdout/stderr ---- cannot reopen the event stream.
    let results_end = (results_start..report_end)
        .find(|&i| {
            matches!(lines[i].trim(), "failures:" | "successes:")
                || libtest_output_header(lines[i].trim()).is_some()
        })
        .unwrap_or(report_end);
    // The real name list is the final failures: followed only by blank lines
    // and indented names up to the outer summary, not an earlier child report.
    let failure_list = (results_end..report_end).rfind(|&i| {
        lines[i].trim() == "failures:"
            && lines[i + 1..report_end]
                .iter()
                .any(|line| !line.trim().is_empty())
            && lines[i + 1..report_end]
                .iter()
                .all(|line| line.trim().is_empty() || line.starts_with(char::is_whitespace))
    });
    LibtestBounds {
        results_start,
        results_end,
        summary,
        failure_list,
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
    let lines: Vec<_> = text.lines().collect();
    let mut cargo_bounds = libtest_bounds(&lines, 0);
    let mut output_headers = BTreeSet::new();
    for (index, &line) in lines.iter().enumerate() {
        let raw_line = line;
        let line = line.trim();
        if runner == "cargo" {
            if let Some(target) = cargo_binary_header(line)? {
                cargo_target = target;
                cargo_started.insert(cargo_target.clone());
                cargo_bounds = libtest_bounds(&lines, index + 1);
                output_headers.clear();
                pending = None;
                failure_block = None;
                continue;
            } else if cargo_bounds.summary == Some(index) {
                failure_block = None;
                let rest = line.strip_prefix("test result:").unwrap();
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
            } else if index >= cargo_bounds.results_end {
                if cargo_bounds.failure_list.is_some_and(|list| index >= list)
                    || cargo_bounds.summary.is_some_and(|summary| index > summary)
                {
                    failure_block = None;
                    continue;
                }
                if let Some(name) = libtest_output_header(line) {
                    let key = (cargo_target.clone(), name.to_owned());
                    // Unknown or repeated headers can be text from a child run.
                    // Keep them in the current output, not as new test events.
                    if events.0.contains_key(&key)
                        && output_headers.insert((key.clone(), line.to_owned()))
                    {
                        failure_block = Some(key);
                        continue;
                    }
                }
                if let Some(key) = &failure_block {
                    let event = events.0.get_mut(key).unwrap();
                    event.output.push_str(raw_line);
                    event.output.push('\n');
                }
            } else if index < cargo_bounds.results_start {
                continue;
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
    if c.runner != "command" && results.targets.values().any(String::is_empty) {
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

fn breadth_outcome(c: &Control, report: &mut Report, cross_targets: &[String]) -> Outcome {
    if report.outcome != Outcome::Caught || !report.breadth_observed {
        return report.outcome.clone();
    }
    if let Some(reason) = &c.hub {
        let approved = c.hub_targets.as_deref().unwrap_or_default();
        let new_targets: Vec<_> = cross_targets
            .iter()
            .filter(|target| !approved.contains(target))
            .cloned()
            .collect();
        // Same-target failures do not require HUB approval. Fewer cross-target
        // failures are fine, but new targets need another person's review.
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
        report.outcome.clone()
    }
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
    let mut listings = ListingCache::default();
    replay(
        root,
        c,
        allow_dirty,
        stop,
        scope,
        ReplayContext {
            prebuild: &[],
            listings: &mut listings,
        },
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
    let mut listings = ListingCache::default();
    replay(
        root,
        c,
        allow_dirty,
        stop,
        Scope::Broad,
        ReplayContext {
            prebuild: &[],
            listings: &mut listings,
        },
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
    let mut listings = ListingCache::default();
    replay(
        root,
        c,
        allow_dirty,
        stop,
        Scope::Explore { workspace },
        ReplayContext {
            prebuild: &[],
            listings: &mut listings,
        },
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
    listings: ListingCache,
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
            listings: ListingCache::default(),
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
        let mut command_broad_shared: BTreeMap<Vec<String>, Report> = BTreeMap::new();
        for (position, c) in rows.iter().enumerate() {
            // Progress goes to stderr, one line as each baseline starts and
            // ends, so a log shows which row a stuck run is in. Stdout and the
            // JSON report are unaffected.
            let progress = format!(
                "progress: baseline {}/{} {}",
                position + 1,
                rows.len(),
                c.id
            );
            eprintln!("{progress}: started");
            let started = Instant::now();
            let active = c.matches_platform() && c.unreachable.is_none() && c.desk_only.is_none();
            let key = if active && c.runner != "command" {
                let mut key: Vec<String> = command(c, "build", scope)?
                    .get_args()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect();
                // The build arguments omit the ignored-test selection, yet a run
                // that includes ignored tests observes different results.
                key.push(format!(
                    "ignored={}",
                    c.ignored.map_or("none", IgnoredSelection::as_str)
                ));
                if let Some(selection) = &c.select {
                    key.push(selection.key(&c.expect_red));
                }
                key
            } else {
                vec![c.id.clone()]
            };
            let mut report = if let Some(report) = shared.get(&key) {
                let mut report = report.clone();
                report.id = c.id.clone();
                report.source = c.source.clone();
                report.baseline_build_ms = 0;
                report.baseline_test_ms = 0;
                // Shared test results say nothing about this row's edit anchors.
                report.line_endings = None;
                report
            } else {
                c.validate_runner()?;
                let mut selection = (*c).clone();
                // The baseline of a row's own expected tests is per row. The JUnit
                // package-wide baseline below is separate: rows with the same
                // `broad_command` and `broad_report` share one run of it.
                if c.runner == "command" {
                    selection.broad_command = None;
                    selection.broad_report = None;
                    selection.broad_id = None;
                    selection.junit_class = None;
                }
                // Baseline data is shared, but each row's expected reds are
                // validated independently before its mutant can be applied.
                if c.runner != "command" && c.select.is_none() {
                    selection.expect_red.clear();
                }
                let report = replay(
                    root,
                    &selection,
                    allow_dirty,
                    stop,
                    scope,
                    ReplayContext {
                        prebuild: &[],
                        listings: &mut self.listings,
                    },
                    ReplayStage::Baseline,
                )?;
                if report.outcome != Outcome::AnchorMissing {
                    shared.insert(key.clone(), report.clone());
                }
                report
            };
            if active && c.runner == "command" && scope == Scope::Broad && c.broad_command.is_some()
            {
                let mut key = c.broad_command.clone().unwrap();
                key.extend([
                    c.broad_report.clone().unwrap(),
                    c.broad_id.clone().unwrap(),
                    c.junit_class.unwrap_or_default().as_str().to_owned(),
                    c.timeout_s.to_string(),
                ]);
                let broad = if let Some(broad) = command_broad_shared.get(&key) {
                    let mut broad = broad.clone();
                    broad.baseline_test_ms = 0;
                    broad
                } else {
                    let mut broad = Report::new(c);
                    broad.outcome = Outcome::Survived;
                    if let Err(e) = command_breadth(root, c, stop, &mut broad, true) {
                        broad.outcome = Outcome::Error;
                        broad.reason = Some(e);
                    }
                    command_broad_shared.insert(key, broad.clone());
                    broad
                };
                report.baseline_test_ms += broad.baseline_test_ms;
                report.baseline_failures.extend(broad.baseline_failures);
                report.baseline_red = broad.baseline_red;
                report.baseline_results = broad.baseline_results;
                report.baseline_run = broad.baseline_run;
                report.test_tail = tail(&format!("{}\n{}", report.test_tail, broad.test_tail));
                if broad.outcome == Outcome::Error {
                    report.outcome = broad.outcome;
                    report.reason = broad.reason;
                    report.timed_out_phase = broad.timed_out_phase;
                    report.baseline_failure = broad.baseline_failure;
                } else if report.outcome == Outcome::Survived {
                    if let Err(e) = validate_command_broad_baseline(c, &mut report) {
                        report.outcome = Outcome::Error;
                        report.reason = Some(e);
                    }
                }
            }
            if active && report.outcome == Outcome::Survived && c.runner != "command" {
                // Resolve names in exactly the selection that the baseline ran.
                // Package-wide listing can compile unrelated, feature-gated targets.
                let start = Instant::now();
                let (names, listed_now) = self.listings.get(root, c, scope, true, stop);
                if listed_now {
                    report.baseline_build_ms += start.elapsed().as_millis();
                }
                let validation = names
                    .and_then(|names| resolve_listed(c, &names).map(|_| ()))
                    .and_then(|()| {
                        validate_selected_execution(c, report.baseline_results.as_ref().unwrap())
                    })
                    .and_then(|()| validate_baseline(c, &mut report));
                if let Err(e) = validation {
                    report.outcome = Outcome::Error;
                    report.reason = Some(e);
                }
            }
            eprintln!(
                "{progress}: {} ({:.1} s)",
                if report.outcome == Outcome::Survived {
                    "ready".to_owned()
                } else {
                    report.outcome.name()
                },
                started.elapsed().as_secs_f64()
            );
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
            ReplayContext {
                prebuild: &self.prebuild,
                listings: &mut self.listings,
            },
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
            return Err(format!(
                "{label}: exceeded timeout_s = {}{}",
                step.timeout_s,
                deadline_note(&output, false)
            ));
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

struct ReplayContext<'a> {
    prebuild: &'a [Prebuild],
    listings: &'a mut ListingCache,
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
    context: ReplayContext<'_>,
    stage: ReplayStage,
) -> Result<Report> {
    let ReplayContext { prebuild, listings } = context;
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
            c.label()
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
        let replacement = match replaced_scoped(root, &edits, c.scope.as_ref()) {
            Ok(f) => f,
            Err(e) if e.starts_with("ANCHOR_") => {
                report.outcome = Outcome::AnchorMissing;
                report.reason = Some(e);
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        report.line_endings = replacement.used_crlf.then(|| "crlf".into());
        let files = replacement.files;
        if stop.load(Ordering::SeqCst) {
            return Err("interrupted".into());
        }
        if !baseline_ready {
            if c.runner == "command" {
                command_tests(root, c, stop, &mut report, true)?;
                if scope == Scope::Broad && c.broad_command.is_some() {
                    command_breadth(root, c, stop, &mut report, true)?;
                    validate_command_broad_baseline(c, &mut report)?;
                }
            } else {
                if !c.expect_red.is_empty() && !matches!(scope, Scope::Explore { .. }) {
                    let start = Instant::now();
                    let (names, _) = listings.get(root, c, scope, true, stop);
                    let names = names?;
                    report.baseline_build_ms += start.elapsed().as_millis();
                    resolve_listed(c, &names)?;
                }
                cargo_baseline(root, c, scope, stop, &mut report)?;
                if report.outcome == Outcome::TimedOut {
                    return Ok(());
                }
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
        // A caller may have run only the rows' own expected-test baselines
        // before asking for `--broad`. Run the missing package-wide JUnit
        // baseline now, before any mutation is applied.
        if baseline_ready
            && c.runner == "command"
            && scope == Scope::Broad
            && c.broad_command.is_some()
            && report.baseline_results.is_none()
        {
            command_breadth(root, c, stop, &mut report, true)?;
            validate_command_broad_baseline(c, &mut report)?;
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
            if scope == Scope::Broad && c.broad_command.is_some() {
                command_breadth(root, c, stop, &mut report, false)?;
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
                "build exceeded build_timeout_s = {}{}",
                c.build_timeout_s,
                deadline_note(&build, false)
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
            report.reason = Some(format!(
                "test run exceeded timeout_s = {}{}",
                c.timeout_s,
                deadline_note(&tests, c.runner == "cargo")
            ));
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
        validate_selected_execution(c, &results)?;
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
                report.breadth_observed = scope == Scope::Broad && c.select.is_none();
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
                report.outcome = outcome;
                breadth_outcome(c, &mut report, &cross_targets)
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
    if c.select.is_some() && !baseline_only {
        let selection = if c.select == Some(TestSelection::Expected) {
            "selection was expected-only; breadth not observed"
        } else {
            "selection was narrowed to a run set; breadth not observed"
        };
        report.reason = Some(report.reason.map_or_else(
            || selection.to_owned(),
            |reason| format!("{reason}; {selection}"),
        ));
    }
    Ok(report)
}

/// The tests a row's selection runs, plus the `#[ignore]`d tests that the
/// selection lists but skips, as (binary, name) pairs. The skipped ones let a
/// row expecting an ignored test fail by naming the `ignored` field instead of
/// reporting that no expected test ran.
#[derive(Clone)]
struct Listing {
    results: TestResults,
    skipped_ignored: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ListingKey {
    // The list command's arguments capture the runner, package, effective
    // target selection, features, and ignored-test selection.
    args: Vec<String>,
    clean_tree: bool, // False keeps mutant-tree listings out of clean baselines.
}

impl ListingKey {
    fn new(c: &Control, scope: Scope, clean_tree: bool) -> Result<Self> {
        let mut args: Vec<String> = command(c, "list", scope)?
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        // Listings cover the target so membership can be checked before running.
        // Filter and explicit-list selections cache one listing per run set,
        // independently of each row's expected reds. Expected-only rows still
        // share the target listing because their sets are just those reds.
        if let Some(selection @ (TestSelection::Filter(_) | TestSelection::Tests(_))) = &c.select {
            args.push(selection.key(&c.expect_red));
        }
        Ok(Self { args, clean_tree })
    }
}

#[derive(Default)]
struct ListingCache(BTreeMap<ListingKey, Result<Listing>>);

impl ListingCache {
    /// Return the cached result and whether this call performed a listing.
    /// Errors are cached too, so one failed listing is not retried in a session.
    fn get(
        &mut self,
        root: &Path,
        c: &Control,
        scope: Scope,
        clean_tree: bool,
        stop: &AtomicBool,
    ) -> (Result<Listing>, bool) {
        let key = match ListingKey::new(c, scope, clean_tree) {
            Ok(key) => key,
            Err(error) => return (Err(error), false),
        };
        self.get_or_insert_with(key, || list_results(root, c, scope, stop))
    }

    fn get_or_insert_with(
        &mut self,
        key: ListingKey,
        list: impl FnOnce() -> Result<Listing>,
    ) -> (Result<Listing>, bool) {
        if let Some(result) = self.0.get(&key) {
            return (result.clone(), false);
        }
        let result = list();
        self.0.insert(key, result.clone());
        (result, true)
    }
}

#[cfg(test)]
mod listing_cache_tests {
    use super::*;

    fn control() -> Control {
        toml::from_str(
            r#"
id = "listing"
guards = "a listing test"
file = "src/lib.rs"
old = "true"
new = "false"
test_file = "src/lib.rs"
runner = "cargo"
package = "fixture"
target = "--lib"
expect_red = ["test"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn listing_key_separates_ignored_and_mutated_tree_selections() {
        let control = control();
        let clean = ListingKey::new(&control, Scope::Row, true).unwrap();
        let mut ignored = control.clone();
        ignored.ignored = Some(IgnoredSelection::Include);
        assert_ne!(clean, ListingKey::new(&ignored, Scope::Row, true).unwrap());
        assert_ne!(clean, ListingKey::new(&control, Scope::Row, false).unwrap());
    }

    #[test]
    fn listing_cache_reuses_failures_without_retrying() {
        let key = ListingKey::new(&control(), Scope::Row, true).unwrap();
        let mut cache = ListingCache::default();
        let mut calls = 0;
        let (first, first_ran) = cache.get_or_insert_with(key.clone(), || {
            calls += 1;
            Err("list failed".into())
        });
        let (second, second_ran) = cache.get_or_insert_with(key, || {
            calls += 1;
            Err("retried list".into())
        });
        assert_eq!(first.err().as_deref(), Some("list failed"));
        assert_eq!(second.err().as_deref(), Some("list failed"));
        assert!(first_ran);
        assert!(!second_ran);
        assert_eq!(calls, 1);
    }
}

fn list_output(root: &Path, c: &Control, scope: Scope, stop: &AtomicBool) -> Result<String> {
    // List mode compiles the test binaries, so the build deadline bounds it.
    let output = execute(root, command(c, "list", scope)?, c.build_timeout_s, stop)?;
    if !output.success || output.timeout || output.interrupted {
        return Err(format!(
            "{}: list failed: {}",
            c.label(),
            tail(&output.text)
        ));
    }
    Ok(output.text)
}

fn cargo_listed(
    root: &Path,
    c: &Control,
    scope: Scope,
    stop: &AtomicBool,
) -> Result<Vec<(String, String)>> {
    let mut target = String::new();
    let mut listed = vec![];
    for line in list_output(root, c, scope, stop)?.lines().map(str::trim) {
        if let Some(binary) = cargo_binary_header(line)? {
            target = binary;
        } else if let Some(name) = line.strip_suffix(": test") {
            listed.push((target.clone(), name.to_owned()));
        }
    }
    Ok(listed)
}

fn list_results(root: &Path, c: &Control, scope: Scope, stop: &AtomicBool) -> Result<Listing> {
    let mut events = TestEvents::default();
    let mut skipped_ignored = vec![];
    if c.runner == "cargo" {
        let listed = cargo_listed(root, c, scope, stop)?;
        // libtest's plain `--list` includes ignored tests without marking them,
        // so a row that skips them asks the harness which ones they are.
        let ignored: BTreeSet<_> = if c.ignored.is_none() && !c.expect_red.is_empty() {
            let mut only = c.clone();
            only.ignored = Some(IgnoredSelection::Only);
            cargo_listed(root, &only, scope, stop)?
                .into_iter()
                .collect()
        } else {
            BTreeSet::new()
        };
        for (target, name) in listed {
            if ignored.contains(&(target.clone(), name.clone())) {
                skipped_ignored.push((target, name));
            } else {
                events.record(&target, &name, false)?;
            }
        }
    } else {
        let text = list_output(root, c, scope, stop)?;
        let json: serde_json::Value = serde_json::from_str(
            text.lines()
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
            for (name, case) in cases {
                // nextest lists every test whatever the selection, marking
                // each ignored one, so selection is decided from that mark.
                let ignored = case
                    .get("ignored")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                match (ignored, c.ignored) {
                    (true, None) => skipped_ignored.push((target.clone(), name.clone())),
                    (false, Some(IgnoredSelection::Only)) => {}
                    _ => events.record(target, name, false)?,
                }
            }
        }
    }
    Ok(Listing {
        results: events.finish(&c.runner),
        skipped_ignored,
    })
}

/// Resolve a row's expected names against its listing, refusing by name an
/// expected test that exists only as an `#[ignore]`d test the row skips.
fn resolve_listed(c: &Control, listing: &Listing) -> Result<Vec<String>> {
    validate_run_set(c, &listing.results)?;
    let resolved = resolve_expected(c, &listing.results)?;
    for (expected, identity) in c.expect_red.iter().zip(&resolved) {
        if listing.results.names.contains_key(identity) {
            continue;
        }
        if listing.skipped_ignored.iter().any(|(target, name)| {
            name == expected || format!("{}::{name}", stable_target(&c.runner, target)) == *expected
        }) {
            return Err(format!(
                "{}: expect_red test {expected} is #[ignore]d and this row does not run ignored tests; set ignored = \"include\" (or \"only\") on the row",
                c.label()
            ));
        }
    }
    if c.select == Some(TestSelection::Expected) {
        for expected in &c.expect_red {
            // Exact harness selections use raw test names, not binary-qualified report ids.
            if !listing.results.names.values().any(|name| name == expected) {
                return Err(format!(
                    "{}: select = expected test {expected:?} matches no listed test by exact harness name",
                    c.label()
                ));
            }
        }
    }
    Ok(resolved)
}

/// Test-name coverage established by `check`, separate from anchor validation.
#[derive(Debug, Default)]
pub struct CheckReport {
    pub verified: Vec<String>,
    /// Row id and the reason its test names were not verified.
    pub not_verified: Vec<(String, String)>,
    /// Active command rows without a listing protocol, for strict CLI enforcement.
    pub missing_list_commands: Vec<String>,
}

fn check_command_listing(root: &Path, c: &Control, stop: &AtomicBool) -> Result<()> {
    let argv = c.list_command.as_ref().ok_or("list_command is required")?;
    for pattern in &c.expect_red {
        let error = |reason| {
            format!(
                "{}: expect_red pattern {pattern:?}: list_command: {reason}",
                c.label()
            )
        };
        // The external runner owns selection semantics. Pass the pattern intact,
        // just as replay does, instead of interpreting it with a different regex engine.
        let output = execute_output(root, named_command(argv, pattern), c.timeout_s, stop, true)
            .map_err(error)?;
        if output.interrupted {
            return Err(error("interrupted".into()));
        }
        if output.timeout {
            return Err(error(format!("exceeded timeout_s = {}", c.timeout_s)));
        }
        if !output.success {
            return Err(error(format!(
                "failed (exit {:?}, signal {:?}): {}",
                output.code,
                output.signal,
                tail(&output.text)
            )));
        }
        let names: Vec<String> = serde_json::from_str(&output.stdout).map_err(|e| {
            error(format!(
                "stdout must be one JSON array of selected test names: {e}"
            ))
        })?;
        if names.is_empty() {
            return Err(error("matches no listed test".into()));
        }
    }
    Ok(())
}

/// Every row whose edits no longer apply to the tree, in catalogue order, each
/// as `"<row label>: <reason>"`. An anchor that matches nothing, matches more
/// than once, or names a missing file is a failure of that row alone, so all
/// rows are examined: stopping at the first would hide every later broken row.
pub fn anchor_failures(root: &Path, catalogue: &Catalogue) -> Result<Vec<String>> {
    let mut failures = Vec::new();
    for c in &catalogue.control {
        if let Err(e) = replaced_scoped(root, &c.edits()?, c.scope.as_ref()) {
            failures.push(format!("{}: {e}", c.label()));
        }
    }
    Ok(failures)
}

pub fn check(root: &Path, catalogue: &Catalogue, stop: &AtomicBool) -> Result<CheckReport> {
    check_for_platform(root, catalogue, stop, None)
}

pub fn check_for_platform(
    root: &Path,
    catalogue: &Catalogue,
    stop: &AtomicBool,
    platform: Option<&str>,
) -> Result<CheckReport> {
    if let Some(platform) = platform {
        if !known_os(platform) {
            return Err(format!("unknown target_os for --platform: {platform:?}"));
        }
    }
    validate(root, catalogue)?;
    // Anchors are checked before the prebuild so a broken row is reported
    // without waiting for a build; the error lists every broken row, one per line.
    let failures = anchor_failures(root, catalogue)?;
    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    let rows: Vec<_> = catalogue.control.iter().collect();
    let mut session = if let Some(platform) = platform {
        let preparation_rows: Vec<_> = catalogue
            .control
            .iter()
            .filter(|c| c.matches_platform_on(platform))
            .map(|c| {
                let mut selected = c.clone();
                selected.platforms = None;
                selected
            })
            .collect();
        let preparation_refs: Vec<_> = preparation_rows.iter().collect();
        ReplaySession::prepare(root, &catalogue.prebuild, &preparation_refs, false, stop)?
    } else {
        ReplaySession::prepare(root, &catalogue.prebuild, &rows, false, stop)?
    };
    let mut report = CheckReport::default();
    for c in &catalogue.control {
        let platform_matches = platform.map_or_else(
            || c.matches_platform(),
            |platform| c.matches_platform_on(platform),
        );
        let reason = if !platform_matches {
            Some(match platform {
                Some(platform) => format!("platform does not match --platform {platform}"),
                None => "platform does not match this host".to_owned(),
            })
        } else if c.unreachable.is_some() {
            Some("unreachable row".to_owned())
        } else if c.desk_only.is_some() {
            Some("desk-only row".to_owned())
        } else if c.runner == "command" && c.list_command.is_none() {
            report.missing_list_commands.push(c.id.clone());
            Some("command row has no list_command".to_owned())
        } else {
            None
        };
        if let Some(reason) = reason {
            report.not_verified.push((c.id.clone(), reason));
            continue;
        }
        if c.runner == "command" {
            check_command_listing(root, c, stop)?;
            report.verified.push(c.id.clone());
            continue;
        }
        let (selected, _) = session.listings.get(root, c, Scope::Row, true, stop);
        let selected = selected.map_err(|e| format!("{}: {e}", c.label()))?;
        for expected in resolve_listed(c, &selected)? {
            if !selected.results.names.contains_key(&expected) {
                return Err(format!(
                    "{}: expect_red name no longer exists: {expected}",
                    c.label()
                ));
            }
        }
        report.verified.push(c.id.clone());
    }
    Ok(report)
}

fn git_catalogue(root: &Path, path: &str, base: &str) -> Result<Option<Catalogue>> {
    let path = Path::new(path);
    let mut candidates = vec![path.to_path_buf()];
    match path.file_name().and_then(|n| n.to_str()) {
        Some("mutations.toml") => candidates.push(path.with_file_name("mutations.d")),
        Some("mutations.d") => candidates.push(path.with_file_name("mutations.toml")),
        _ => {}
    }
    let mut args = vec!["ls-tree", "-r", "--name-only", "-z", base, "--"];
    let names: Vec<_> = candidates
        .iter()
        .map(|p| p.to_str().ok_or("non-UTF-8 catalogue path"))
        .collect::<std::result::Result<_, _>>()?;
    args.extend(names);
    let listing = git(root, &args)?;
    let files: Vec<_> = listing
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|b| std::str::from_utf8(b).map_err(|e| e.to_string()))
        .collect::<Result<_>>()?;
    let mut fragments = Vec::new();
    let mut forms = BTreeSet::new();
    for candidate in &candidates {
        for file in &files {
            let file_path = Path::new(file);
            let single = file_path == candidate;
            let direct = file_path.parent() == Some(candidate.as_path());
            if single
                || (direct
                    && !file_path
                        .file_name()
                        .unwrap()
                        .as_encoded_bytes()
                        .starts_with(b".")
                    && file_path.extension().is_some_and(|e| e == "toml"))
            {
                forms.insert(candidate);
                let bytes = git(root, &["show", &format!("{base}:{file}")])?;
                fragments.push((
                    (*file).to_owned(),
                    String::from_utf8(bytes).map_err(|e| e.to_string())?,
                ));
            }
        }
    }
    if forms.len() > 1 {
        return Err(format!(
            "base catalogue has both forms: {}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" and ")
        ));
    }
    if fragments.is_empty() {
        return Ok(None);
    }
    fragments.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    load_fragments(fragments)
        .map(Some)
        .map_err(|e| format!("base catalogue is invalid: {e}"))
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
    let old = match git_catalogue(root, catalogue_path, base)? {
        Some(catalogue) => catalogue,
        None if paths
            .iter()
            .any(|p| Path::new(p).starts_with(catalogue_path)) =>
        {
            Catalogue::default()
        }
        None => return Err("base catalogue missing and catalogue not added in diff".into()),
    };
    let mut selected = BTreeSet::new();
    for c in &catalogue.control {
        if paths.contains(c.test_file.as_str())
            || old.prebuild != catalogue.prebuild
            || c.edits()?.iter().any(|e| paths.contains(e.file.as_str()))
            || !old.control.iter().any(|o| {
                let mut definition = o.clone();
                // Moving a row between fragments does not change its mutation.
                definition.source = c.source.clone();
                definition == *c
            })
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
mod line_ending_tests {
    use super::*;

    fn apply(bytes: &[u8], old: &str, new: &str) -> Result<Replacements> {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source.rs"), bytes).unwrap();
        let result = replaced_scoped(
            root.path(),
            &[Edit {
                file: "source.rs".into(),
                old: old.into(),
                new: new.into(),
            }],
            None,
        );
        assert_eq!(fs::read(root.path().join("source.rs")).unwrap(), bytes);
        result
    }

    #[test]
    fn scoped_rows_refuse_edits_in_more_than_one_file() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.rs"), "fn a() { x }\n").unwrap();
        fs::write(root.path().join("b.rs"), "fn a() { x }\n").unwrap();
        let edit = |file: &str| Edit {
            file: file.into(),
            old: "x".into(),
            new: "y".into(),
        };
        let scope = AnchorScope {
            after: "fn a()".into(),
            before: None,
        };
        let error = replaced_scoped(root.path(), &[edit("a.rs"), edit("b.rs")], Some(&scope))
            .err()
            .expect("a scoped row editing two files is refused");
        assert!(error.starts_with("ANCHOR_SCOPE_MULTI_FILE a.rs"), "{error}");
        // Two scoped edits in the same file stay allowed.
        fs::write(root.path().join("a.rs"), "fn a() { x; z }\n").unwrap();
        let same_file = [
            edit("a.rs"),
            Edit {
                file: "a.rs".into(),
                old: "z".into(),
                new: "w".into(),
            },
        ];
        assert!(replaced_scoped(root.path(), &same_file, Some(&scope)).is_ok());
    }

    #[test]
    fn lf_multiline_anchor_retries_crlf_without_changing_other_bytes() {
        let result = apply(
            b"before\r\nold\r\nbody\r\nafter\r\n",
            "old\nbody",
            "new\nbody",
        )
        .unwrap();
        assert!(result.used_crlf);
        assert_eq!(
            result.files.values().next().unwrap().as_bytes(),
            b"before\r\nnew\r\nbody\r\nafter\r\n"
        );
    }

    #[test]
    fn mixed_endings_use_only_the_exact_anchor_form() {
        let result = apply(
            b"old\nbody\nseparator\r\nold\r\nbody\r\n",
            "old\nbody",
            "new\nbody",
        )
        .unwrap();
        assert!(!result.used_crlf);
        assert_eq!(
            result.files.values().next().unwrap().as_bytes(),
            b"new\nbody\nseparator\r\nold\r\nbody\r\n"
        );
    }

    #[test]
    fn exact_anchor_ambiguity_never_retries_the_crlf_form() {
        let error = apply(
            b"old\nbody\nold\nbody\nold\r\nbody\r\n",
            "old\nbody",
            "new\nbody",
        )
        .err()
        .unwrap();
        assert!(error.contains("replaced 2 occurrences"), "{error}");
    }

    #[test]
    fn crlf_retry_requires_exactly_one_occurrence() {
        for (bytes, expected) in [
            (b"old\r\nbody\r\nold\r\nbody\r\n".as_slice(), 2),
            (b"unrelated\r\nbody\r\n".as_slice(), 0),
        ] {
            let error = apply(bytes, "old\nbody", "new\nbody").err().unwrap();
            assert!(
                error.contains(&format!("replaced {expected} occurrences")),
                "{error}"
            );
        }
    }

    #[test]
    fn explicit_crlf_anchor_matches_exactly_without_retry() {
        let result = apply(b"old\r\nbody\r\n", "old\r\nbody", "new\r\nbody").unwrap();
        assert!(!result.used_crlf);
        assert_eq!(
            result.files.values().next().unwrap().as_bytes(),
            b"new\r\nbody\r\n"
        );
    }
}

#[cfg(test)]
mod broad_command_tests {
    use super::*;

    #[test]
    fn run_set_commands_use_literal_contains_or_exact_names_and_separate_listing_keys() {
        let mut row =
            toml::from_str::<Catalogue>(include_str!("../tests/fixture/mutations-v0.1.toml"))
                .unwrap()
                .control
                .remove(0);
        row.expect_red = vec!["tests::guard".into()];
        let filter = "-a.*),b\\c/\n\r\t\u{1}";
        for runner in ["cargo", "nextest"] {
            row.runner = runner.into();
            row.select = Some(TestSelection::Filter(filter.into()));
            let cmd = command(&row, "run", Scope::Broad).unwrap();
            let args: Vec<_> = cmd.get_args().map(|arg| arg.to_str().unwrap()).collect();
            assert!(args.contains(&"--lib") && !args.contains(&"--tests"));
            if runner == "cargo" {
                assert!(args.ends_with(&["--", filter]));
                assert!(!args.contains(&"--exact"));
            } else {
                assert!(
                    args.windows(2)
                        .any(|args| args == ["-E", "test(~-a.*\\)\\,b\\\\c\\/\\n\\r\\t\\u{1})"]),
                    "{args:?}"
                );
            }
            let first = ListingKey::new(&row, Scope::Row, true).unwrap();
            assert_eq!(first, ListingKey::new(&row, Scope::Broad, true).unwrap());
            row.select = Some(TestSelection::Filter("other".into()));
            assert_ne!(first, ListingKey::new(&row, Scope::Row, true).unwrap());
            row.select = Some(TestSelection::Tests(vec![
                "tests::guard".into(),
                "tests::sibling".into(),
            ]));
            let tests_key = ListingKey::new(&row, Scope::Row, true).unwrap();
            assert_ne!(first, tests_key);
            let cmd = command(&row, "run", Scope::Row).unwrap();
            let args: Vec<_> = cmd.get_args().map(|arg| arg.to_str().unwrap()).collect();
            if runner == "cargo" {
                assert!(args.ends_with(&["--exact", "tests::guard", "tests::sibling"]));
            } else {
                assert!(args
                    .windows(2)
                    .any(|args| args == ["-E", "test(=tests::guard) | test(=tests::sibling)"]));
            }
            row.select = Some(TestSelection::Tests(vec![
                "tests::guard".into(),
                "tests::other".into(),
            ]));
            assert_ne!(tests_key, ListingKey::new(&row, Scope::Row, true).unwrap());
        }
    }

    #[test]
    fn expected_selection_builds_exact_libtest_and_nextest_filters_without_widening() {
        let mut row: Control = toml::from_str(
            r#"
id = "selected"
guards = "only the exact guard executes"
file = "src/lib.rs"
old = "guard()"
new = "true"
test_file = "src/lib.rs"
runner = "cargo"
package = "fixture"
target = "--lib"
select = "expected"
ignored = "only"
expect_red = ["tests::guard", "tests::other"]
"#,
        )
        .unwrap();
        for runner in ["cargo", "nextest"] {
            row.runner = runner.into();
            for scope in [Scope::Row, Scope::Broad, Scope::Package] {
                let cmd = command(&row, "run", scope).unwrap();
                let args: Vec<_> = cmd
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect();
                assert!(args.contains(&"--lib".into()));
                assert!(!args.contains(&"--tests".into()));
                if runner == "cargo" {
                    assert!(args.ends_with(&[
                        "--ignored".into(),
                        "--exact".into(),
                        "tests::guard".into(),
                        "tests::other".into()
                    ]));
                } else {
                    let filter = args.windows(2).find(|args| args[0] == "-E").unwrap();
                    assert_eq!(filter[1], "test(=tests::guard) | test(=tests::other)");
                    assert!(args
                        .windows(2)
                        .any(|args| args == ["--run-ignored", "only"]));
                }
                let build = command(&row, "build", scope).unwrap();
                assert!(!build.get_args().any(|arg| arg == "--exact" || arg == "-E"));
                let list = command(&row, "list", scope).unwrap();
                assert!(!list.get_args().any(|arg| arg == "--exact" || arg == "-E"));
            }
        }
        row.expect_red = vec!["a),b\\c".into()];
        let cmd = command(&row, "run", Scope::Row).unwrap();
        assert!(cmd.get_args().any(|arg| arg == "test(=a\\)\\,b\\\\c)"));
    }

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
    fn ignored_selection_reaches_list_and_run_for_every_scope_but_not_build() {
        let mut c =
            toml::from_str::<Catalogue>(include_str!("../tests/fixture/mutations-v0.1.toml"))
                .unwrap()
                .control
                .remove(0);
        for scope in [
            Scope::Row,
            Scope::Broad,
            Scope::Package,
            Scope::Explore { workspace: false },
            Scope::Explore { workspace: true },
        ] {
            for (selection, libtest, nextest) in [
                (IgnoredSelection::Include, "--include-ignored", "all"),
                (IgnoredSelection::Only, "--ignored", "only"),
            ] {
                c.ignored = Some(selection);
                let args = |runner: &str, mode: &str| -> Vec<String> {
                    let mut c = c.clone();
                    c.runner = runner.into();
                    command(&c, mode, scope)
                        .unwrap()
                        .get_args()
                        .map(|s| s.to_string_lossy().into_owned())
                        .collect()
                };
                assert!(args("cargo", "list").ends_with(&[
                    "--".into(),
                    "--list".into(),
                    libtest.into()
                ]));
                assert!(args("cargo", "run").ends_with(&[
                    "--".into(),
                    "--test-threads=1".into(),
                    libtest.into()
                ]));
                for mode in ["list", "run"] {
                    let args = args("nextest", mode);
                    assert!(
                        args.windows(2).any(|w| w == ["--run-ignored", nextest]),
                        "{args:?}"
                    );
                    assert!(!args.iter().any(|a| a == "--"), "{args:?}");
                }
                for runner in ["cargo", "nextest"] {
                    let args = args(runner, "build");
                    assert!(
                        !args.iter().any(|a| a.contains("ignored")),
                        "a build selects no tests: {args:?}"
                    );
                }
            }
        }
        c.ignored = None;
        for runner in ["cargo", "nextest"] {
            c.runner = runner.into();
            for mode in ["build", "list", "run"] {
                let cmd = command(&c, mode, Scope::Row).unwrap();
                assert!(!cmd
                    .get_args()
                    .any(|a| a.to_string_lossy().contains("ignored")));
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
    fn cargo_captured_nested_runs_use_only_outer_outcomes_and_summary() {
        let text = include_str!("../tests/fixture/cargo-embedded.txt");
        // Both stdout and stderr block headings delimit captured output.
        for text in [
            text.to_owned(),
            text.replacen(" stdout ----", " stderr ----", 1),
        ] {
            let results = parse_test_results(&text, "cargo").unwrap();
            assert_eq!(results.red, ["tests::captures_child_run"]);
            assert_eq!(results.green, ["tests::shared_passes"]);
            assert_eq!(results.targets.len(), 2);
            let output = &results.failure_output["tests::captures_child_run"];
            assert!(output.contains("---- nested stdout ----"));
            assert!(output.contains("---- nested stderr ----"));
            assert!(output.contains("test result: FAILED. 0 passed; 1 failed;"));
            assert!(output.contains("parent assertion tail"));
        }
    }

    #[test]
    fn cargo_outer_inconsistency_is_not_hidden_by_embedded_summary() {
        let text = include_str!("../tests/fixture/cargo-embedded.txt")
            .replace("FAILED. 1 passed; 1 failed;", "FAILED. 2 passed; 1 failed;");
        assert!(parse_test_results(&text, "cargo")
            .unwrap_err()
            .contains("libtest summary counts disagree with per-test output"));
    }

    #[test]
    fn nextest_embedded_libtest_text_is_output_not_status_in_human_and_json() {
        let child = include_str!("../tests/fixture/cargo-embedded.txt");
        let human = format!("FAIL [ 0.01s] fixture tests::captures_child_run\n  stdout ───\n{child}\nPASS [ 0.01s] fixture tests::shared_passes\nSummary [ 0.02s] 2 tests run: 1 passed, 1 failed");
        let json = format!(
            "{}\n{}\n{}",
            serde_json::json!({"type": "test", "event": "failed", "name": "fixture::fixture$tests::captures_child_run", "stdout": child}),
            serde_json::json!({"type": "test", "event": "ok", "name": "fixture::fixture$tests::shared_passes"}),
            r#"{"type":"suite","event":"failed","passed":1,"failed":1}"#,
        );
        for text in [human, json] {
            let results = parse_test_results(&text, "nextest").unwrap();
            assert_eq!(results.red, ["tests::captures_child_run"]);
            assert_eq!(results.green, ["tests::shared_passes"]);
            assert!(results.failure_output["tests::captures_child_run"].contains(child.trim()));
        }
    }

    #[test]
    fn inline_output_flags_are_not_reachable_through_row_targets() {
        let mut c =
            toml::from_str::<Catalogue>(include_str!("../tests/fixture/mutations-v0.1.toml"))
                .unwrap()
                .control
                .remove(0);
        for target in [
            "--lib --nocapture",
            "--lib --show-output",
            "--lib -- --nocapture",
        ] {
            c.target = Some(target.into());
            assert!(c
                .validate_runner()
                .unwrap_err()
                .contains("invalid cargo target selector"));
        }
    }

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

    #[test]
    fn saved_restore_checks_original_bytes_and_reports_a_failed_write() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let original = b"original\r\nnon-utf8: \xff\n";
        let file = root.join("lib.rs");
        fs::write(&file, original).unwrap();
        let edit = Edit {
            file: "lib.rs".into(),
            old: "original".into(),
            new: "mutated".into(),
        };
        let mut saved = Saved::new(&root, std::slice::from_ref(&edit), true).unwrap();
        fs::write(&file, b"mutated").unwrap();
        saved.restore().unwrap();
        assert_eq!(fs::read(&file).unwrap(), original);

        let failed_file = root.join("unwritable.rs");
        fs::write(&failed_file, b"saved bytes").unwrap();
        let failed_edit = Edit {
            file: "unwritable.rs".into(),
            old: "saved".into(),
            new: "mutated".into(),
        };
        let mut failed = Saved::new(&root, &[failed_edit], true).unwrap();
        fs::remove_file(&failed_file).unwrap();
        fs::create_dir(&failed_file).unwrap();
        let error = failed.restore().unwrap_err();
        assert!(
            error.contains("restore failed") && error.contains("unwritable.rs"),
            "restore must name an unrestored source file: {error}"
        );
        failed.active = false;
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
            source: RowSource::default(),
            guards: "g".into(),
            file: Some(file.into()),
            old: Some("a".into()),
            new: Some("b".into()),
            scope: None,
            edits: vec![],
            test_file: file.into(),
            runner: "cargo".into(),
            package: Some(package.into()),
            target: None,
            features: None,
            no_default_features: None,
            all_features: None,
            ignored: None,
            select: None,
            command: None,
            list_command: None,
            broad_command: None,
            broad_report: None,
            broad_id: None,
            junit_class: None,
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

#[cfg(test)]
mod narrow_tests {
    use super::*;

    fn row() -> Control {
        toml::from_str(
            r#"
id = "guard"
guards = "reject zero"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "src/lib.rs"
runner = "cargo"
package = "fixture"
target = "--lib"
expect_red = ["tests::expected", "tests::other_expected"]
"#,
        )
        .unwrap()
    }

    fn args(c: &Control) -> Vec<String> {
        command(c, "run", Scope::Row)
            .unwrap()
            .get_args()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn run_wide_narrow_reuses_exact_expected_runner_arguments() {
        let mut c = row();
        assert_eq!(c.narrow(), NarrowSelection::Narrowed);
        assert_eq!(
            args(&c),
            [
                "test",
                "--locked",
                "-p",
                "fixture",
                "--lib",
                "--no-fail-fast",
                "--",
                "--test-threads=1",
                "--exact",
                "tests::expected",
                "tests::other_expected"
            ]
        );
        let mut explicit = row();
        explicit.select = Some(TestSelection::Expected);
        assert_eq!(args(&c), args(&explicit));
        assert_eq!(
            c.select.as_ref().unwrap().key(&c.expect_red),
            explicit.select.as_ref().unwrap().key(&explicit.expect_red)
        );
    }

    #[test]
    fn run_wide_narrow_preserves_all_own_selection_forms_and_skips() {
        for selection in [
            TestSelection::Expected,
            TestSelection::Filter("tests::".into()),
            TestSelection::Tests(vec!["tests::expected".into(), "tests::sibling".into()]),
        ] {
            let mut c = row();
            c.select = Some(selection.clone());
            let before = args(&c);
            assert_eq!(c.narrow(), NarrowSelection::OwnSelect);
            assert_eq!(c.select, Some(selection));
            assert_eq!(args(&c), before);
        }
        let mut only = row();
        only.only = true;
        let before = args(&only);
        assert_eq!(only.narrow(), NarrowSelection::Only);
        assert_eq!(args(&only), before);
        assert!(!args(&only).contains(&"--exact".into()));

        let mut skipped = row();
        skipped.desk_only = Some("needs interactive desktop".into());
        skipped.expect_red.clear();
        assert_eq!(skipped.narrow(), NarrowSelection::NotRun);
        assert_eq!(skipped.select, None);
    }
}
