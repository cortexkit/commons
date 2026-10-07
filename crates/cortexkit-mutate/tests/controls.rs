#![forbid(unsafe_code)]

use cortexkit_mutate::*;
use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            include_str!("fixture/Cargo.toml"),
        )
        .unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            include_str!("fixture/src/lib.rs"),
        )
        .unwrap();
        cmd(dir.path(), "cargo", &["generate-lockfile", "--offline"]);
        cmd(dir.path(), "git", &["init", "-q"]);
        cmd(
            dir.path(),
            "git",
            &["config", "user.email", "fixture@example.invalid"],
        );
        cmd(
            dir.path(),
            "git",
            &["config", "user.name", "Mutation fixture"],
        );
        fs::write(
            dir.path().join("test_contract.rs"),
            "// Test contract marker\n",
        )
        .unwrap();
        let f = Self { dir };
        let catalogue = Catalogue {
            control: vec![f.control()],
            ..Catalogue::default()
        };
        fs::write(
            f.root().join("mutations.toml"),
            toml::to_string(&catalogue).unwrap(),
        )
        .unwrap();
        f.commit();
        f
    }
    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn commit(&self) {
        cmd(
            self.root(),
            "git",
            &[
                "add",
                "Cargo.toml",
                "Cargo.lock",
                "src/lib.rs",
                "test_contract.rs",
                "mutations.toml",
            ],
        );
        cmd(
            self.root(),
            "git",
            &["commit", "--allow-empty", "-qm", "fixture"],
        );
    }
    fn control(&self) -> Control {
        Control {
            id: "guard".into(),
            guards: "zero is rejected".into(),
            file: Some("src/lib.rs".into()),
            old: Some("value > 0".into()),
            new: Some("value >= 0".into()),
            edits: vec![],
            test_file: "test_contract.rs".into(),
            runner: "cargo".into(),
            package: Some("mutation-fixture".into()),
            target: Some("--lib".into()),
            features: None,
            no_default_features: None,
            all_features: None,
            command: None,
            test_count_pattern: None,
            catch_on: None,
            output_normalize: vec![],
            expect_message: None,
            signal_is_catch: None,
            expect_red: vec!["tests::guard_rejects_zero".into()],
            only: true,
            equivalent: None,
            equivalent_guard: None,
            unreachable: None,
            desk_only: None,
            hub: None,
            hub_targets: None,
            platforms: None,
            timeout_s: 30,
            build_timeout_s: default_build_timeout(),
        }
    }
    fn run(&self, c: &Control) -> Report {
        let before = fs::read(self.root().join("src/lib.rs")).unwrap();
        let lock = fs::read(self.root().join("Cargo.lock")).unwrap();
        let _lock = TreeLock::acquire(self.root()).unwrap();
        let row = run_row(self.root(), c, false, &AtomicBool::new(false), false).unwrap();
        assert_eq!(
            before,
            fs::read(self.root().join("src/lib.rs")).unwrap(),
            "source restored for {:?}",
            row.outcome
        );
        assert_eq!(
            lock,
            fs::read(self.root().join("Cargo.lock")).unwrap(),
            "lock restored for {:?}",
            row.outcome
        );
        row
    }
    /// Explore one mutant through the library and assert the same byte
    /// restoration `run` assertions make, whatever the outcome.
    fn explore(&self, c: &Control, workspace: bool) -> Report {
        let before = fs::read(self.root().join("src/lib.rs")).unwrap();
        let lock = fs::read(self.root().join("Cargo.lock")).unwrap();
        let _lock = TreeLock::acquire(self.root()).unwrap();
        let row = explore_row(self.root(), c, false, &AtomicBool::new(false), workspace).unwrap();
        assert_eq!(
            before,
            fs::read(self.root().join("src/lib.rs")).unwrap(),
            "source restored after explore {:?}",
            row.outcome
        );
        assert_eq!(
            lock,
            fs::read(self.root().join("Cargo.lock")).unwrap(),
            "lock restored after explore {:?}",
            row.outcome
        );
        row
    }
    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_ck-mutate"))
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap()
    }
}
fn cmd(root: &Path, program: &str, args: &[&str]) {
    let out = Command::new(program)
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn change(c: &mut Control, old: &str, new: &str, expected: &str) {
    c.old = Some(old.into());
    c.new = Some(new.into());
    c.expect_red = vec![expected.into()];
}

const COMMAND_GUARD: &str = "script.tests.flows_rig.RigChecks.test_guard";
const COMMAND_OTHER: &str = "script.tests.flows_rig.RigChecks.test_other";
const COMMAND_VACUOUS: &str = "script.tests.flows_rig.RigChecks.test_vacuous";

fn python_available() -> bool {
    match Command::new("python3").arg("--version").output() {
        Ok(out) => {
            assert!(out.status.success(), "python3 exists but --version failed");
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("CK_MUTATE_REQUIRE_PYTHON").is_none(),
                "CK_MUTATE_REQUIRE_PYTHON is set but python3 is absent"
            );
            eprintln!("skipping command invocation fixture: python3 is absent");
            false
        }
        Err(e) => panic!("python3 could not execute: {e}"),
    }
}

struct CommandFixture {
    dir: TempDir,
}
impl CommandFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("guard.py"), b"ENABLED = True\n").unwrap();
        fs::write(root.join("rig.py"), include_str!("fixture/command.py")).unwrap();
        cmd(root, "git", &["init", "-q"]);
        cmd(
            root,
            "git",
            &["config", "user.email", "fixture@example.invalid"],
        );
        cmd(root, "git", &["config", "user.name", "Command fixture"]);
        cmd(root, "git", &["add", "guard.py", "rig.py"]);
        cmd(root, "git", &["commit", "-qm", "fixture"]);
        Self { dir }
    }
    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn control(&self) -> Control {
        Control {
            id: "command-guard".into(),
            guards: "the rig observes the guard".into(),
            file: Some("guard.py".into()),
            old: Some("ENABLED = True".into()),
            new: Some("ENABLED = False".into()),
            edits: vec![],
            test_file: "rig.py".into(),
            runner: "command".into(),
            package: None,
            target: None,
            features: None,
            no_default_features: None,
            all_features: None,
            command: Some(vec!["python3".into(), "rig.py".into(), "{test}".into()]),
            test_count_pattern: Some("Ran {count} tests".into()),
            catch_on: None,
            output_normalize: vec![],
            expect_message: None,
            signal_is_catch: None,
            expect_red: vec![COMMAND_GUARD.into()],
            only: false,
            equivalent: None,
            equivalent_guard: None,
            unreachable: None,
            desk_only: None,
            hub: None,
            hub_targets: None,
            platforms: None,
            timeout_s: 5,
            build_timeout_s: default_build_timeout(),
        }
    }
    fn snapshot(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        (
            fs::read(self.root().join("guard.py")).unwrap(),
            fs::read(self.root().join("rig.py")).unwrap(),
            git(self.root(), &["rev-parse", "HEAD^{tree}"]).unwrap(),
            git(self.root(), &["status", "--porcelain"]).unwrap(),
        )
    }
    fn run(&self, c: &Control, broad: bool) -> Report {
        let before = self.snapshot();
        let _lock = TreeLock::acquire(self.root()).unwrap();
        let stop = AtomicBool::new(false);
        let report = if broad {
            run_broad_row(self.root(), c, false, &stop)
        } else {
            run_row(self.root(), c, false, &stop, false)
        }
        .unwrap();
        assert_eq!(before, self.snapshot(), "source and Git tree must restore");
        git(self.root(), &["diff", "HEAD", "--exit-code"]).unwrap();
        assert_eq!(report.build_ms, 0);
        assert!(report.build_tail.is_empty());
        assert_eq!(report.collateral.count, 0);
        assert!(report.collateral.targets.is_empty());
        assert!(!report.breadth_observed);
        report
    }
    fn log(&self) -> String {
        fs::read_to_string(self.root().join(".git/command-log")).unwrap()
    }
    fn load_error(&self, c: &Control, field: &str) {
        let text = toml::to_string(&Catalogue {
            control: vec![c.clone()],
            ..Catalogue::default()
        })
        .unwrap();
        self.load_text_error(&text, field);
    }
    fn load_text_error(&self, text: &str, field: &str) {
        let before = self.snapshot();
        let path = self.root().join(".git/invalid.toml");
        fs::write(&path, text).unwrap();
        let error = load(&path)
            .err()
            .expect("malformed command row must fail at load");
        assert!(
            error.contains("command-guard") && error.contains(field),
            "{error}"
        );
        assert_eq!(before, self.snapshot());
    }
    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_ck-mutate"))
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap()
    }
}

#[test]
fn command_caught_fresh_baseline_each_id_and_restores() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    let mut c = f.control();
    c.expect_red.push(COMMAND_OTHER.into());
    validate(
        f.root(),
        &Catalogue {
            control: vec![c.clone()],
            ..Catalogue::default()
        },
    )
    .unwrap();
    for broad in [false, true] {
        let row = f.run(&c, broad);
        assert_eq!(row.outcome, Outcome::Caught, "{row:?}");
        assert_eq!(row.red, [COMMAND_GUARD, COMMAND_OTHER]);
        assert!(row.green.is_empty());
        // A green baseline's output is shown under a neutral heading; the
        // failure wording appears only when a baseline actually failed.
        assert!(row.test_tail.contains("baseline output:"), "{row:?}");
        assert!(!row.test_tail.contains("not green"), "{row:?}");
    }
    let one_run = format!("baseline {COMMAND_GUARD}\nbaseline {COMMAND_OTHER}\nmutant {COMMAND_GUARD}\nmutant {COMMAND_OTHER}\n");
    assert_eq!(
        f.log(),
        one_run.repeat(2),
        "baseline must run every id before mutation, on every replay"
    );
}

#[test]
fn command_zero_tests_on_baseline_or_mutant_is_error() {
    assert!(
        python_available(),
        "Python is required to verify command-row counts"
    );
    let f = CommandFixture::new();
    for id in ["zero_baseline", "zero_mutant"] {
        let mut c = f.control();
        c.expect_red = vec![id.into()];
        let row = f.run(&c, false);
        assert_eq!(row.outcome, Outcome::Error, "{id}: {row:?}");
        assert!(
            row.reason.as_ref().unwrap().contains("zero tests executed"),
            "{row:?}"
        );
        assert!(row.red.is_empty() && row.green.is_empty());
    }
}

#[test]
fn command_count_pattern_is_required_and_missing_or_ambiguous_output_is_error() {
    let f = CommandFixture::new();
    for pattern in [
        None,
        Some("count only"),
        Some("{count}"),
        Some("Ran {count} tests {count}"),
    ] {
        let mut c = f.control();
        c.test_count_pattern = pattern.map(str::to_owned);
        f.load_error(&c, "test_count_pattern");
    }
    assert!(
        python_available(),
        "Python is required to verify command-row counts"
    );
    let mut c = f.control();
    c.test_count_pattern = Some("Executed {count} tests".into());
    let row = f.run(&c, false);
    assert_eq!(row.outcome, Outcome::Error);
    assert!(row.reason.unwrap().contains("did not match"));
    let script = f.root().join("rig.py");
    let source = fs::read_to_string(&script).unwrap();
    fs::write(&script, format!("print('Ran 1 tests')\n{source}")).unwrap();
    cmd(f.root(), "git", &["add", "rig.py"]);
    cmd(f.root(), "git", &["commit", "-qm", "ambiguous output"]);
    let row = f.run(&f.control(), false);
    assert_eq!(row.outcome, Outcome::Error);
    assert!(row.reason.unwrap().contains("multiple counts"));
}

#[test]
fn command_rows_respect_platform_desk_and_prebuild_before_baseline_and_mutant() {
    assert!(
        python_available(),
        "Python is required to verify command-row prerequisites"
    );
    let f = CommandFixture::new();
    let mut c = f.control();
    c.platforms = Some(vec![if std::env::consts::OS == "macos" {
        "linux"
    } else {
        "macos"
    }
    .into()]);
    assert_eq!(f.run(&c, true).outcome, Outcome::SkippedPlatform);
    c.desk_only = Some("requires a physical desktop input session".into());
    assert_eq!(f.run(&c, true).outcome, Outcome::DeskOnly);
    assert!(!f.root().join(".git/command-log").exists());
    let path = f.root().join("rig.py");
    let source = fs::read_to_string(&path).unwrap();
    // Every test invocation needs the prerequisite matching the current source.
    fs::write(
        &path,
        source.replace(
            "test_id = sys.argv[1]",
            "assert Path('.git/fixture-output').read_text() == str(mutated)\ntest_id = sys.argv[1]",
        ),
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "rig.py"]);
    cmd(f.root(), "git", &["commit", "-qm", "fixture prerequisite"]);
    let mut second = f.control();
    second.id = "command-second".into();
    let cat = Catalogue { control: vec![f.control(), second], prebuild: vec![Prebuild {
        name: "command-fixture".into(),
        command: vec!["python3".into(), "-c".into(), "from pathlib import Path; v = str(Path('guard.py').read_bytes() == b'ENABLED = False\\n'); Path('.git/fixture-output').write_text(v); f = Path('.git/prebuild-runs').open('a', newline='\\n'); f.write(v + '\\n'); f.close(); print('fixture refreshed: ' + v)".into()],
        timeout_s: 5,
    }] };
    let path = f.root().join(".git/catalogue.toml");
    fs::write(&path, toml::to_string(&cat).unwrap()).unwrap();
    let output = f.cli(&[
        "--catalogue",
        ".git/catalogue.toml",
        "run",
        "--all",
        "--broad",
        "--report",
        ".git/report.json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join(".git/report.json")).unwrap()).unwrap();
    assert_eq!(report[0]["outcome"], "CAUGHT");
    assert_eq!(report[1]["outcome"], "CAUGHT");
    assert_eq!(
        fs::read_to_string(f.root().join(".git/prebuild-runs")).unwrap(),
        "False\nTrue\nTrue\nFalse\n"
    );
    assert_eq!(
        f.log(),
        format!("baseline {COMMAND_GUARD}\n").repeat(2)
            + &format!("mutant {COMMAND_GUARD}\n").repeat(2)
    );
}

#[test]
fn command_survived_if_any_expected_id_green_and_restores() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    let mut c = f.control();
    c.expect_red.push(COMMAND_VACUOUS.into());
    let row = f.run(&c, false);
    assert_eq!(row.outcome, Outcome::Survived, "{row:?}");
    assert_eq!(row.red, [COMMAND_GUARD]);
    assert_eq!(row.green, [COMMAND_VACUOUS]);
}

