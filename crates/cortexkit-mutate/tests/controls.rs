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
#[test]
fn timeout_kills_process_group_and_restores() {
    let f = Fixture::new();
    let mut c = f.control();
    change(
        &mut c,
        "pub fn wait_hook() {}",
        "pub fn wait_hook() { std::fs::write(\"wait-pid\", std::process::id().to_string()).unwrap(); std::thread::sleep(std::time::Duration::from_secs(60)); }",
        "tests::waits",
    );
    // Precompile to keep the short timeout specific to the test process.
    cmd(f.root(), "cargo", &["test", "--no-run", "--locked"]);
    c.timeout_s = 2;
    let started = Instant::now();
    assert_eq!(f.run(&c).outcome, Outcome::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(15));
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
fn signal_restores(signal: i32) {
    use std::process::Stdio;
    let f = Fixture::new();
    let mut c = f.control();
    change(&mut c, "pub fn wait_hook() {}", "pub fn wait_hook() { std::fs::write(\"started\", \"ready\").unwrap(); std::thread::sleep(std::time::Duration::from_secs(60)); }", "tests::waits");
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue { control: vec![c] }).unwrap(),
    )
    .unwrap();
    let before = fs::read(f.root().join("src/lib.rs")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ck-mutate"))
        .current_dir(f.root())
        .args(["run", "--all"])
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
    unsafe {
        libc::kill(child.id() as i32, signal);
    }
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
    signal_restores(libc::SIGINT);
}
#[cfg(unix)]
#[test]
fn sigterm_restores_and_releases_lock() {
    signal_restores(libc::SIGTERM);
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
