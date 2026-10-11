#![forbid(unsafe_code)]

mod support;

use cortexkit_mutate::*;
use std::{fs, path::Path, process::Command, sync::atomic::AtomicBool};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        support::isolate_fixture_environment();
        let f = Self(tempfile::tempdir().unwrap());
        for (path, source) in [
            ("Cargo.toml", include_str!("fixture/ignored/Cargo.toml")),
            ("src/lib.rs", include_str!("fixture/ignored/src/lib.rs")),
            (
                "tests/contract.rs",
                include_str!("fixture/ignored/tests/contract.rs"),
            ),
        ] {
            let path = f.root().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        f.command("cargo", &["generate-lockfile", "--offline"]);
        f.command("git", &["init", "-q"]);
        f.command("git", &["config", "user.email", "fixture@example.invalid"]);
        f.command("git", &["config", "user.name", "Ignored fixture"]);
        f.catalogue(vec![f.row("cargo")]);
        f
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn command(&self, program: &str, args: &[&str]) {
        let out = Command::new(program)
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{program} {args:?}: {out:?}");
    }

    /// A guard that only an ignored test exercises, like a real-daemon test
    /// kept out of the default run.
    fn row(&self, runner: &str) -> Control {
        let mut row: Control = toml::from_str(
            r#"
id = "ignored-guard"
guards = "zero is rejected, observed only by an ignored test"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "tests/contract.rs"
runner = "cargo"
package = "ignored-fixture"
target = "--test contract"
ignored = "include"
expect_red = ["ignored_guard"]
timeout_s = 60
"#,
        )
        .unwrap();
        row.runner = runner.into();
        row
    }

    /// A guard that an ordinary test exercises, in the same target.
    fn ordinary_row(&self, runner: &str) -> Control {
        let mut row = self.row(runner);
        row.id = "ordinary-guard".into();
        row.old = Some("value < 10".into());
        row.new = Some("value <= 10".into());
        row.ignored = None;
        row.expect_red = vec!["ordinary_guard".into()];
        row
    }

    fn catalogue(&self, rows: Vec<Control>) {
        fs::write(
            self.root().join("mutations.toml"),
            toml::to_string(&Catalogue {
                control: rows,
                ..Catalogue::default()
            })
            .unwrap(),
        )
        .unwrap();
        self.command(
            "git",
            &[
                "add",
                "Cargo.toml",
                "Cargo.lock",
                "src",
                "tests",
                "mutations.toml",
            ],
        );
        self.command("git", &["commit", "--allow-empty", "-qm", "fixture"]);
        let _ = fs::remove_file(self.root().join(".git/guard-runs"));
    }

    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_ckdev-mutate"))
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap()
    }

    fn run(&self, broad: bool) -> (std::process::Output, serde_json::Value) {
        let source = fs::read(self.root().join("src/lib.rs")).unwrap();
        let mut args = vec!["run", "--all", "--report", ".git/report.json"];
        if broad {
            args.push("--broad");
        }
        let out = self.cli(&args);
        let report =
            serde_json::from_slice(&fs::read(self.root().join(".git/report.json")).unwrap())
                .unwrap();
        assert_eq!(source, fs::read(self.root().join("src/lib.rs")).unwrap());
        (out, report)
    }

    /// How many times each recording test ran, independent of run order.
    fn runs(&self, name: &str) -> usize {
        fs::read_to_string(self.root().join(".git/guard-runs"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.trim() == name)
            .count()
    }
}

/// Both runners when nextest is installed; CI requires it.
fn runners() -> Vec<&'static str> {
    if Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|out| out.status.success())
    {
        return vec!["cargo", "nextest"];
    }
    assert!(
        std::env::var("CK_MUTATE_REQUIRE_NEXTEST").as_deref() != Ok("1"),
        "CI must install nextest"
    );
    eprintln!("SKIP nextest ignored-test replay: nextest is not installed");
    vec!["cargo"]
}