#[test]
fn command_baseline_not_green_is_error_and_restores() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    let mut c = f.control();
    c.expect_red.push("always_red".into());
    let row = f.run(&c, false);
    assert_eq!(row.outcome, Outcome::Error, "{row:?}");
    let reason = row.reason.unwrap();
    assert!(
        reason.contains("always_red") && reason.contains("baseline was not green"),
        "{reason}"
    );
    assert!(row.red.is_empty() && row.green.is_empty());
    assert!(row.failures.is_empty());
    assert_eq!(row.baseline_failures.len(), 1);
    assert!(row.baseline_failures["always_red"].contains("Ran 1 tests"));
    assert_eq!(
        f.log(),
        format!("baseline {COMMAND_GUARD}\nbaseline always_red\n")
    );
}

#[test]
fn command_invalid_process_is_error_not_red_and_restores() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    for id in ["exit127", "exit126"] {
        let mut c = f.control();
        c.expect_red = vec![id.into()];
        let row = f.run(&c, false);
        assert_eq!(row.outcome, Outcome::Error, "{row:?}");
        assert!(row.red.is_empty());
        let reason = row.reason.unwrap();
        assert!(
            reason.contains(if id == "exit126" {
                "exit 126: not executable, or the command refused to run"
            } else {
                "exit 127: program not found"
            }),
            "{reason}"
        );
    }
    let mut c = f.control();
    c.command.as_mut().unwrap()[0] = "./ck-mutate-program-that-does-not-exist".into();
    let row = f.run(&c, false);
    assert_eq!(row.outcome, Outcome::Error, "{row:?}");
    assert!(row.red.is_empty());
    assert!(row.reason.unwrap().contains("spawn/execution failed"));
    #[cfg(unix)]
    {
        let mut c = f.control();
        c.expect_red = vec!["signal".into()];
        let row = f.run(&c, false);
        assert_eq!(row.outcome, Outcome::Error, "{row:?}");
        assert!(row.red.is_empty());
        assert!(row.reason.unwrap().contains("signal"));
    }
}

#[test]
fn command_timeout_is_test_phase_error_and_restores() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    for id in ["sleep_mutant", "sleep_baseline"] {
        let mut c = f.control();
        c.expect_red = vec![id.into()];
        c.timeout_s = 1;
        let start = Instant::now();
        let row = f.run(&c, false);
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(row.outcome, Outcome::Error, "{row:?}");
        assert_eq!(row.timed_out_phase, Some(Phase::Test));
        assert!(row.red.is_empty());
        let reason = row.reason.unwrap();
        assert!(
            reason.contains(id) && reason.contains("timeout_s = 1"),
            "{reason}"
        );
    }
}

#[test]
fn command_load_rejects_mixed_and_malformed_fields() {
    let f = CommandFixture::new();
    for field in ["package", "target"] {
        let mut c = f.control();
        if field == "package" {
            c.package = Some(String::new());
        } else {
            c.target = Some(String::new());
        }
        f.load_error(&c, field);
    }
    for runner in ["cargo", "nextest"] {
        let mut c = f.control();
        c.runner = runner.into();
        c.package = Some("some-package".into());
        f.load_error(&c, "command");
    }
    let mut c = f.control();
    c.command = None;
    f.load_error(&c, "command");
    for argv in [vec![], vec!["", "{test}"]] {
        c.command = Some(argv.into_iter().map(str::to_owned).collect());
        f.load_error(&c, "command");
    }
    let text = toml::to_string(&Catalogue {
        control: vec![f.control()],
        ..Catalogue::default()
    })
    .unwrap();
    f.load_text_error(
        &text.replace(
            "command = [\"python3\", \"rig.py\", \"{test}\"]",
            "command = \"python3 rig.py {test}\"",
        ),
        "command",
    );
    f.load_text_error(&format!("{text}\nunknown_field = true\n"), "unknown_field");
}

#[test]
fn command_load_rejects_missing_placeholder() {
    let f = CommandFixture::new();
    let mut c = f.control();
    c.command.as_mut().unwrap()[2] = "a-test".into();
    f.load_error(&c, "command");
}

#[test]
fn command_load_rejects_repeated_placeholder() {
    let f = CommandFixture::new();
    for duplicate in ["{test}", "prefix{test}{test}"] {
        let mut c = f.control();
        if duplicate == "{test}" {
            c.command.as_mut().unwrap().push(duplicate.into());
        } else {
            c.command.as_mut().unwrap()[2] = duplicate.into();
        }
        f.load_error(&c, "command");
    }
}

#[test]
fn command_load_rejects_only() {
    let f = CommandFixture::new();
    let mut c = f.control();
    c.only = true;
    f.load_error(&c, "only");
}

#[test]
fn command_load_rejects_invalid_ids_without_normalizing_dotted_ids() {
    let f = CommandFixture::new();
    for id in [
        "",
        "has space",
        "has\ttab",
        "has\nnewline",
        "has\u{7f}control",
    ] {
        let mut c = f.control();
        c.expect_red = vec![id.into()];
        f.load_error(&c, "expect_red");
    }
    let c = f.control();
    let path = f.root().join(".git/valid.toml");
    fs::write(
        &path,
        toml::to_string(&Catalogue {
            control: vec![c.clone()],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    let loaded = load(&path).unwrap();
    assert_eq!(loaded.control[0], c);
    // Check mode does not invent a Cargo or test-list invocation for command rows.
    check(f.root(), &loaded, &AtomicBool::new(false)).unwrap();
    assert!(!f.root().join(".git/command-log").exists());
}

#[test]
fn command_explore_refuses_without_mutation() {
    let f = CommandFixture::new();
    let before = f.snapshot();
    let c = f.control();
    let error = explore_row(f.root(), &c, false, &AtomicBool::new(false), false).unwrap_err();
    assert!(error.contains("explore refuses command rows"), "{error}");
    let out = f.cli(&[
        "explore",
        "--runner",
        "command",
        "--package",
        "unused",
        "--file",
        "guard.py",
        "--old",
        "ENABLED = True",
        "--new",
        "ENABLED = False",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("explore refuses command rows"));
    assert_eq!(before, f.snapshot());
}

#[test]
fn command_prove_writes_canonical_row_and_restores() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    let before = f.snapshot();
    let out = f.cli(&[
        "--catalogue",
        ".git/proved.toml",
        "prove",
        "--id",
        "command-guard",
        "--guards",
        "the rig observes the guard",
        "--file",
        "guard.py",
        "--old",
        "ENABLED = True",
        "--new",
        "ENABLED = False",
        "--test-file",
        "rig.py",
        "--expect-red",
        COMMAND_GUARD,
        "--expect-red",
        COMMAND_OTHER,
        "--report",
        ".git/proof.json",
        "--test-count-pattern",
        "Ran {count} tests",
        "--command",
        "python3",
        "-u",
        "rig.py",
        "{test}",
    ]);
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let path = f.root().join(".git/proved.toml");
    let cat = load(&path).unwrap();
    assert_eq!(cat.control.len(), 1);
    let c = &cat.control[0];
    assert_eq!(c.runner, "command");
    assert_eq!(
        c.command.as_ref().unwrap(),
        &["python3", "-u", "rig.py", "{test}"]
    );
    assert_eq!(c.expect_red, [COMMAND_GUARD, COMMAND_OTHER]);
    assert_eq!(c.package, None);
    assert_eq!(c.target, None);
    let text = fs::read_to_string(path).unwrap();
    assert!(!text.contains("package =") && !text.contains("target ="));
    assert_eq!(text, format!("\n{}\n", toml::to_string(&cat).unwrap()));
    assert_eq!(before, f.snapshot());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join(".git/proof.json")).unwrap()).unwrap();
    assert_eq!(report[0]["outcome"], "CAUGHT");
    let broad = f.cli(&[
        "--catalogue",
        ".git/proved.toml",
        "run",
        "--all",
        "--broad",
        "--report",
        ".git/broad.json",
    ]);
    assert!(
        broad.status.success(),
        "{}",
        String::from_utf8_lossy(&broad.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join(".git/broad.json")).unwrap()).unwrap();
    assert_eq!(report[0]["outcome"], "CAUGHT");
    assert_eq!(report[0]["breadth_observed"], false);
    assert_eq!(report[0]["collateral"]["count"], 0);
    assert_eq!(before, f.snapshot());

    let survivor = f.cli(&[
        "--catalogue",
        ".git/survivor.toml",
        "prove",
        "--id",
        "command-guard",
        "--guards",
        "the rig observes the guard",
        "--file",
        "guard.py",
        "--old",
        "ENABLED = True",
        "--new",
        "ENABLED = False",
        "--test-file",
        "rig.py",
        "--expect-red",
        COMMAND_VACUOUS,
        "--report",
        ".git/survivor.json",
        "--test-count-pattern",
        "Ran {count} tests",
        "--command",
        "python3",
        "rig.py",
        "{test}",
    ]);
    assert!(!survivor.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join(".git/survivor.json")).unwrap()).unwrap();
    assert_eq!(
        report.as_array().unwrap().len(),
        1,
        "command survivors cannot replay a broader package"
    );
    assert_eq!(report[0]["outcome"], "SURVIVED");
    assert!(!f.root().join(".git/survivor.toml").exists());
    assert_eq!(before, f.snapshot());
}

#[test]
fn caught_all_failed_is_not_empty_and_restores() {
    let f = Fixture::new();
    let mut c = f.control();
    change(
        &mut c,
        "value > 0",
        "{ panic!(\"mutated\") }",
        "tests::guard_rejects_zero",
    );
    c.expect_red.push("tests::guard_accepts_positive".into());
    let r = f.run(&c);
    assert_eq!(r.outcome, Outcome::Caught);
    assert_eq!(r.red.len(), 2);
    assert!(!r.red.contains(&"result:".into()));
}

#[cfg(unix)]
#[test]
fn signal_abort_is_error_for_cargo_and_nextest() {
    let f = Fixture::new();
    for runner in ["cargo", "nextest"] {
        if !runner_available(runner) {
            continue;
        }
        let mut c = f.control();
        c.runner = runner.into();
        change(
            &mut c,
            "pub fn wait_hook() {}",
            "pub fn wait_hook() { std::process::abort(); }",
            "tests::waits",
        );
        let row = f.run(&c);
        eprintln!("ACTUAL {runner} OUTPUT:\n{}", row.test_tail);
        assert_eq!(row.outcome, Outcome::Error, "{row:?}");
        let reason = row.reason.as_deref().unwrap();
        assert!(
            reason.contains("SIGABRT") && reason.contains("tests::waits"),
            "{reason}"
        );
        assert!(!row.passes());
    }
}

fn runner_available(runner: &str) -> bool {
    if runner != "nextest" {
        return true;
    }
    let installed = Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        installed || std::env::var_os("CK_MUTATE_REQUIRE_NEXTEST").is_none(),
        "CI must install nextest"
    );
    installed
}

#[cfg(unix)]
#[test]
fn signal_abort_reason_allows_catch_for_cargo_and_nextest() {
    let f = Fixture::new();
    for runner in ["cargo", "nextest"] {
        if !runner_available(runner) {
            continue;
        }
        let mut c = f.control();
        c.runner = runner.into();
        change(
            &mut c,
            "pub fn wait_hook() {}",
            "pub fn wait_hook() { std::process::abort(); }",
            "tests::waits",
        );
        c.signal_is_catch = Some("This guard intentionally aborts on invalid input".into());
        let row = f.run(&c);
        assert_eq!(row.outcome, Outcome::Caught, "{row:?}");
        assert_eq!(row.red, ["tests::waits"]);
    }
}

#[test]
fn assertion_message_literal_and_regex_are_caught_for_both_runners() {
    let f = Fixture::new();
    for runner in ["cargo", "nextest"] {
        if !runner_available(runner) {
            continue;
        }
        let mut c = f.control();
        c.runner = runner.into();
        for pattern in [
            None,
            Some("assertion failed: !super::guarded(0)"),
            Some(r"/assertion failed: !super::guarded\(0\)/"),
        ] {
            c.expect_message = pattern.map(str::to_owned);
            let row = f.run(&c);
            assert_eq!(row.outcome, Outcome::Caught, "{row:?}");
        }
    }
}

#[test]
fn different_message_is_red_for_another_reason_for_both_runners() {
    let f = Fixture::new();
    for runner in ["cargo", "nextest"] {
        if !runner_available(runner) {
            continue;
        }
        let mut c = f.control();
        c.runner = runner.into();
        change(
            &mut c,
            "pub fn wait_hook() {}",
            "pub fn wait_hook() { panic!(\"wrong invariant\"); }",
            "tests::waits",
        );
        c.expect_message = Some("required invariant".into());
        let row = f.run(&c);
        assert_eq!(row.outcome, Outcome::RedForAnotherReason, "{row:?}");
        assert!(!row.passes());
        let reason = row.reason.unwrap();
        assert!(
            reason.contains("tests::waits") && reason.contains("wrong invariant"),
            "{reason}"
        );
        assert_eq!(
            serde_json::to_value(row.outcome).unwrap(),
            "RED_FOR_ANOTHER_REASON"
        );
    }
}

