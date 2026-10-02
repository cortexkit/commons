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
            package: "mutation-fixture".into(),
            target: "--lib".into(),
            expect_red: vec!["tests::guard_rejects_zero".into()],
            only: true,
            equivalent: None,
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
    assert!(r.reason.unwrap().contains("Cargo.lock changed"));
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
    assert!(validate(f.root(), &Catalogue { control: vec![c] }).is_err());
    let duplicate = Catalogue {
        control: vec![f.control(), f.control()],
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
            &Catalogue { control: vec![c] },
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
    assert!(String::from_utf8_lossy(&out.stdout).contains("removing its call site"));
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
        toml::to_string(&Catalogue { control: vec![c] }).unwrap(),
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
        &Catalogue { control: vec![c] },
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
    c.target = "--test empty".into();
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
    c.target = "--test guard".into();
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
        toml::to_string(&Catalogue { control: vec![c] }).unwrap(),
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
    assert_eq!(row.package, FIXTURE_PACKAGE);
    assert_eq!(row.target, "");
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

/// `explore` and `run` must not grow separate mutation paths. The library makes
/// this structural: `Saved` (target and Cargo.lock bytes), `execute` (the
/// build/test child) and `command` are private, and the only public functions
/// reaching them are `run_row` and `explore_row`, each a one-line call into the
/// private `replay`. This test pins that shape in the source, then checks both
/// entry points produce identical per-test results for one mutant.
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
        let end = src[start..].find("\n}\n").expect("function end") + start;
        &src[start..end]
    }
    let replay = body(lib, "\nfn replay(");
    for needle in [
        "Saved::new(",
        "saved.restore()",
        "replaced(root, &edits)",
        "command(c, \"build\", scope)",
        "command(c, \"run\", scope)",
        "parse_tests(&tests.text",
    ] {
        assert_eq!(lib.matches(needle).count(), 1, "{needle} must occur once");
        assert!(replay.contains(needle), "{needle} must live in replay");
    }
    for entry in ["pub fn run_row(", "pub fn explore_row("] {
        let entry_body = body(lib, entry);
        assert!(
            entry_body.contains("replay(root, c, allow_dirty, stop, "),
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
    c.target = String::new();
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