fn baseline_red_names(row: &serde_json::Value) -> Vec<String> {
    row["baseline_red"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|names| names.as_array().unwrap())
        .map(|name| name.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn ignored_guard_is_caught_only_when_the_row_selects_ignored_tests() {
    let f = Fixture::new();
    for runner in runners() {
        for selection in [IgnoredSelection::Include, IgnoredSelection::Only] {
            let mut row = f.row(runner);
            row.ignored = Some(selection);
            f.catalogue(vec![row]);
            let checked = f.cli(&["check"]);
            assert!(
                checked.status.success(),
                "{runner} {selection:?}: {checked:?}"
            );
            for broad in [false, true] {
                let (out, rows) = f.run(broad);
                assert!(
                    out.status.success(),
                    "{runner} {selection:?}: {out:?}; {rows}"
                );
                // No other target fails, so a broad replay is a plain catch
                // that records it observed the whole package.
                assert_eq!(
                    rows[0]["outcome"], "CAUGHT",
                    "{runner} {selection:?}: {rows}"
                );
                assert_eq!(rows[0]["breadth_observed"], broad, "{rows}");
                assert_eq!(rows[0]["red"], serde_json::json!(["ignored_guard"]));
                // The always-failing ignored test is red at baseline, so the
                // baseline itself ran the ignored selection.
                assert_eq!(baseline_red_names(&rows[0]), ["ignored_baseline"], "{rows}");
            }
            // Ordinary tests run with "include" and are skipped with "only".
            let ordinary = f.runs("ordinary");
            match selection {
                IgnoredSelection::Include => assert!(ordinary > 0, "{runner}"),
                IgnoredSelection::Only => assert_eq!(ordinary, 0, "{runner}"),
            }
        }
        // Calling `run_row` directly (no CLI session, so no shared baseline)
        // must apply the selection to its own listing, baseline and run.
        let row = run_row(
            f.root(),
            &f.row(runner),
            false,
            &AtomicBool::new(false),
            false,
        )
        .unwrap();
        assert_eq!(row.outcome, Outcome::Caught, "{runner}: {row:?}");
    }
}

#[test]
fn check_and_run_refuse_an_ignored_expected_test_without_the_field() {
    let f = Fixture::new();
    for runner in runners() {
        let mut row = f.row(runner);
        row.ignored = None;
        f.catalogue(vec![row.clone()]);
        let checked = f.cli(&["check"]);
        assert!(!checked.status.success(), "{runner}: {checked:?}");
        let stderr = String::from_utf8_lossy(&checked.stderr);
        assert!(
            stderr.contains("expect_red test ignored_guard is #[ignore]d")
                && stderr.contains("set ignored = \"include\""),
            "{runner}: {stderr}"
        );
        let (out, rows) = f.run(false);
        assert!(!out.status.success(), "{runner}: {rows}");
        assert_eq!(rows[0]["outcome"], "ERROR", "{runner}: {rows}");
        assert!(
            rows[0]["reason"]
                .as_str()
                .unwrap()
                .contains("set ignored = \"include\""),
            "{runner}: {rows}"
        );
        // A qualified expectation is refused the same way.
        row.expect_red = vec![if runner == "cargo" {
            "contract::ignored_guard".into()
        } else {
            "ignored-fixture::contract::ignored_guard".into()
        }];
        let error = check(
            f.root(),
            &Catalogue {
                control: vec![row],
                ..Catalogue::default()
            },
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(
            error.contains("set ignored = \"include\""),
            "{runner}: {error}"
        );
    }
}

#[test]
fn baselines_are_not_shared_across_ignored_selections() {
    let f = Fixture::new();
    for runner in runners() {
        let mut ignored = f.row(runner);
        ignored.ignored = Some(IgnoredSelection::Only);
        // Same runner, package, target and features: only the selection differs.
        f.catalogue(vec![f.ordinary_row(runner), ignored]);
        let (out, rows) = f.run(false);
        assert!(out.status.success(), "{runner}: {out:?}; {rows}");
        let by_id = |id: &str| {
            rows.as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == id)
                .unwrap()
                .clone()
        };
        let (ordinary, ignored) = (by_id("ordinary-guard"), by_id("ignored-guard"));
        assert_eq!(ordinary["outcome"], "CAUGHT", "{runner}: {rows}");
        assert_eq!(ignored["outcome"], "CAUGHT", "{runner}: {rows}");
        assert!(baseline_red_names(&ordinary).is_empty(), "{runner}: {rows}");
        assert_eq!(
            baseline_red_names(&ignored),
            ["ignored_baseline"],
            "{runner}: {rows}"
        );
        for row in rows.as_array().unwrap() {
            assert!(row["baseline_test_ms"].as_u64().unwrap() > 0, "{row}");
        }
        // One baseline and one mutant run for each row.
        assert_eq!(f.runs("ordinary"), 2, "{runner}");
        assert_eq!(f.runs("ignored"), 2, "{runner}");
    }
}

#[test]
fn ignored_field_validates_and_round_trips_without_argument_injection() {
    let f = Fixture::new();
    for runner in ["cargo", "nextest"] {
        for selection in [IgnoredSelection::Include, IgnoredSelection::Only] {
            let mut row = f.row(runner);
            row.ignored = Some(selection);
            let cat = Catalogue {
                control: vec![row],
                ..Catalogue::default()
            };
            validate(f.root(), &cat).unwrap();
            let text = toml::to_string(&cat).unwrap();
            assert!(
                text.contains(&format!("ignored = \"{}\"", selection.as_str())),
                "{text}"
            );
            let decoded: Catalogue = toml::from_str(&text).unwrap();
            assert_eq!(decoded.control, cat.control);
        }
    }
    let base = toml::to_string(&Catalogue {
        control: vec![f.row("cargo")],
        ..Catalogue::default()
    })
    .unwrap();
    for value in ["all", "--include-ignored", "include --nocapture", ""] {
        let text = base.replace(
            "ignored = \"include\"",
            &format!("ignored = {}", toml::Value::String(value.into())),
        );
        assert_ne!(text, base);
        let error = toml::from_str::<Catalogue>(&text)
            .err()
            .expect("an unknown ignored selection must not parse")
            .to_string();
        assert!(
            error.contains("include") && error.contains("only"),
            "{error}"
        );
    }
    let row: Control = toml::from_str(
        r#"
id = "command-ignored"
guards = "command rows own their argv"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "tests/contract.rs"
runner = "command"
command = ["runner", "{test}"]
test_count_pattern = "Ran {count} tests"
expect_red = ["guard"]
ignored = "include"
"#,
    )
    .unwrap();
    let error = check(
        f.root(),
        &Catalogue {
            control: vec![row],
            ..Catalogue::default()
        },
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(
        error.contains("ignored must be absent for runner = command"),
        "{error}"
    );
}

#[test]
fn explore_and_prove_preserve_the_ignored_selection_when_appending() {
    for action in ["explore", "prove"] {
        let f = Fixture::new();
        let mut args = vec![
            action,
            "--package",
            "ignored-fixture",
            "--ignored",
            "only",
            "--file",
            "src/lib.rs",
            "--old",
            "value > 0",
            "--new",
            "value >= 0",
            "--id",
            "appended",
            "--guards",
            "zero must be rejected",
            "--test-file",
            "tests/contract.rs",
        ];
        if action == "explore" {
            args.push("--append");
        } else {
            args.extend([
                "--target",
                "--test contract",
                "--expect-red",
                "ignored_guard",
            ]);
        }
        let out = f.cli(&args);
        assert!(out.status.success(), "{action}: {out:?}");
        let cat = load(&f.root().join("mutations.toml")).unwrap();
        let row = cat.control.iter().find(|row| row.id == "appended").unwrap();
        assert_eq!(row.ignored, Some(IgnoredSelection::Only), "{action}");
        assert_eq!(row.expect_red, ["ignored_guard"], "{action}");
        let checked = f.cli(&["check"]);
        assert!(checked.status.success(), "{action}: {checked:?}");
    }
}