#[test]
fn long_run_retains_expected_and_collateral_failures_for_both_runners() {
    let panic_path = regex::Regex::new(r"src[\\/]lib\.rs:\d+:").unwrap();
    for runner in ["cargo", "nextest"] {
        if !runner_available(runner) {
            continue;
        }
        let f = Fixture::new();
        fs::create_dir_all(f.root().join("tests")).unwrap();
        let passing = (0..350)
            .map(|i| {
                format!(
                    "#[test] fn z_passing_{i:03}() {{ assert!(mutation_fixture::unrelated()); }}\n"
                )
            })
            .collect::<String>();
        fs::write(f.root().join("tests/zz_passes.rs"), passing).unwrap();
        // Serialize nextest and prioritize the two failures so the fixture always
        // leaves hundreds of PASS lines after their captured output.
        fs::create_dir_all(f.root().join(".config")).unwrap();
        fs::write(
            f.root().join(".config/nextest.toml"),
            "[profile.default]\ntest-threads = 1\n[[profile.default.overrides]]\nfilter = 'test(tests::guard_rejects_zero) | test(tests::waits)'\npriority = 100\n",
        )
        .unwrap();
        let mut c = f.control();
        c.runner = runner.into();
        c.target = Some("--tests".into());
        c.file = None;
        c.old = None;
        c.new = None;
        c.edits = vec![
            Edit { file: "src/lib.rs".into(), old: "value > 0".into(), new: "value >= 0".into() },
            Edit { file: "src/lib.rs".into(), old: "pub fn wait_hook() {}".into(), new: "pub fn wait_hook() { eprintln!(\"collateral setup\"); panic!(\"collateral invariant failed\"); }".into() },
        ];
        fs::write(
            f.root().join("mutations.toml"),
            toml::to_string(&Catalogue {
                control: vec![c.clone()],
                ..Catalogue::default()
            })
            .unwrap(),
        )
        .unwrap();
        cmd(f.root(), "git", &["add", "tests", ".config"]);
        f.commit();
        let row = f.run(&c);
        assert_eq!(row.outcome, Outcome::WrongTest, "{runner}: {row:?}");
        assert_eq!(row.green.len(), 354);
        assert_eq!(row.failures.len(), 2);
        assert!(row.baseline_failures.is_empty());
        assert!(
            !row.test_tail.contains("collateral invariant failed"),
            "{runner}: {}",
            row.test_tail
        );
        assert!(row.failures["tests::waits"].contains("collateral setup"));
        assert!(row.failures["tests::waits"].contains("collateral invariant failed"));
        assert!(!row.failures["tests::waits"].contains("z_passing_"));
        assert!(row.failures["tests::guard_rejects_zero"]
            .contains("assertion failed: !super::guarded(0)"));
        // Cargo's panic path may use either separator on Windows.
        assert!(panic_path.is_match(&row.failures["tests::waits"]));

        let out = f.cli(&["run", "--all", "--report", ".git/failures.json"]);
        assert!(!out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("guard: WRONG_TEST"), "{text}");
        assert!(text.contains("  unexpected red: tests::waits\n"), "{text}");
        assert!(text.contains("collateral invariant failed"), "{text}");
        assert!(
            !text.contains("  unexpected red: tests::guard_rejects_zero"),
            "{text}"
        );
        let json = report_json(&f, ".git/failures.json");
        assert!(json[0]["failures"]["tests::waits"]
            .as_str()
            .unwrap()
            .contains("collateral invariant failed"));
        assert_eq!(json[0]["baseline_failures"], serde_json::json!({}));
    }
}

#[test]
fn command_failure_maps_keep_both_streams_and_uncapped_message_proof() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    fs::write(
        f.root().join("rig.py"),
        r#"from pathlib import Path
import sys
# Python on Windows writes CRLF to text streams; the assertions below match LF.
sys.stdout.reconfigure(newline="\n")
sys.stderr.reconfigure(newline="\n")
mutated = Path("guard.py").read_bytes() == b"ENABLED = False\n"
test_id = sys.argv[1]
print("Ran 1 tests", flush=True)
if mutated:
    print("setup " + test_id, flush=True)
    for i in range(1, 10):
        print("detail " + str(i), flush=True)
    print("x" * 20000 + "MIDDLE_ONLY" + "x" * 20000, flush=True)
    print("panic " + test_id, file=sys.stderr, flush=True)
sys.exit(1 if mutated else 0)
"#,
    )
    .unwrap();
    let mut c = f.control();
    c.expect_red.push(COMMAND_OTHER.into());
    c.expect_message = Some("MIDDLE_ONLY".into());
    cmd(f.root(), "git", &["add", "rig.py"]);
    cmd(
        f.root(),
        "git",
        &["commit", "-qm", "verbose command fixture"],
    );
    let row = f.run(&c, false);
    assert_eq!(row.outcome, Outcome::Caught, "{row:?}");
    assert_eq!(row.failures.len(), 2);
    assert!(row.baseline_failures.is_empty());
    for (id, other) in [
        (COMMAND_GUARD, COMMAND_OTHER),
        (COMMAND_OTHER, COMMAND_GUARD),
    ] {
        let output = &row.failures[id];
        assert!(output.len() <= 16 * 1024);
        assert!(output.starts_with(&format!("Ran 1 tests\nsetup {id}\n")));
        assert!(output.trim_end().ends_with(&format!("panic {id}")));
        assert!(!output.contains(other));
        assert!(output.contains("bytes elided …]"));
        assert!(!output.contains("MIDDLE_ONLY"));
    }

    c.expect_message = Some("required invariant".into());
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![c],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "mutations.toml"]);
    cmd(
        f.root(),
        "git",
        &["commit", "-qm", "message mismatch fixture"],
    );
    let out = f.cli(&["run", "--all"]);
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("command-guard: RED_FOR_ANOTHER_REASON"),
        "{text}"
    );
    for id in [COMMAND_GUARD, COMMAND_OTHER] {
        let heading = format!("  unexpected red: {id}\n");
        let block = text.split_once(&heading).unwrap().1;
        assert!(block.starts_with(&format!("    Ran 1 tests\n    setup {id}\n    detail 1\n    detail 2\n    detail 3\n    detail 4\n")), "{block}");
        assert!(
            !block.lines().take(7).any(|line| line.contains("detail 5")),
            "{block}"
        );
    }
}

#[test]
fn expect_message_must_match_each_expected_test_not_another_tests_output() {
    let f = Fixture::new();
    for runner in ["cargo", "nextest"] {
        if !runner_available(runner) {
            continue;
        }
        let mut c = f.control();
        c.runner = runner.into();
        c.only = false;
        c.edits = vec![
            Edit {
                file: "src/lib.rs".into(),
                old: "value > 0".into(),
                new: "value >= 0".into(),
            },
            Edit {
                file: "src/lib.rs".into(),
                old: "pub fn wait_hook() {}".into(),
                new: "pub fn wait_hook() { panic!(\"assertion failed: !super::guarded(0)\"); }"
                    .into(),
            },
        ];
        c.file = None;
        c.old = None;
        c.new = None;
        c.expect_red.push("tests::waits".into());
        // The panic location names the guarded assertion's line; Windows prints
        // it with a backslash, so the pattern accepts either separator.
        c.expect_message = Some(r"/src[\\/]lib\.rs:23:/".into());
        let row = f.run(&c);
        assert_eq!(row.outcome, Outcome::RedForAnotherReason, "{row:?}");
        assert!(row.reason.unwrap().contains("tests::waits"));
    }
}

#[test]
fn command_expect_message_uses_each_ids_combined_output() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    let mut c = f.control();
    c.expect_red = vec![COMMAND_GUARD.into(), COMMAND_OTHER.into()];
    for pattern in ["guard assertion", r"/guard assert[a-z]+/"] {
        c.expect_message = Some(pattern.into());
        assert_eq!(f.run(&c, false).outcome, Outcome::Caught);
    }
    c.expect_message = Some(COMMAND_GUARD.into());
    let row = f.run(&c, false);
    assert_eq!(row.outcome, Outcome::RedForAnotherReason, "{row:?}");
    assert!(row.reason.unwrap().contains(COMMAND_OTHER));
}

#[cfg(unix)]
#[test]
fn command_signal_reason_allows_catch_but_not_a_failed_baseline() {
    if !python_available() {
        return;
    }
    let f = CommandFixture::new();
    let mut c = f.control();
    c.expect_red = vec!["signal".into()];
    let row = f.run(&c, false);
    assert_eq!(row.outcome, Outcome::Error);
    assert!(row.reason.unwrap().contains("signal 15"));
    c.signal_is_catch = Some("The command deliberately terminates on invalid input".into());
    assert_eq!(f.run(&c, false).outcome, Outcome::Caught);
    c.expect_red = vec!["always_red".into()];
    assert_eq!(f.run(&c, false).outcome, Outcome::Error);
}

#[test]
fn message_and_signal_fields_validate_and_round_trip() {
    let f = Fixture::new();
    let mut c = f.control();
    for (message, signal, field) in [
        (Some("/[/"), None, "expect_message"),
        (Some(" "), None, "expect_message"),
        (None, Some(" \n"), "signal_is_catch"),
    ] {
        c.expect_message = message.map(str::to_owned);
        c.signal_is_catch = signal.map(str::to_owned);
        assert!(validate_mutant(f.root(), &c).unwrap_err().contains(field));
    }
    c.expect_message = Some("/assertion failed/".into());
    c.signal_is_catch = Some("Intentional abort guard".into());
    append_control(&f.root().join("new.toml"), &c).unwrap();
    assert_eq!(load(&f.root().join("new.toml")).unwrap().control, [c]);
}

#[test]
fn prove_preserves_message_and_signal_options_in_the_catalogue() {
    let f = Fixture::new();
    let out = f.cli(&[
        "prove",
        "--id",
        "assertion-identity",
        "--guards",
        "zero is rejected",
        "--file",
        "src/lib.rs",
        "--old",
        "value > 0",
        "--new",
        "value >= 0",
        "--test-file",
        "test_contract.rs",
        "--package",
        "mutation-fixture",
        "--target=--lib",
        "--expect-red",
        "tests::guard_rejects_zero",
        "--expect-message",
        "assertion failed: !super::guarded(0)",
        "--signal-is-catch",
        "An abort is an intentional invalid-input guard",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows = load(&f.root().join("mutations.toml")).unwrap().control;
    let appended = rows.iter().find(|c| c.id == "assertion-identity").unwrap();
    assert_eq!(
        appended.expect_message.as_deref(),
        Some("assertion failed: !super::guarded(0)")
    );
    assert_eq!(
        appended.signal_is_catch.as_deref(),
        Some("An abort is an intentional invalid-input guard")
    );
}

fn add_collateral_targets(f: &Fixture) {
    fs::create_dir(f.root().join("tests")).unwrap();
    for (target, tests) in [("capacity", 2), ("readers", 1)] {
        let source: String = (0..tests)
            .map(|i| {
                format!(
                    "#[test]\nfn {target}_{i}() {{ assert!(!mutation_fixture::guarded(0)); }}\n"
                )
            })
            .collect();
        fs::write(f.root().join(format!("tests/{target}.rs")), source).unwrap();
    }
    cmd(f.root(), "git", &["add", "tests"]);
    f.commit();
}

fn collateral_run_fixture(target: &str, runner: &str) -> Fixture {
    let f = Fixture::new();
    add_collateral_targets(&f);
    let mut c = f.control();
    c.target = Some(target.into());
    c.runner = runner.into();
    c.only = false;
    c.new = Some("{ panic!(\"mutated\") }".into());
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![c],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    f.commit();
    f
}

fn collateral_cli(f: &Fixture, broad: bool) -> (serde_json::Value, String) {
    let mut args = vec!["run", "--all", "--report", "collateral.json"];
    if broad {
        args.push("--broad");
    }
    let out = f.cli(&args);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(f.root().join("src/lib.rs")).unwrap(),
        include_str!("fixture/src/lib.rs")
    );
    assert!(git(f.root(), &["diff", "--exit-code"]).unwrap().is_empty());
    (report_json(f, "collateral.json"), stdout)
}

#[test]
fn breadth_cross_target_catch_is_caught_broadly() {
    let f = collateral_run_fixture("--lib", "cargo");
    let (rows, stdout) = collateral_cli(&f, true);
    assert_eq!(rows[0]["outcome"], "CAUGHT_BROADLY");
    assert_eq!(rows[0]["breadth_observed"], true);
    assert_eq!(rows[0]["collateral"]["count"], 4);
    assert_eq!(
        rows[0]["collateral"]["targets"].as_array().unwrap().len(),
        3
    );
    assert_eq!(
        rows[0]["red"],
        serde_json::json!([
            "capacity_0",
            "capacity_1",
            "readers_0",
            "tests::guard_accepts_positive",
            "tests::guard_rejects_zero"
        ])
    );
    assert!(
        stdout.contains("guard: CAUGHT_BROADLY (warning; collateral: 4"),
        "{stdout}"
    );
    assert!(stdout.contains("1 CAUGHT_BROADLY (warning)"), "{stdout}");
    assert!(!stdout.contains("breadth not observed"), "{stdout}");
}

#[test]
fn breadth_normal_cross_target_proof_is_caught_without_observation() {
    let f = collateral_run_fixture("--lib", "cargo");
    let (rows, stdout) = collateral_cli(&f, false);
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(rows[0]["breadth_observed"], false);
    assert_eq!(rows[0]["collateral"]["count"], 1);
    assert_eq!(
        rows[0]["collateral"]["targets"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        rows[0]["red"],
        serde_json::json!(["tests::guard_accepts_positive", "tests::guard_rejects_zero"])
    );
    assert!(stdout.contains("breadth not observed"), "{stdout}");
    assert!(!stdout.contains("CAUGHT_BROADLY"), "{stdout}");

    // Even a normal selector that covers multiple binaries has not opted in to
    // the breadth audit. Its collateral is evidence, not a broad-catch grade.
    let f = collateral_run_fixture("--tests", "cargo");
    let (rows, _) = collateral_cli(&f, false);
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(rows[0]["breadth_observed"], false);
    assert_eq!(rows[0]["collateral"]["count"], 4);
}

#[test]
fn breadth_same_target_collateral_is_caught() {
    let f = Fixture::new();
    let mut c = f.control();
    c.only = false;
    c.new = Some("{ panic!(\"mutated\") }".into());
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![c],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    f.commit();
    let (rows, stdout) = collateral_cli(&f, true);
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(rows[0]["breadth_observed"], true);
    assert_eq!(rows[0]["collateral"]["count"], 1);
    assert_eq!(
        rows[0]["collateral"]["targets"].as_array().unwrap().len(),
        1
    );
    assert!(!stdout.contains("CAUGHT_BROADLY"), "{stdout}");
}

fn hub_load(fields: &str) -> std::result::Result<Catalogue, String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mutations.toml");
    fs::write(
        &path,
        format!(
            "{}\n{fields}\n",
            include_str!("fixture/mutations-v0.1.toml")
        ),
    )
    .unwrap();
    load(&path)
}

#[test]
fn hub_load_refuses_missing_reason_naming_row() {
    let error = hub_load("hub_targets = [\"capacity\"]").err().unwrap();
    assert!(
        error.contains("guard: HUB requires a hub reason"),
        "{error}"
    );
}

#[test]
fn hub_load_refuses_short_trimmed_reason_naming_row() {
    let error = hub_load("hub = '   nineteen characters   '\nhub_targets = ['capacity']")
        .err()
        .unwrap();
    assert!(
        error.contains("guard: HUB reason must be at least 20 characters after trimming"),
        "{error}"
    );
    let error = hub_load("hub = ' '\nhub_targets = ['capacity']")
        .err()
        .unwrap();
    assert!(error.contains("guard: HUB reason"), "{error}");
    let catalogue =
        hub_load("hub = '  twenty characters!!!  '\nhub_targets = ['capacity']").unwrap();
    assert_eq!(
        catalogue.control[0].hub.as_deref(),
        Some("  twenty characters!!!  ")
    );
}

#[test]
fn hub_load_refuses_empty_targets_naming_row() {
    let error = hub_load("hub = 'Multiple targets guard this shared property'\nhub_targets = []")
        .err()
        .unwrap();
    assert!(
        error.contains("guard: HUB requires non-empty hub_targets"),
        "{error}"
    );
}

#[test]
fn hub_load_refuses_missing_targets_naming_row() {
    let error = hub_load("hub = 'Multiple targets guard this shared property'")
        .err()
        .unwrap();
    assert!(
        error.contains("guard: HUB requires non-empty hub_targets"),
        "{error}"
    );
}

#[test]
fn hub_load_refuses_other_recorded_dispositions() {
    for other in ["equivalent", "unreachable"] {
        let guard = if other == "equivalent" {
            "\nequivalent_guard = 'The reviewed code fact'"
        } else {
            ""
        };
        let error = hub_load(&format!(
            "hub = 'Multiple targets guard this shared property'\nhub_targets = ['capacity']\n{other} = 'Reviewed explanation'{guard}"
        ))
        .err()
        .unwrap();
        assert!(
            error.contains("guard: hub, equivalent and unreachable are exclusive"),
            "{error}"
        );
    }
}

const HUB_REASON: &str = "Unit and integration tests deliberately guard the same zero refusal";

fn hub_run_fixture(targets: &[&str]) -> Fixture {
    let f = collateral_run_fixture("--lib", "cargo");
    let mut catalogue = load(&f.root().join("mutations.toml")).unwrap();
    catalogue.control[0].hub = Some(HUB_REASON.into());
    catalogue.control[0].hub_targets = Some(targets.iter().map(|t| (*t).into()).collect());
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&catalogue).unwrap(),
    )
    .unwrap();
    f.commit();
    f
}

#[test]
fn hub_broad_equal_targets_counts_as_hub() {
    let f = hub_run_fixture(&["capacity", "readers"]);
    let (rows, stdout) = collateral_cli(&f, true);
    assert_eq!(rows[0]["outcome"], "HUB");
    assert_eq!(rows[0]["reason"], HUB_REASON);
    assert_eq!(rows[0]["breadth_observed"], true);
    assert_eq!(rows[0]["collateral"]["count"], 4);
    assert_eq!(
        rows[0]["collateral"]["targets"],
        serde_json::json!(["capacity", "mutation_fixture", "readers"])
    );
    assert!(stdout.contains("guard: HUB (collateral: 4"), "{stdout}");
    assert!(stdout.contains("0 CAUGHT_BROADLY (warning)"), "{stdout}");
}

#[test]
fn hub_broad_subset_targets_counts_as_hub() {
    let f = hub_run_fixture(&["capacity", "readers", "other_contract"]);
    let (rows, _) = collateral_cli(&f, true);
    assert_eq!(rows[0]["outcome"], "HUB");
    assert_eq!(rows[0]["reason"], HUB_REASON);
}

#[test]
fn hub_broad_new_same_target_failure_still_counts_as_hub() {
    let f = hub_run_fixture(&["capacity", "readers"]);
    let (before, _) = collateral_cli(&f, true);
    assert_eq!(before[0]["collateral"]["count"], 4);

    let source = format!(
        "{}\n#[test]\nfn additional_same_target_rejects_zero() {{ assert!(!guarded(0)); }}\n",
        include_str!("fixture/src/lib.rs")
    );
    fs::write(f.root().join("src/lib.rs"), &source).unwrap();
    f.commit();
    let lock = fs::read(f.root().join("Cargo.lock")).unwrap();
    let c = load(&f.root().join("mutations.toml"))
        .unwrap()
        .control
        .remove(0);
    let row = run_broad_row(f.root(), &c, false, &AtomicBool::new(false)).unwrap();
    assert_eq!(row.outcome, Outcome::Hub);
    assert!(row.passes());
    assert_eq!(row.reason.as_deref(), Some(HUB_REASON));
    assert!(row
        .red
        .contains(&"additional_same_target_rejects_zero".into()));
    assert_eq!(row.collateral.count, 5);
    assert_eq!(
        row.collateral.targets,
        ["capacity", "mutation_fixture", "readers"]
    );
    assert_eq!(
        fs::read_to_string(f.root().join("src/lib.rs")).unwrap(),
        source
    );
    assert_eq!(fs::read(f.root().join("Cargo.lock")).unwrap(), lock);
    assert!(git(f.root(), &["diff", "--exit-code"]).unwrap().is_empty());
}

#[test]
fn hub_summary_counts_and_lists_reviewed_reasons() {
    let f = hub_run_fixture(&["capacity", "readers"]);
    let (_, stdout) = collateral_cli(&f, true);
    assert!(
        stdout.contains(
            "Summary: 0 CAUGHT, 0 CAUGHT_BROADLY (warning), 0 EQUIVALENT, 0 EQUIVALENT_CAUGHT, 0 UNREACHABLE, 1 HUB"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("HUB rows (1):\n  guard: {HUB_REASON}")),
        "{stdout}"
    );
}

#[test]
fn hub_broad_new_target_is_caught_broadly_and_named() {
    let f = hub_run_fixture(&["capacity"]);
    let (rows, stdout) = collateral_cli(&f, true);
    assert_eq!(rows[0]["outcome"], "CAUGHT_BROADLY");
    assert_eq!(
        rows[0]["reason"],
        "HUB collateral outside hub_targets: readers"
    );
    assert!(
        stdout.contains("HUB collateral outside hub_targets: readers"),
        "{stdout}"
    );
    assert!(!stdout.contains("HUB rows ("), "{stdout}");
}

#[test]
fn hub_is_ignored_without_broad() {
    let f = hub_run_fixture(&["capacity", "readers"]);
    let (rows, stdout) = collateral_cli(&f, false);
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(rows[0]["breadth_observed"], false);
    assert!(rows[0]["reason"].is_null());
    assert!(stdout.contains("breadth not observed"), "{stdout}");
    assert!(!stdout.contains("HUB"), "{stdout}");
}

#[test]
fn hub_broad_requires_expected_tests_to_fail() {
    let f = hub_run_fixture(&["capacity", "readers"]);
    let mut c = load(&f.root().join("mutations.toml"))
        .unwrap()
        .control
        .remove(0);
    c.new = Some("value >= 0".into());
    c.expect_red = vec!["tests::guard_accepts_positive".into()];
    let row = run_broad_row(f.root(), &c, false, &AtomicBool::new(false)).unwrap();
    assert_eq!(row.outcome, Outcome::WrongTest);
    assert!(!row.outcome.is_caught());
    assert!(!row.passes());
    assert!(row.reason.is_none());
    assert_eq!(
        fs::read_to_string(f.root().join("src/lib.rs")).unwrap(),
        include_str!("fixture/src/lib.rs")
    );
}

#[test]
fn hub_append_refuses_without_writing() {
    let f = hub_run_fixture(&["capacity", "readers"]);
    let path = f.root().join("mutations.toml");
    let c = load(&path).unwrap().control.remove(0);
    let before = fs::read(&path).unwrap();
    let error = append_control(&path, &c).unwrap_err();
    assert!(error.contains("guard: cannot append HUB"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
    let absent = f.root().join("new-catalogue.toml");
    assert!(append_control(&absent, &c).is_err());
    assert!(!absent.exists());
}

#[test]
fn nextest_broad_replay_observes_cross_target_collateral_when_installed() {
    let installed = Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !installed {
        assert!(
            std::env::var_os("CK_MUTATE_REQUIRE_NEXTEST").is_none(),
            "CI must install nextest"
        );
        eprintln!("nextest unavailable: skipping actual nextest broad replay");
        return;
    }
    let f = collateral_run_fixture("--lib", "nextest");
    let (rows, _) = collateral_cli(&f, true);
    assert_eq!(rows[0]["outcome"], "CAUGHT_BROADLY");
    assert_eq!(rows[0]["breadth_observed"], true);
    assert_eq!(rows[0]["collateral"]["count"], 4);
    assert_eq!(
        rows[0]["collateral"]["targets"].as_array().unwrap().len(),
        3
    );
}

#[test]
fn prove_reports_cross_target_collateral_without_broad_grading() {
    let f = Fixture::new();
    add_collateral_targets(&f);
    let out = f.cli(&[
        "prove",
        "--id",
        "broad",
        "--guards",
        "zero is rejected",
        "--file",
        "src/lib.rs",
        "--old",
        "value > 0",
        "--new",
        "{ panic!(\"mutated\") }",
        "--test-file",
        "test_contract.rs",
        "--package",
        "mutation-fixture",
        "--expect-red",
        "tests::guard_rejects_zero",
        "--report",
        "broad.json",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows = report_json(&f, "broad.json");
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(rows[0]["breadth_observed"], false);
    assert_eq!(rows[0]["collateral"]["count"], 4);
    let targets: Vec<_> = rows[0]["collateral"]["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(targets.len(), 3, "two capacity tests share a single target");
    assert_eq!(targets, ["capacity", "mutation_fixture", "readers"]);
    assert!(
        stdout.contains("broad: CAUGHT (collateral: 4 tests in targets:"),
        "{stdout}"
    );
    assert!(stdout.contains("breadth not observed"), "{stdout}");
    assert!(!stdout.contains("CAUGHT_BROADLY"), "{stdout}");
    let catalogue = load(&f.root().join("mutations.toml")).unwrap();
    let control = catalogue
        .control
        .iter()
        .find(|c| c.id == "broad")
        .expect("collateral still records the proof");
    let row = f.run(control);
    assert_eq!(row.outcome, Outcome::Caught);
    assert!(!row.breadth_observed);
    assert!(row.passes());
    assert!(row.outcome.is_caught());
    assert_eq!(
        fs::read_to_string(f.root().join("src/lib.rs")).unwrap(),
        include_str!("fixture/src/lib.rs")
    );
}

#[test]
fn collateral_same_target_is_caught_with_count() {
    let f = Fixture::new();
    let mut c = f.control();
    c.only = false;
    c.new = Some("{ panic!(\"mutated\") }".into());
    let row = f.run(&c);
    assert_eq!(row.outcome, Outcome::Caught);
    assert_eq!(row.collateral.count, 1);
    assert_eq!(row.collateral.targets.len(), 1);
    assert_eq!(row.collateral.targets[0], "mutation_fixture");
    let json = serde_json::to_value(&row).unwrap();
    assert_eq!(json["collateral"]["count"], 1);
    assert_eq!(json["collateral"]["targets"].as_array().unwrap().len(), 1);
}

#[test]
fn shared_test_names_keep_each_binary_identity_and_require_qualified_expectations() {
    let f = Fixture::new();
    fs::create_dir_all(f.root().join("src/bin")).unwrap();
    let source = r#"fn main() {}
#[cfg(test)] mod tests {
    #[test] fn shared() { assert!(!mutation_fixture::guarded(0)); }
}
"#;
    for bin in ["one", "two"] {
        fs::write(f.root().join(format!("src/bin/{bin}.rs")), source).unwrap();
    }
    cmd(f.root(), "git", &["add", "src/bin"]);
    f.commit();
    let mut c = f.control();
    c.only = false;
    c.target = None;
    c.expect_red = vec!["one::tests::shared".into()];
    let row = run_broad_row(f.root(), &c, false, &AtomicBool::new(false)).unwrap();
    assert_eq!(row.outcome, Outcome::CaughtBroadly, "{row:?}");
    assert!(row.red.contains(&"one::tests::shared".into()), "{row:?}");
    assert!(row.red.contains(&"two::tests::shared".into()), "{row:?}");
    assert_eq!(row.collateral.count, 2);
    assert_eq!(row.collateral.targets, ["mutation_fixture", "two"]);
    let package = run_row(f.root(), &c, false, &AtomicBool::new(false), true).unwrap();
    assert_eq!(package.outcome, Outcome::Caught);
    assert_eq!(package.red, row.red);
    let explored = f.explore(&c, false);
    assert_eq!(explored.red, row.red);

    let cat = Catalogue {
        control: vec![c.clone()],
        ..Catalogue::default()
    };
    check(f.root(), &cat, &AtomicBool::new(false)).unwrap();
    c.target = Some("--bin one".into());
    assert_eq!(
        f.run(&c).outcome,
        Outcome::Caught,
        "qualified ids work in narrow targets too"
    );
    c.expect_red = vec!["tests::shared".into()];
    let narrow = Catalogue {
        control: vec![c.clone()],
        ..Catalogue::default()
    };
    check(f.root(), &narrow, &AtomicBool::new(false)).unwrap();
    assert_eq!(f.run(&c).outcome, Outcome::Caught);
    // Ambiguity applies to binaries in the selection, not excluded siblings.
    c.target = None;
    let cat = Catalogue {
        control: vec![c.clone()],
        ..Catalogue::default()
    };
    let error = check(f.root(), &cat, &AtomicBool::new(false)).unwrap_err();
    assert!(
        error.contains("ambiguous expect_red name tests::shared"),
        "{error}"
    );
    assert!(
        error.contains("one::tests::shared") && error.contains("two::tests::shared"),
        "{error}"
    );
    let refused = f.run(&c);
    assert_eq!(refused.outcome, Outcome::Error, "{refused:?}");
    assert!(refused.reason.unwrap().contains("choose a qualified name"));
    assert_eq!(
        fs::read_to_string(f.root().join("src/lib.rs")).unwrap(),
        include_str!("fixture/src/lib.rs")
    );

    // A sibling with the same name can stay green; qualification must preserve
    // this independent result rather than letting the failing binary overwrite it.
    fs::write(
        f.root().join("src/bin/two.rs"),
        source.replace(
            "!mutation_fixture::guarded(0)",
            "mutation_fixture::guarded(1)",
        ),
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "src/bin/two.rs"]);
    f.commit();
    c.target = None;
    c.expect_red = vec!["two::tests::shared".into()];
    let mixed = f.run(&c);
    assert_eq!(mixed.outcome, Outcome::WrongTest, "{mixed:?}");
    assert!(mixed.red.contains(&"one::tests::shared".into()));
    assert!(mixed.green.contains(&"two::tests::shared".into()));
}

#[test]
fn collateral_none_reports_zero_and_empty_targets() {
    let f = Fixture::new();
    let row = f.run(&f.control());
    assert_eq!(row.outcome, Outcome::Caught);
    assert_eq!(row.collateral.count, 0);
    assert!(row.collateral.targets.is_empty());
    assert_eq!(
        serde_json::to_value(&row).unwrap()["collateral"],
        serde_json::json!({"count": 0, "targets": []})
    );
}

#[test]
fn unreachable_requires_reason_is_recorded_and_listed_separately() {
    let f = Fixture::new();
    let mut c = f.control();
    c.unreachable = Some(" \n".into());
    assert!(validate(
        f.root(),
        &Catalogue {
            control: vec![c.clone()],
            ..Catalogue::default()
        }
    )
    .is_err());
    assert!(
        explore_row(f.root(), &c, false, &AtomicBool::new(false), false)
            .unwrap_err()
            .contains("non-empty reason")
    );
    let reason = "guarded has no production caller; its only references are unit tests";
    c.unreachable = Some(reason.into());
    validate(
        f.root(),
        &Catalogue {
            control: vec![c.clone()],
            ..Catalogue::default()
        },
    )
    .unwrap();
    for row in [f.run(&c), f.explore(&c, false)] {
        assert_eq!(row.outcome, Outcome::Unreachable);
        assert!(
            !row.outcome.is_caught(),
            "UNREACHABLE is not proof of a catch"
        );
        assert!(row.passes(), "a justified recorded disposition succeeds");
        assert_eq!(row.reason.as_deref(), Some(reason));
        assert!(row.red.is_empty() && row.green.is_empty());
        assert_eq!(row.build_ms, 0);
        assert_eq!(row.test_ms, 0);
    }
    c.equivalent = Some("redundant condition".into());
    assert!(validate(
        f.root(),
        &Catalogue {
            control: vec![c],
            ..Catalogue::default()
        }
    )
    .is_err());
    let out = f.cli(&explore_args(
        "value > 0",
        "value >= 0",
        &[
            "--unreachable",
            reason,
            "--append",
            "--id",
            "dead-guard",
            "--guards",
            "zero rejected",
            "--test-file",
            "src/lib.rs",
            "--report",
            "unreachable.json",
        ],
    ));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("0 CAUGHT, 0 CAUGHT_BROADLY (warning), 0 EQUIVALENT, 0 EQUIVALENT_CAUGHT, 1 UNREACHABLE"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("UNREACHABLE rows (1):\n  dead-guard: {reason}")),
        "{stdout}"
    );
    let rows = report_json(&f, "unreachable.json");
    assert_eq!(rows[0]["outcome"], "UNREACHABLE");
    assert_eq!(rows[0]["reason"], reason);
    let catalogue = load(&f.root().join("mutations.toml")).unwrap();
    let appended = catalogue
        .control
        .iter()
        .find(|c| c.id == "dead-guard")
        .unwrap();
    assert_eq!(appended.unreachable.as_deref(), Some(reason));
    assert!(appended.expect_red.is_empty());
    assert!(f.cli(&["check"]).status.success());
    let run = f.cli(&["run", "--only", "dead-guard"]);
    assert!(run.status.success());
    assert!(String::from_utf8_lossy(&run.stdout).contains("UNREACHABLE rows (1):\n  dead-guard:"));
}

#[test]
fn previous_format_catalogue_proof_remains_readable() {
    let f = Fixture::new();
    fs::write(
        f.root().join("mutations.toml"),
        include_str!("fixture/mutations-v0.1.toml"),
    )
    .unwrap();
    let catalogue = load(&f.root().join("mutations.toml"))
        .expect("unversioned 0.1 proof catalogue must still load");
    validate(f.root(), &catalogue).unwrap();
    assert_eq!(catalogue.control.len(), 1);
    let c = &catalogue.control[0];
    assert!(c.unreachable.is_none());
    assert!(c.equivalent.is_none());
    assert_eq!(c.timeout_s, 600);
    assert_eq!(c.build_timeout_s, 1800);
    assert_eq!(f.run(c).outcome, Outcome::Caught);
    let mut next = c.clone();
    next.id = "new-proof".into();
    append_control(&f.root().join("mutations.toml"), &next).unwrap();
    let reloaded = load(&f.root().join("mutations.toml")).unwrap();
    assert_eq!(reloaded.control.len(), 2);
    assert_eq!(reloaded.control[0], *c);
}
#[test]
fn survived_vacuous_test_restores() {
    let f = Fixture::new();
    let mut c = f.control();
    change(
        &mut c,
        "pub fn vacuous() -> bool {\n    true\n}",
        "pub fn vacuous() -> bool { false }",
        "tests::vacuous_test",
    );
    assert_eq!(f.run(&c).outcome, Outcome::Survived);
}
#[test]
fn wrong_test_expected_green_and_unrelated_red_restores() {
    let f = Fixture::new();
    let mut c = f.control();
    change(
        &mut c,
        "pub fn unrelated() -> bool {\n    true\n}",
        "pub fn unrelated() -> bool { false }",
        "tests::vacuous_test",
    );
    let r = f.run(&c);
    assert_eq!(r.outcome, Outcome::WrongTest);
    assert_eq!(r.red, ["tests::unrelated_test"]);
}
#[test]
fn only_rejects_additional_failure() {
    let f = Fixture::new();
    let mut c = f.control();
    c.file = None;
    c.old = None;
    c.new = None;
    c.edits = vec![
        Edit {
            file: "src/lib.rs".into(),
            old: "value > 0".into(),
            new: "value >= 0".into(),
        },
        Edit {
            file: "src/lib.rs".into(),
            old: "pub fn unrelated() -> bool {\n    true\n}".into(),
            new: "pub fn unrelated() -> bool { false }".into(),
        },
    ];
    assert_eq!(f.run(&c).outcome, Outcome::WrongTest);
    c.only = false;
    assert_eq!(f.run(&c).outcome, Outcome::Caught);
}
#[test]
fn anchor_missing_zero_and_two_restore() {
    let f = Fixture::new();
    let mut c = f.control();
    c.old = Some("not in source".into());
    assert_eq!(f.run(&c).outcome, Outcome::AnchorMissing);
    c.old = Some("    true".into());
    assert_eq!(f.run(&c).outcome, Outcome::AnchorMissing);
}
#[test]
fn syntax_error_is_did_not_compile_and_restores() {
    let f = Fixture::new();
    let mut c = f.control();
    c.new = Some("this is not rust !".into());
    assert_eq!(f.run(&c).outcome, Outcome::DidNotCompile);
}
#[test]
fn missing_exact_name_is_no_tests_ran_and_restores() {
    let f = Fixture::new();
    let mut c = f.control();
    c.expect_red = vec!["guard_rejects_zero".into()];
    assert_eq!(f.run(&c).outcome, Outcome::NoTestsRan);
}
#[test]
fn empty_libtest_and_word_based_summary() {
    let f = Fixture::new();
    let c = f.control();
    let (red, green) = parse_tests("test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s", "cargo").unwrap();
    assert_eq!(grade(&c, &red, &green), Outcome::NoTestsRan);
    let (red, green) = parse_tests("test history::some_test ... FAILED\ntest result: FAILED. 2 ignored; 0 measured; 0 filtered out; 1 failed; 0 passed;", "cargo").unwrap();
    assert_eq!(red, ["history::some_test"]);
    assert!(green.is_empty());
    assert!(parse_tests("test result: ok. wording changed", "cargo").is_err());
}
/// Give this fixture a build script that sleeps on every rebuild of the
/// library, so building any mutant provably takes longer than `secs`.
fn slow_build(f: &Fixture, secs: u64) {
    fs::write(
        f.root().join("build.rs"),
        format!(
            "fn main() {{\n    println!(\"cargo:rerun-if-changed=src/lib.rs\");\n    std::thread::sleep(std::time::Duration::from_secs({secs}));\n}}\n"
        ),
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "build.rs"]);
    f.commit();
}
#[test]
fn timeout_kills_process_group_and_restores() {
    let f = Fixture::new();
    // Every mutant build sleeps 4 s, twice the test deadline: if `timeout_s`
    // bounded the build, the build would time out before the test process
    // ever reached its sleep.
    slow_build(&f, 4);
    let mut c = f.control();
    change(
        &mut c,
        "pub fn wait_hook() {}",
        "pub fn wait_hook() { std::fs::write(\"wait-pid\", std::process::id().to_string()).unwrap(); std::thread::sleep(std::time::Duration::from_secs(60)); }",
        "tests::waits",
    );
    // Warm dependencies; the mutant build itself is bounded by build_timeout_s.
    cmd(f.root(), "cargo", &["test", "--no-run", "--locked"]);
    c.timeout_s = 2;
    let started = Instant::now();
    let row = f.run(&c);
    assert_eq!(row.outcome, Outcome::TimedOut);
    assert_eq!(row.timed_out_phase, Some(Phase::Test), "{row:?}");
    assert!(row
        .reason
        .as_deref()
        .unwrap()
        .contains("test run exceeded timeout_s = 2"));
    assert!(started.elapsed() < Duration::from_secs(50));
    #[cfg(unix)]
    {
        let pid =
            fs::read_to_string(f.root().join("wait-pid")).expect("test process reached sleep");
        let start = Instant::now();
        loop {
            let out = Command::new("ps")
                .args(["-p", pid.trim(), "-o", "stat="])
                .output()
                .unwrap();
            let state = String::from_utf8_lossy(&out.stdout);
            if state.trim().is_empty() || state.trim().starts_with('Z') {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "test descendant survived timeout: {pid} ({state})"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert!(TreeLock::acquire(f.root()).is_ok());
}
#[test]
fn build_timeout_is_reported_as_build_phase_and_restores() {
    let f = Fixture::new();
    slow_build(&f, 6);
    cmd(f.root(), "cargo", &["test", "--no-run", "--locked"]);
    let mut c = f.control();
    // Both deadlines are short so this row pins only that the build has a
    // deadline and the report names its phase. That `timeout_s` does not bound
    // the build is pinned by timeout_kills_process_group_and_restores.
    c.build_timeout_s = 2;
    c.timeout_s = 2;
    let row = f.run(&c);
    assert_eq!(row.outcome, Outcome::TimedOut, "{row:?}");
    assert_eq!(row.timed_out_phase, Some(Phase::Build), "{row:?}");
    assert!(row
        .reason
        .as_deref()
        .unwrap()
        .contains("build exceeded build_timeout_s = 2"));
    assert_eq!(
        serde_json::to_value(&row).unwrap()["timed_out_phase"],
        "build"
    );
    assert!(row.red.is_empty() && row.green.is_empty());
    assert!(TreeLock::acquire(f.root()).is_ok());
}
#[test]
fn dirty_target_refused_without_overwriting() {
    let f = Fixture::new();
    let c = f.control();
    fs::write(
        f.root().join("src/lib.rs"),
        format!("{}\n// local fix\n", include_str!("fixture/src/lib.rs")),
    )
    .unwrap();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    assert!(run_row(f.root(), &c, false, &AtomicBool::new(false), false)
        .unwrap_err()
        .contains("dirty target"));
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
    let _lock = TreeLock::acquire(f.root()).unwrap();
    assert_eq!(
        run_row(f.root(), &c, true, &AtomicBool::new(false), false)
            .unwrap()
            .outcome,
        Outcome::Caught
    );
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
}
#[test]
fn cargo_lock_listed_edit_is_caught_and_restores() {
    let f = Fixture::new();
    fs::create_dir_all(f.root().join("dependency/src")).unwrap();
    fs::write(
        f.root().join("dependency/Cargo.toml"),
        "[package]\nname = \"lock-fixture-dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        f.root().join("dependency/src/lib.rs"),
        "pub fn guarded(value: i32) -> bool { value >= 0 }\n",
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "dependency"]);
    f.commit();

    let manifest_path = f.root().join("Cargo.toml");
    let lock_path = f.root().join("Cargo.lock");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let lock = fs::read_to_string(&lock_path).unwrap();
    let mutated_manifest =
        format!("{manifest}\n[dependencies]\nlock-fixture-dep = {{ path = \"dependency\" }}\n");
    fs::write(&manifest_path, &mutated_manifest).unwrap();
    // Resolve the real dependency graph before replay, so --locked can build it.
    cmd(f.root(), "cargo", &["generate-lockfile", "--offline"]);
    let mutated_lock = fs::read_to_string(&lock_path).unwrap();
    assert_ne!(lock, mutated_lock);
    fs::write(&manifest_path, &manifest).unwrap();
    fs::write(&lock_path, &lock).unwrap();

    let mut c = f.control();
    c.file = None;
    c.old = None;
    c.new = None;
    c.edits = vec![
        Edit {
            file: "Cargo.toml".into(),
            old: manifest.clone(),
            new: mutated_manifest,
        },
        Edit {
            file: "Cargo.lock".into(),
            old: lock.clone(),
            new: mutated_lock,
        },
        Edit {
            file: "src/lib.rs".into(),
            old: "value > 0".into(),
            new: "lock_fixture_dep::guarded(value)".into(),
        },
    ];
    let r = f.run(&c);
    assert_eq!(r.outcome, Outcome::Caught, "{:?}", r.reason);
    assert_eq!(r.red, ["tests::guard_rejects_zero"]);
    assert_eq!(manifest.as_bytes(), fs::read(&manifest_path).unwrap());
    assert_eq!(lock.as_bytes(), fs::read(&lock_path).unwrap());
}
#[test]
fn cargo_lock_change_fails_row_and_restores() {
    let f = Fixture::new();
    let mut c = f.control();
    change(
        &mut c,
        "pub fn lock_hook() {}",
        "pub fn lock_hook() { std::fs::write(\"Cargo.lock\", \"changed\").unwrap(); }",
        "tests::lock_integrity",
    );
    let r = f.run(&c);
    assert_eq!(r.outcome, Outcome::Error);
    assert!(r.reason.unwrap().contains("Cargo.lock changed during row"));
}
#[test]
fn multiline_anchor_is_replaced_exactly_once() {
    let f = Fixture::new();
    let mut c = f.control();
    change(
        &mut c,
        "pub fn guarded(value: i32) -> bool {\n    value > 0\n}",
        "pub fn guarded(value: i32) -> bool { value >= 0 }",
        "tests::guard_rejects_zero",
    );
    assert_eq!(f.run(&c).outcome, Outcome::Caught);
}
#[test]
fn lock_excludes_second_runner_and_releases() {
    let f = Fixture::new();
    let lock = TreeLock::acquire(f.root()).unwrap();
    assert!(TreeLock::acquire(f.root()).is_err());
    drop(lock);
    assert!(TreeLock::acquire(f.root()).is_ok());
}
#[test]
fn check_validates_names_anchors_and_fields_without_mutation() {
    let f = Fixture::new();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    assert!(f.cli(&["check"]).status.success());
    let mut c = f.control();
    c.expect_red = vec!["guard_rejects_zero".into()];
    let cat = Catalogue {
        control: vec![c.clone()],
        ..Catalogue::default()
    };
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&cat).unwrap(),
    )
    .unwrap();
    let out = f.cli(&["check"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("name no longer exists"));
    c.test_file = "missing.rs".into();
    assert!(validate(
        f.root(),
        &Catalogue {
            control: vec![c],
            ..Catalogue::default()
        }
    )
    .is_err());
    let duplicate = Catalogue {
        control: vec![f.control(), f.control()],
        ..Catalogue::default()
    };
    assert!(validate(f.root(), &duplicate).is_err());
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
}
#[test]
fn diff_selects_source_edits_test_file_and_changed_row() {
    for trigger in ["source", "edits", "test_file", "row", "unrelated"] {
        let f = Fixture::new();
        let mut c = f.control();
        if trigger == "edits" {
            c.edits = c.edits().unwrap();
            c.file = None;
            c.old = None;
            c.new = None;
        }
        let cat = Catalogue {
            control: vec![c.clone()],
            ..Catalogue::default()
        };
        fs::write(
            f.root().join("mutations.toml"),
            toml::to_string(&cat).unwrap(),
        )
        .unwrap();
        f.commit();
        let base = String::from_utf8(git(f.root(), &["rev-parse", "HEAD"]).unwrap()).unwrap();
        match trigger {
            "source" | "edits" => fs::write(
                f.root().join("src/lib.rs"),
                format!("{}\n// changed\n", include_str!("fixture/src/lib.rs")),
            )
            .unwrap(),
            "test_file" => fs::write(
                f.root().join("test_contract.rs"),
                "// changed test contract\n",
            )
            .unwrap(),
            "row" => {
                c.guards = "updated guard description".into();
                fs::write(
                    f.root().join("mutations.toml"),
                    toml::to_string(&Catalogue {
                        control: vec![c.clone()],
                        ..Catalogue::default()
                    })
                    .unwrap(),
                )
                .unwrap();
            }
            _ => fs::write(
                f.root().join("Cargo.toml"),
                format!("{}\n# unrelated\n", include_str!("fixture/Cargo.toml")),
            )
            .unwrap(),
        }
        f.commit();
        let selected = select_diff(
            f.root(),
            "mutations.toml",
            &Catalogue {
                control: vec![c],
                ..Catalogue::default()
            },
            base.trim(),
        )
        .unwrap();
        assert_eq!(
            selected.contains("guard"),
            trigger != "unrelated",
            "{trigger}"
        );
    }
}
#[test]
fn cli_report_shard_equivalent_and_prove_append() {
    let f = Fixture::new();
    let out = f.cli(&["run", "--all", "--shard", "1/1", "--report", "report.json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join("report.json")).unwrap()).unwrap();
    assert_eq!(json[0]["outcome"], "CAUGHT");
    assert_eq!(json[0]["red"][0], "tests::guard_rejects_zero");
    let mut c = f.control();
    c.equivalent = Some("identity transformation of a redundant condition".into());
    c.equivalent_guard = Some("guarded: strict comparison is unchanged by operand reversal".into());
    c.new = Some("0 < value".into());
    assert_eq!(f.run(&c).outcome, Outcome::Equivalent);
    let out = f.cli(&[
        "prove",
        "--id",
        "second",
        "--guards",
        "zero rejected",
        "--file",
        "src/lib.rs",
        "--old",
        "value > 0",
        "--new",
        "value >= 0",
        "--test-file",
        "test_contract.rs",
        "--package",
        "mutation-fixture",
        "--target=--lib",
        "--expect-red",
        "tests::guard_rejects_zero",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The proved row mutates inside `guarded`, so the hint names `guarded`.
    assert!(String::from_utf8_lossy(&out.stdout)
        .contains("Add a second row removing a call to `guarded`."));
    assert_eq!(
        load(&f.root().join("mutations.toml"))
            .unwrap()
            .control
            .len(),
        2
    );
}

#[test]
fn run_reports_in_sorted_id_order_whatever_the_execution_order() {
    let f = Fixture::new();
    let lib = format!("{}\npub mod other;\n", include_str!("fixture/src/lib.rs"));
    fs::write(f.root().join("src/lib.rs"), lib).unwrap();
    fs::write(
        f.root().join("src/other.rs"),
        "pub fn other_flag() -> bool { true }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn other_flag_holds() {\n        assert!(super::other_flag());\n    }\n}\n",
    )
    .unwrap();
    // `a-other` sorts first by ID, but its file sorts after `src/lib.rs`, so it
    // executes second.
    let mut other = f.control();
    other.id = "a-other".into();
    other.file = Some("src/other.rs".into());
    other.test_file = "src/other.rs".into();
    change(
        &mut other,
        "{ true }",
        "{ false }",
        "other::tests::other_flag_holds",
    );
    let catalogue = Catalogue {
        control: vec![f.control(), other],
        ..Catalogue::default()
    };
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&catalogue).unwrap(),
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "src/other.rs"]);
    f.commit();
    let out = f.cli(&["run", "--all", "--report", "order.json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "a-other: CAUGHT (breadth not observed; use run --broad to audit)\n\
         guard: CAUGHT (breadth not observed; use run --broad to audit)\n"
    );
    let json = report_json(&f, "order.json");
    assert_eq!(json[0]["id"], "a-other");
    assert_eq!(json[1]["id"], "guard");
    // Each row rewrites its file's original bytes when it finishes, so the
    // later modification time belongs to the row that executed last.
    let modified = |name: &str| {
        fs::metadata(f.root().join(name))
            .unwrap()
            .modified()
            .unwrap()
    };
    assert!(modified("src/other.rs") > modified("src/lib.rs"));
}

#[test]
fn prove_call_site_removal_prints_no_call_site_hint() {
    let f = Fixture::new();
    let caller_code = format!(
        "{}\npub fn caller_hook() -> bool {{ guarded(1) }}\n",
        include_str!("fixture/src/lib.rs")
    );
    fs::write(f.root().join("src/lib.rs"), caller_code).unwrap();
    fs::create_dir_all(f.root().join("tests")).unwrap();
    fs::write(
        f.root().join("tests/caller.rs"),
        "#[test]\nfn caller_checks() { assert!(mutation_fixture::caller_hook()); }\n",
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "src/lib.rs", "tests/caller.rs"]);
    f.commit();
    let out = f.cli(&[
        "prove",
        "--id",
        "caller-reaches-guard",
        "--guards",
        "caller reaches guard",
        "--file",
        "src/lib.rs",
        "--old",
        "{ guarded(1) }",
        "--new",
        "{ false }",
        "--test-file",
        "tests/caller.rs",
        "--package",
        "mutation-fixture",
        "--target=--test caller",
        "--expect-red",
        "caller_checks",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The mutation itself removes the call site, so `prove` has no call-site
    // hint to print: its stdout must be the report line and nothing else.
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "caller-reaches-guard: CAUGHT (breadth not observed; use run --broad to audit)\n"
    );
    assert_eq!(
        load(&f.root().join("mutations.toml"))
            .unwrap()
            .control
            .len(),
        2
    );
}
#[cfg(unix)]
const WAIT_STARTED: &str = "pub fn wait_hook() { std::fs::write(\"started\", \"ready\").unwrap(); std::thread::sleep(std::time::Duration::from_secs(60)); }";
#[cfg(unix)]
fn signal_restores(signal: rustix::process::Signal, args: &[&str]) {
    use std::process::Stdio;
    let f = Fixture::new();
    let mut c = f.control();
    change(
        &mut c,
        "pub fn wait_hook() {}",
        WAIT_STARTED,
        "tests::waits",
    );
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![c],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ck-mutate"))
        .current_dir(f.root())
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !f.root().join("started").exists() {
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_ne!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
    rustix::process::kill_process(rustix::process::Pid::from_child(&child), signal).unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
    assert!(TreeLock::acquire(f.root()).is_ok());
}
#[cfg(unix)]
#[test]
fn sigint_restores_and_releases_lock() {
    signal_restores(rustix::process::Signal::INT, &["run", "--all"]);
}
#[cfg(unix)]
#[test]
fn sigint_during_explore_restores_and_releases_lock() {
    signal_restores(
        rustix::process::Signal::INT,
        &[
            "explore",
            "--package",
            "mutation-fixture",
            "--file",
            "src/lib.rs",
            "--old",
            "pub fn wait_hook() {}",
            "--new",
            WAIT_STARTED,
        ],
    );
}
#[cfg(unix)]
#[test]
fn sigterm_restores_and_releases_lock() {
    signal_restores(rustix::process::Signal::TERM, &["run", "--all"]);
}
#[test]
fn nextest_status_parsing_is_full_name_not_exit_status() {
    let (red, green) = parse_tests(
        "FAIL [ 0.001s] fixture history::rejects\nPASS [ 0.001s] fixture history::accepts\nSummary [ 0.01s] 2 tests run: 1 passed, 1 failed",
        "nextest",
    )
    .unwrap();
    assert_eq!(red, ["history::rejects"]);
    assert_eq!(green, ["history::accepts"]);
}

#[test]
fn nextest_machine_replay_and_check_when_installed() {
    let installed = Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !installed {
        assert!(
            std::env::var_os("CK_MUTATE_REQUIRE_NEXTEST").is_none(),
            "CI must install nextest"
        );
        eprintln!("nextest unavailable: skipping actual nextest replay");
        return;
    }
    let f = Fixture::new();
    let mut c = f.control();
    c.runner = "nextest".into();
    let r = f.run(&c);
    assert_eq!(r.outcome, Outcome::Caught, "{r:?}");
    assert_eq!(r.red, ["tests::guard_rejects_zero"]);
    let _lock = TreeLock::acquire(f.root()).unwrap();
    check(
        f.root(),
        &Catalogue {
            control: vec![c],
            ..Catalogue::default()
        },
        &AtomicBool::new(false),
    )
    .unwrap();
}

#[test]
fn zero_tests_actual_binary_restores() {
    let f = Fixture::new();
    fs::create_dir(f.root().join("tests")).unwrap();
    fs::write(f.root().join("tests/empty.rs"), "// No tests by design.\n").unwrap();
    cmd(f.root(), "git", &["add", "tests/empty.rs"]);
    f.commit();
    let mut c = f.control();
    c.target = Some("--test empty".into());
    assert_eq!(f.run(&c).outcome, Outcome::NoTestsRan);
}

#[test]
fn all_tests_red_zero_green_actual_binary_restores() {
    let f = Fixture::new();
    fs::create_dir(f.root().join("tests")).unwrap();
    fs::write(
        f.root().join("tests/guard.rs"),
        "#[test]\nfn rejects_zero() { assert!(!mutation_fixture::guarded(0)); }\n",
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "tests/guard.rs"]);
    f.commit();
    let mut c = f.control();
    c.target = Some("--test guard".into());
    c.expect_red = vec!["rejects_zero".into()];
    let r = f.run(&c);
    assert_eq!(r.outcome, Outcome::Caught);
    assert_eq!(r.red, ["rejects_zero"]);
    assert!(r.green.is_empty());
}

#[test]
fn prove_survivor_replays_unscoped_package() {
    let f = Fixture::new();
    fs::create_dir(f.root().join("tests")).unwrap();
    fs::write(
        f.root().join("tests/vacuous.rs"),
        "#[test]\nfn vacuous() { let _ = mutation_fixture::guarded(0); }\n",
    )
    .unwrap();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    let out = f.cli(&[
        "prove",
        "--id",
        "scope",
        "--guards",
        "zero rejected",
        "--file",
        "src/lib.rs",
        "--old",
        "value > 0",
        "--new",
        "value >= 0",
        "--test-file",
        "test_contract.rs",
        "--package",
        "mutation-fixture",
        "--target=--test vacuous",
        "--expect-red",
        "vacuous",
        "--report",
        "survivor.json",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("original command scope omitted"));
    let rows: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join("survivor.json")).unwrap()).unwrap();
    assert_eq!(rows[0]["outcome"], "SURVIVED");
    assert_eq!(rows[1]["outcome"], "WRONG_TEST");
    assert_eq!(
        load(&f.root().join("mutations.toml"))
            .unwrap()
            .control
            .len(),
        1
    );
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
}

#[test]
fn reasoned_ignore_in_target_still_grades_caught() {
    let f = Fixture::new();
    let row = f.run(&f.control());
    assert_eq!(row.outcome, Outcome::Caught, "{row:?}");
    assert!(row
        .test_tail
        .contains("test tests::daemon_contract ... ignored, needs a daemon"));
    assert_eq!(row.red, ["tests::guard_rejects_zero"]);
    assert!(!row.green.contains(&"tests::daemon_contract".into()));
}

#[test]
fn deleted_target_reports_anchor_missing_and_next_cli_row_runs() {
    let f = Fixture::new();
    fs::write(f.root().join("deleted.rs"), "original guard\n").unwrap();
    cmd(f.root(), "git", &["add", "deleted.rs"]);
    f.commit();
    let base = String::from_utf8(git(f.root(), &["rev-parse", "HEAD"]).unwrap()).unwrap();
    fs::remove_file(f.root().join("deleted.rs")).unwrap();
    cmd(f.root(), "git", &["add", "deleted.rs"]);
    let mut deleted = f.control();
    deleted.id = "a-deleted".into();
    deleted.file = Some("deleted.rs".into());
    deleted.old = Some("original guard".into());
    deleted.new = Some("removed guard".into());
    let mut caught = f.control();
    caught.id = "b-caught".into();
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![deleted, caught],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    f.commit();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    let lock = fs::read(f.root().join("Cargo.lock")).unwrap();
    let out = f.cli(&[
        "run",
        "--diff",
        base.trim(),
        "--report",
        "deleted-report.json",
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(
        &fs::read(f.root().join("deleted-report.json")).expect("both rows must be reported"),
    )
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["id"], "a-deleted");
    assert_eq!(rows[0]["outcome"], "ANCHOR_MISSING");
    assert!(rows[0]["reason"].as_str().unwrap().contains("deleted.rs"));
    assert_eq!(rows[1]["id"], "b-caught");
    assert_eq!(rows[1]["outcome"], "CAUGHT");
    assert_eq!(rows[1]["red"][0], "tests::guard_rejects_zero");
    assert!(!f.root().join("deleted.rs").exists());
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
    assert_eq!(lock, fs::read(f.root().join("Cargo.lock")).unwrap());
    assert!(TreeLock::acquire(f.root()).is_ok());
}

#[test]
fn nextest_check_and_run_agree_on_exact_test_names() {
    let name = "nextest_check_and_run_agree_on_exact_test_names";
    let installed = Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !installed {
        assert!(
            std::env::var("CK_MUTATE_REQUIRE_NEXTEST").as_deref() != Ok("1"),
            "{name}: CI must install nextest"
        );
        eprintln!("SKIP {name}: nextest is not installed");
        return;
    }
    let f = Fixture::new();
    let mut c = f.control();
    c.runner = "nextest".into();
    c.expect_red.push("tests::guard_accepts_positive".into());
    c.new = Some("{ panic!(\"guard removed\") }".into());
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![c],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    let lock = fs::read(f.root().join("Cargo.lock")).unwrap();
    let checked = f.cli(&["check"]);
    assert!(
        checked.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let run = f.cli(&["run", "--all", "--report", "parity-report.json"]);
    assert!(
        run.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let rows: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join("parity-report.json")).unwrap()).unwrap();
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(
        rows[0]["red"],
        serde_json::json!(["tests::guard_accepts_positive", "tests::guard_rejects_zero"])
    );
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
    assert_eq!(lock, fs::read(f.root().join("Cargo.lock")).unwrap());
}

const FIXTURE_PACKAGE: &str = "mutation-fixture";

fn explore_args<'a>(old: &'a str, new: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "explore",
        "--package",
        FIXTURE_PACKAGE,
        "--file",
        "src/lib.rs",
        "--old",
        old,
        "--new",
        new,
        "--timeout-s",
        "120",
    ];
    args.extend_from_slice(extra);
    args
}

fn report_json(f: &Fixture, name: &str) -> serde_json::Value {
    serde_json::from_slice(&fs::read(f.root().join(name)).unwrap()).unwrap()
}

#[test]
fn explore_catches_and_lists_every_red_test_by_full_name() {
    let f = Fixture::new();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    let catalogue = fs::read(f.root().join("mutations.toml")).unwrap();
    let out = f.cli(&explore_args(
        "value > 0",
        "{ panic!(\"mutated\") }",
        &["--report", "explore.json"],
    ));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("explore: CAUGHT"), "{stdout}");
    assert!(
        stdout.contains("  tests::guard_accepts_positive\n"),
        "{stdout}"
    );
    assert!(stdout.contains("  tests::guard_rejects_zero\n"), "{stdout}");
    let rows = report_json(&f, "explore.json");
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(
        rows[0]["red"],
        serde_json::json!(["tests::guard_accepts_positive", "tests::guard_rejects_zero"])
    );
    assert!(rows[0]["green"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("tests::vacuous_test")));
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
    assert_eq!(
        catalogue,
        fs::read(f.root().join("mutations.toml")).unwrap(),
        "explore without --append never writes the catalogue"
    );
}

#[test]
fn explore_survivor_prints_diagnosis_and_never_appends() {
    let f = Fixture::new();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    let catalogue = fs::read(f.root().join("mutations.toml")).unwrap();
    let out = f.cli(&explore_args(
        "pub fn vacuous() -> bool {\n    true\n}",
        "pub fn vacuous() -> bool { false }",
        &[
            "--append",
            "--id",
            "vacuous-explored",
            "--guards",
            "vacuous returns true",
            "--test-file",
            "src/lib.rs",
            "--report",
            "survivor.json",
        ],
    ));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("vacuous-explored: SURVIVED"), "{stdout}");
    assert!(stdout.contains("unscoped by design"), "{stdout}");
    assert!(stdout.contains("Three causes"), "{stdout}");
    assert!(stdout.contains("The mutant is equivalent"), "{stdout}");
    assert!(
        stdout.contains("The guarding test lives outside the scope run"),
        "{stdout}"
    );
    assert!(stdout.contains("The guard is missing"), "{stdout}");
    assert!(stdout.contains("nothing appended"), "{stdout}");
    let rows = report_json(&f, "survivor.json");
    assert_eq!(rows[0]["outcome"], "SURVIVED");
    assert_eq!(rows[0]["red"], serde_json::json!([]));
    assert_eq!(
        catalogue,
        fs::read(f.root().join("mutations.toml")).unwrap(),
        "--append must not write on SURVIVED"
    );
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
}

#[test]
fn explore_append_writes_row_check_accepts_and_run_grades_caught() {
    let f = Fixture::new();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    let out = f.cli(&explore_args(
        "value > 0",
        "value >= 0",
        &[
            "--append",
            "--id",
            "zero-explored",
            "--guards",
            "zero is rejected",
            "--test-file",
            "src/lib.rs",
        ],
    ));
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
    let catalogue = load(&f.root().join("mutations.toml")).unwrap();
    assert_eq!(catalogue.control.len(), 2);
    let row = catalogue
        .control
        .iter()
        .find(|c| c.id == "zero-explored")
        .unwrap();
    assert_eq!(row.expect_red, ["tests::guard_rejects_zero"]);
    assert_eq!(row.package.as_deref(), Some(FIXTURE_PACKAGE));
    assert_eq!(row.target, None);
    assert_eq!(row.test_file, "src/lib.rs");
    let checked = f.cli(&["check"]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let run = f.cli(&["run", "--only", "zero-explored", "--report", "later.json"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let rows = report_json(&f, "later.json");
    assert_eq!(rows[0]["id"], "zero-explored");
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(before, fs::read(f.root().join("src/lib.rs")).unwrap());
}

#[test]
fn explore_restores_after_every_outcome() {
    let f = Fixture::new();
    let mut c = f.control();
    // explore ignores the row's target selector and expected names.
    c.expect_red = vec![];
    let caught = f.explore(&c, false);
    assert_eq!(caught.outcome, Outcome::Caught);
    assert_eq!(caught.red, ["tests::guard_rejects_zero"]);
    let workspace = f.explore(&c, true);
    assert_eq!(workspace.outcome, Outcome::Caught);
    assert_eq!(workspace.red, ["tests::guard_rejects_zero"]);

    let mut survived = c.clone();
    survived.old = Some("pub fn vacuous() -> bool {\n    true\n}".into());
    survived.new = Some("pub fn vacuous() -> bool { false }".into());
    let row = f.explore(&survived, false);
    assert_eq!(row.outcome, Outcome::Survived);
    assert!(row.red.is_empty() && !row.green.is_empty());

    let mut broken = c.clone();
    broken.new = Some("this is not rust !".into());
    assert_eq!(f.explore(&broken, false).outcome, Outcome::DidNotCompile);

    let mut missing = c.clone();
    missing.old = Some("    true".into());
    assert_eq!(f.explore(&missing, false).outcome, Outcome::AnchorMissing);

    let mut lockfile = c.clone();
    lockfile.old = Some("pub fn lock_hook() {}".into());
    lockfile.new =
        Some("pub fn lock_hook() { std::fs::write(\"Cargo.lock\", \"changed\").unwrap(); }".into());
    let row = f.explore(&lockfile, false);
    assert_eq!(row.outcome, Outcome::Error);
    assert!(row.reason.unwrap().contains("Cargo.lock changed"));

    let mut slow = c.clone();
    slow.old = Some("pub fn wait_hook() {}".into());
    slow.new = Some(
        "pub fn wait_hook() { std::thread::sleep(std::time::Duration::from_secs(60)); }".into(),
    );
    // Each build and test command gets its own timeout; the crate is already
    // built above, so the short timeout lands on the sleeping test process.
    slow.timeout_s = 5;
    let row = f.explore(&slow, false);
    assert_eq!(row.outcome, Outcome::TimedOut);
    assert_eq!(row.timed_out_phase, Some(Phase::Test));

    fs::write(
        f.root().join("src/lib.rs"),
        format!("{}\n// local fix\n", include_str!("fixture/src/lib.rs")),
    )
    .unwrap();
    let dirty = fs::read(f.root().join("src/lib.rs")).unwrap();
    assert!(
        explore_row(f.root(), &c, false, &AtomicBool::new(false), false)
            .unwrap_err()
            .contains("dirty target")
    );
    assert_eq!(dirty, fs::read(f.root().join("src/lib.rs")).unwrap());
}

#[test]
fn explore_grades_per_test_results_not_exit_status() {
    let f = Fixture::new();
    let mut c = f.control();
    c.expect_red = vec![];
    // The mutant compiles the unit tests away: the test command exits 0, but no
    // test ran, so nothing can have caught (or survived) anything.
    c.old = Some("#[cfg(test)]\nmod tests".into());
    c.new = Some("#[cfg(any())]\nmod tests".into());
    let row = f.explore(&c, false);
    assert_eq!(row.outcome, Outcome::NoTestsRan, "{row:?}");
    assert!(row.red.is_empty() && row.green.is_empty());
    // The mutant kills the test process mid-run: the command exits nonzero
    // although no test went red, so the result cannot be a catch.
    c.old = Some("pub fn wait_hook() {}".into());
    c.new = Some("pub fn wait_hook() { std::process::exit(1) }".into());
    let row = f.explore(&c, false);
    assert_eq!(row.outcome, Outcome::Error, "{row:?}");
    assert!(row.red.is_empty());
}

#[test]
fn explore_edits_accept_toml_and_json() {
    let toml_edits =
        parse_edits("[{ file = \"src/lib.rs\", old = \"value > 0\", new = \"value >= 0\" }]")
            .unwrap();
    let json_edits =
        parse_edits(r#"[{"file":"src/lib.rs","old":"value > 0","new":"value >= 0"}]"#).unwrap();
    let doc_edits = parse_edits(
        "[[edits]]\nfile = \"src/lib.rs\"\nold = \"value > 0\"\nnew = \"value >= 0\"\n",
    )
    .unwrap();
    assert_eq!(toml_edits, json_edits);
    assert_eq!(toml_edits, doc_edits);
    assert_eq!(toml_edits[0].old, "value > 0");
    assert!(parse_edits("[{ file = \"src/lib.rs\", typo = 1 }]").is_err());
}

/// `explore`, Cargo and command rows must not grow separate mutation paths. The library makes
/// this structural: `Saved` (target and Cargo.lock bytes), `execute` (the
/// build/test child) and `command` are private, and the only public functions
/// reaching mutation are thin calls into private `replay`. Session preparation
/// also snapshots the unmutated tree to protect it from prerequisite side effects,
/// but must never apply a mutant or grade tests. This test pins those boundaries,
/// then checks both replay entry points produce identical per-test results.
#[test]
fn explore_and_run_share_one_execution_and_restoration_path() {
    let lib = include_str!("../src/lib.rs");
    let lib = lib
        .split("#[cfg(test)]\nmod restoration_tests")
        .next()
        .unwrap();
    fn body<'a>(src: &'a str, signature: &str) -> &'a str {
        let start = src
            .find(signature)
            .unwrap_or_else(|| panic!("missing {signature}"));
        let closing = if signature.starts_with("    ") {
            "\n    }\n"
        } else {
            "\n}\n"
        };
        let end = src[start..].find(closing).expect("function end") + start;
        &src[start..end]
    }
    let replay = body(lib, "\nfn replay(");
    let prepare = body(lib, "    pub fn prepare(");
    let clean_prebuild = body(lib, "\nfn clean_prebuild(");
    for needle in [
        "Saved::new(",
        "saved.restore()",
        "replaced(root, &edits)",
        "command(c, \"build\", scope)",
        "command(c, \"run\", scope)",
        "parse_test_results(&tests.text",
        "command_tests(root, c, stop, &mut report, true)",
        "command_tests(root, c, stop, &mut report, false)",
    ] {
        // Each needle must appear exactly as often as listed. Clean-tree Cargo
        // baselines build, run and parse on their own path, and the session also
        // builds a cargo command line to key the per-selection baseline cache,
        // which is why some call sites appear twice.
        let expected = match needle {
            "Saved::new("
            | "saved.restore()"
            | "command(c, \"run\", scope)"
            | "parse_test_results(&tests.text" => 2,
            "command(c, \"build\", scope)" => 3,
            _ => 1,
        };
        assert_eq!(lib.matches(needle).count(), expected, "{needle} count");
        assert_eq!(
            replay.matches(needle).count(),
            1,
            "{needle} must occur once in replay"
        );
        assert!(replay.contains(needle), "{needle} must live in replay");
    }
    assert!(prepare.contains("clean_prebuild("));
    assert!(clean_prebuild.contains("Saved::new(") && clean_prebuild.contains("saved.restore()"));
    for forbidden in ["replaced(", "fs::write(", "command_tests(", "grade("] {
        assert!(
            !prepare.contains(forbidden),
            "unmutated preparation must not mutate or grade: {forbidden}"
        );
    }
    let baseline = replay
        .find("command_tests(root, c, stop, &mut report, true)")
        .unwrap();
    let mutation = replay.find("fs::write(path, text)").unwrap();
    let mutant = replay
        .find("command_tests(root, c, stop, &mut report, false)")
        .unwrap();
    assert!(
        baseline < mutation && mutation < mutant,
        "command baselines must precede the shared mutation and mutant tests must follow it"
    );
    let command_tests = body(lib, "\nfn command_tests(");
    assert!(command_tests.contains("execute(root, cmd, c.timeout_s, stop)"));
    for forbidden in ["Saved", "replaced(", "fs::write(", "spawn("] {
        assert!(
            !command_tests.contains(forbidden),
            "command tests must use the shared mutation and child supervisor: {forbidden}"
        );
    }
    for entry in [
        "pub fn run_row(",
        "pub fn run_broad_row(",
        "pub fn explore_row(",
    ] {
        let entry_body = body(lib, entry);
        assert!(
            entry_body
                .split_whitespace()
                .collect::<String>()
                .contains("replay(root,c,allow_dirty,stop,"),
            "{entry} must delegate to replay"
        );
        for forbidden in [
            "Saved",
            "execute(",
            "command(",
            "parse_tests",
            "fs::",
            "grade(",
        ] {
            assert!(
                !entry_body.contains(forbidden),
                "{entry} must not contain {forbidden}"
            );
        }
    }
    for private in [
        "\nstruct Saved",
        "\nfn execute(",
        "\nfn command(",
        "\nfn replay(",
        "\nfn command_tests(",
    ] {
        assert!(lib.contains(private), "{private} must stay private");
    }
    // The binary takes the tree lock and installs the signal flag once, before
    // dispatching any subcommand, and cannot spawn Cargo itself.
    let main = include_str!("../src/main.rs");
    assert_eq!(main.matches("TreeLock::acquire").count(), 1);
    assert_eq!(main.matches("signal_flag()").count(), 1);
    assert!(main.find("signal_flag()").unwrap() < main.find("match cli.command").unwrap());
    assert!(main.find("TreeLock::acquire").unwrap() < main.find("match cli.command").unwrap());
    assert_eq!(main.matches("explore_row(").count(), 1);
    assert!(!main.contains("Command::new"));

    let f = Fixture::new();
    let mut c = f.control();
    c.target = None;
    c.only = false;
    c.new = Some("{ panic!(\"mutated\") }".into());
    c.expect_red = vec![
        "tests::guard_accepts_positive".into(),
        "tests::guard_rejects_zero".into(),
    ];
    let ran = f.run(&c);
    let explored = f.explore(&c, false);
    assert_eq!(ran.outcome, Outcome::Caught);
    assert_eq!(explored.outcome, Outcome::Caught);
    assert_eq!(ran.red, explored.red);
    assert_eq!(ran.green, explored.green);
}

fn replace_fixture_source(f: &Fixture, old: &str, new: &str) {
    let path = f.root().join("src/lib.rs");
    let source = fs::read_to_string(&path).unwrap();
    assert!(source.contains(old));
    fs::write(path, source.replace(old, new)).unwrap();
    f.commit();
}

#[test]
fn expected_baseline_red_is_error_and_never_runs_mutant() {
    for runner in ["cargo", "nextest"] {
        if !runner_available(runner) {
            continue;
        }
        let f = Fixture::new();
        replace_fixture_source(
            &f,
            "assert!(!super::guarded(0));",
            "assert!(super::guarded(0));",
        );
        let mut c = f.control();
        c.runner = runner.into();
        c.new =
            Some("{ std::fs::write(\".git/mutant-ran\", \"yes\").unwrap(); value >= 0 }".into());
        let row = f.run(&c);
        assert_eq!(row.outcome, Outcome::Error, "{row:?}");
        assert_eq!(
            row.reason.as_deref(),
            Some("baseline red: tests::guard_rejects_zero")
        );
        assert!(!f.root().join(".git/mutant-ran").exists());
        assert_eq!(row.build_ms, 0);
        assert_eq!(row.test_ms, 0);
        assert!(row.baseline_test_ms > 0);
        assert!(row.failures.is_empty());
        assert_eq!(row.baseline_failures.len(), 1);
        assert!(row.baseline_failures["tests::guard_rejects_zero"]
            .contains("assertion failed: super::guarded(0)"));
    }
}

#[test]
fn unrelated_baseline_red_is_excluded_from_wrong_test_and_broad_catches() {
    for runner in ["cargo", "nextest"] {
        if !runner_available(runner) {
            continue;
        }
        let f = Fixture::new();
        replace_fixture_source(
            &f,
            "assert!(super::unrelated());",
            "assert!(!super::unrelated());",
        );
        fs::create_dir_all(f.root().join("tests")).unwrap();
        fs::write(
            f.root().join("tests/prebroken.rs"),
            "#[test] fn unrelated_test() { panic!(\"already broken\"); }",
        )
        .unwrap();
        cmd(f.root(), "git", &["add", "tests"]);
        f.commit();
        let mut c = f.control();
        c.runner = runner.into();
        for broad in [false, true] {
            let row = if broad {
                run_broad_row(f.root(), &c, false, &AtomicBool::new(false)).unwrap()
            } else {
                f.run(&c)
            };
            assert_eq!(row.outcome, Outcome::Caught, "{row:?}");
            assert_eq!(row.red, ["tests::guard_rejects_zero"]);
            assert_eq!(row.collateral.count, 0);
            assert!(row.baseline_failures["tests::unrelated_test"]
                .contains("assertion failed: !super::unrelated()"));
            assert!(row.failures["tests::unrelated_test"]
                .contains("assertion failed: !super::unrelated()"));
            assert!(row
                .baseline_red
                .values()
                .flatten()
                .any(|n| n == "tests::unrelated_test"));
            if broad {
                assert!(row.baseline_failures["unrelated_test"].contains("already broken"));
                assert!(row.failures["unrelated_test"].contains("already broken"));
                assert!(row
                    .baseline_red
                    .values()
                    .flatten()
                    .any(|n| n == "unrelated_test"));
            }
        }
        c.new = Some("0 < value".into());
        assert_eq!(f.run(&c).outcome, Outcome::Survived);
        // Through the CLI: every row's clean-tree baseline runs before any mutant,
        // and the summary names each test that was red at baseline.
        c.new = Some("value >= 0".into());
        fs::write(
            f.root().join("mutations.toml"),
            toml::to_string(&Catalogue {
                control: vec![c],
                ..Catalogue::default()
            })
            .unwrap(),
        )
        .unwrap();
        f.commit();
        let out = f.cli(&["run", "--all", "--broad", "--report", ".git/baseline.json"]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("red at baseline (ignored):"), "{text}");
        assert!(
            text.contains("tests::unrelated_test") && text.contains("prebroken::unrelated_test"),
            "{text}"
        );
    }
}

#[test]
fn cargo_baseline_is_shared_once_per_target_selection_and_times_each_phase() {
    let f = Fixture::new();
    replace_fixture_source(
        &f,
        "fn guard_rejects_zero() {",
        r#"fn guard_rejects_zero() {
        use std::io::Write;
        writeln!(std::fs::OpenOptions::new().create(true).append(true).open(".git/test-runs").unwrap(), "run").unwrap();"#,
    );
    let first = f.control();
    let mut second = first.clone();
    second.id = "positive".into();
    second.new = Some("value > 0 && value < 1".into());
    second.expect_red = vec!["tests::guard_accepts_positive".into()];
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![first, second],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    f.commit();
    let out = f.cli(&["run", "--all", "--report", ".git/shared.json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join(".git/shared.json")).unwrap()).unwrap();
    assert_eq!(
        fs::read_to_string(f.root().join(".git/test-runs")).unwrap(),
        "run\nrun\nrun\n"
    );
    assert!(rows[0]["baseline_build_ms"].as_u64().unwrap() > 0);
    assert!(rows[0]["baseline_test_ms"].as_u64().unwrap() > 0);
    assert_eq!(rows[1]["baseline_build_ms"], 0);
    assert_eq!(rows[1]["baseline_test_ms"], 0);
    eprintln!(
        "MEASURED SHARED FIXTURE BASELINE COST: build={} ms test={} ms (two rows, one clean run)",
        rows[0]["baseline_build_ms"], rows[0]["baseline_test_ms"]
    );
}

#[test]
fn equivalent_survivor_is_replayed_and_counted_separately() {
    let f = Fixture::new();
    let mut c = f.control();
    c.equivalent = Some("Reversing the operands preserves strict comparison".into());
    c.equivalent_guard = Some("guarded: value > 0 equals 0 < value".into());
    c.new = Some("0 < value".into());
    let row = f.run(&c);
    assert_eq!(row.outcome, Outcome::Equivalent, "{row:?}");
    assert!(row.passes());
    assert!(!row.outcome.is_caught());
    assert!(row.build_ms > 0 && row.test_ms > 0);
    assert!(row.green.contains(&"tests::guard_rejects_zero".into()));
    let mut without_expectations = c.clone();
    without_expectations.expect_red.clear();
    assert_eq!(f.run(&without_expectations).outcome, Outcome::Equivalent);
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![c],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    f.commit();
    assert!(f.cli(&["check"]).status.success());
    let out = f.cli(&["run", "--all"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout)
        .contains("0 CAUGHT, 0 CAUGHT_BROADLY (warning), 1 EQUIVALENT, 0 EQUIVALENT_CAUGHT"));
}

#[test]
fn caught_equivalent_is_equivalent_caught_and_fails() {
    let f = Fixture::new();
    let mut c = f.control();
    c.equivalent = Some("Incorrect claim that zero cannot reach guarded".into());
    c.equivalent_guard = Some("guarded: alleged positive-only caller".into());
    for expected in ["tests::guard_rejects_zero", "tests::vacuous_test"] {
        c.expect_red = vec![expected.into()];
        let row = f.run(&c);
        assert_eq!(row.outcome, Outcome::EquivalentCaught, "{row:?}");
        assert!(!row.passes());
        assert!(!row.outcome.is_caught());
    }
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![c],
            ..Catalogue::default()
        })
        .unwrap(),
    )
    .unwrap();
    f.commit();
    let out = f.cli(&["run", "--all"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("guard: EQUIVALENT_CAUGHT"));
}

fn output_control(f: &Fixture, probe: &str) -> Control {
    fs::create_dir_all(f.root().join("src/bin")).unwrap();
    fs::write(
        f.root().join("src/bin/output.rs"),
        format!(
            r#"fn main() {{
        let path = ".git/output-runs";
        let n = std::fs::read_to_string(path).unwrap_or_default().parse::<usize>().unwrap_or(0) + 1;
        std::fs::write(path, n.to_string()).unwrap();
        eprintln!("stderr changes on every run: {{n}}");
        {probe}
    }}"#
        ),
    )
    .unwrap();
    cmd(f.root(), "git", &["add", "src/bin"]);
    f.commit();
    let mut c = f.control();
    c.runner = "command".into();
    c.package = None;
    c.target = None;
    c.only = false;
    c.catch_on = Some("output_differs".into());
    c.command = Some(
        ["cargo", "run", "--quiet", "--locked", "--bin", "output"]
            .map(str::to_owned)
            .to_vec(),
    );
    c.expect_red.clear();
    c
}

#[test]
fn output_differs_catches_normalized_stdout_not_stderr() {
    let f = Fixture::new();
    let mut c = output_control(
        &f,
        "println!(\"guard={} timing={}\", mutation_fixture::guarded(0), n);",
    );
    c.output_normalize = vec![r" timing=\d+".into()];
    let row = f.run(&c);
    assert_eq!(row.outcome, Outcome::Caught, "{row:?}");
    assert!(row.passes());
    assert_eq!(
        fs::read_to_string(f.root().join(".git/output-runs")).unwrap(),
        "3"
    );
    assert!(row.baseline_test_ms > 0);
    assert!(row.test_ms > 0);
}

#[test]
fn identical_command_output_survives() {
    let f = Fixture::new();
    let c = output_control(&f, "println!(\"constant output\");");
    let row = f.run(&c);
    assert_eq!(row.outcome, Outcome::Survived, "{row:?}");
    assert!(!row.passes());
}

#[test]
fn output_equality_keeps_nonzero_exit_and_timeout_rules() {
    let f = Fixture::new();
    let c = output_control(
        &f,
        r#"println!("constant output");
        if mutation_fixture::guarded(0) { std::process::exit(23); }"#,
    );
    assert_eq!(f.run(&c).outcome, Outcome::Caught);
    let f = Fixture::new();
    let mut c = output_control(
        &f,
        r#"println!("constant output");
        if mutation_fixture::guarded(0) { std::thread::sleep(std::time::Duration::from_secs(10)); }"#,
    );
    // Compile before setting the short per-invocation deadline, so the timeout
    // proves a hung mutant, not an overloaded clean-tree compiler.
    cmd(f.root(), "cargo", &["build", "--locked", "--bin", "output"]);
    c.timeout_s = 2;
    let row = f.run(&c);
    assert_eq!(row.outcome, Outcome::Error, "{row:?}");
    assert_eq!(row.timed_out_phase, Some(Phase::Test));
}

#[test]
fn output_equality_prove_appends_and_replays_the_comparison() {
    let f = Fixture::new();
    output_control(&f, "println!(\"{}\", mutation_fixture::guarded(0));");
    let out = f.cli(&[
        "prove",
        "--id",
        "output-proof",
        "--guards",
        "stdout guards zero refusal",
        "--file",
        "src/lib.rs",
        "--old",
        "value > 0",
        "--new",
        "value >= 0",
        "--test-file",
        "src/bin/output.rs",
        "--catch-on",
        "output_differs",
        "--output-normalize",
        "unused-timing=[0-9]+",
        "--command",
        "cargo",
        "run",
        "--quiet",
        "--locked",
        "--bin",
        "output",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cat = load(&f.root().join("mutations.toml")).unwrap();
    let c = cat.control.iter().find(|c| c.id == "output-proof").unwrap();
    assert_eq!(c.catch_on.as_deref(), Some("output_differs"));
    assert!(c.expect_red.is_empty());
    assert_eq!(c.output_normalize, ["unused-timing=[0-9]+"]);
    assert!(f.cli(&["check"]).status.success());
    assert!(f.cli(&["run", "--only", "output-proof"]).status.success());
}

#[test]
fn nondeterministic_baseline_output_is_error_and_skips_mutant() {
    let f = Fixture::new();
    let c = output_control(
        &f,
        "println!(\"constant first line\\nchanging second line: {n}\");",
    );
    let row = f.run(&c);
    assert_eq!(row.outcome, Outcome::Error, "{row:?}");
    assert!(row
        .reason
        .as_deref()
        .unwrap()
        .contains("baseline output is not deterministic: line 2"));
    assert_eq!(
        fs::read_to_string(f.root().join(".git/output-runs")).unwrap(),
        "2"
    );
    assert_eq!(row.test_ms, 0);
}

#[test]
fn equivalent_and_output_equality_fields_validate_and_round_trip() {
    let f = Fixture::new();
    let mut c = f.control();
    c.equivalent = Some("The comparison is identical".into());
    let cat = |c: Control| Catalogue {
        control: vec![c],
        ..Catalogue::default()
    };
    assert!(validate(f.root(), &cat(c.clone()))
        .unwrap_err()
        .contains("required together"));
    c.equivalent_guard = Some("guarded: operand reversal".into());
    validate(f.root(), &cat(c.clone())).unwrap();
    for guard in [None, Some(" ".into())] {
        let mut invalid = c.clone();
        invalid.equivalent_guard = guard;
        assert!(validate(f.root(), &cat(invalid)).is_err());
    }
    c.equivalent = None;
    assert!(validate(f.root(), &cat(c)).is_err());
    let mut output = output_control(&f, "println!(\"stable\");");
    output.output_normalize = vec!["timing=[0-9]+".into()];
    validate(f.root(), &cat(output.clone())).unwrap();
    assert_eq!(
        toml::from_str::<Catalogue>(&toml::to_string(&cat(output.clone())).unwrap())
            .unwrap()
            .control[0],
        output
    );
    for (field, value) in [
        ("expect_message", "unexpected"),
        ("output_normalize", "["),
        ("catch_on", "exit_nonzero"),
    ] {
        let mut invalid = output.clone();
        match field {
            "expect_message" => invalid.expect_message = Some(value.into()),
            "output_normalize" => invalid.output_normalize = vec![value.into()],
            _ => invalid.catch_on = Some(value.into()),
        }
        assert!(validate(f.root(), &cat(invalid))
            .unwrap_err()
            .contains(field));
    }
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&cat(output.clone())).unwrap(),
    )
    .unwrap();
    f.commit();
    assert!(f.cli(&["check"]).status.success());
    output.expect_message = Some("not applicable".into());
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&cat(output)).unwrap(),
    )
    .unwrap();
    let out = f.cli(&["check"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("expect_message"));
}
