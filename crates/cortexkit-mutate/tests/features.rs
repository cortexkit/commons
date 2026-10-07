#![forbid(unsafe_code)]

use cortexkit_mutate::*;
use std::{fs, path::Path, process::Command, sync::atomic::AtomicBool};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        let f = Self(tempfile::tempdir().unwrap());
        for (path, source) in [
            ("Cargo.toml", include_str!("fixture/features/Cargo.toml")),
            ("src/lib.rs", include_str!("fixture/features/src/lib.rs")),
            (
                "tests/contract.rs",
                include_str!("fixture/features/tests/contract.rs"),
            ),
            (
                "tests/collateral.rs",
                include_str!("fixture/features/tests/collateral.rs"),
            ),
        ] {
            let path = f.root().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        f.command("cargo", &["generate-lockfile", "--offline"]);
        f.command("git", &["init", "-q"]);
        f.command("git", &["config", "user.email", "fixture@example.invalid"]);
        f.command("git", &["config", "user.name", "Feature fixture"]);
        f.catalogue(vec![f.row()]);
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

    fn row(&self) -> Control {
        toml::from_str(
            r#"
id = "feature-guard"
guards = "zero is rejected through the non-default test seam"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "tests/contract.rs"
runner = "cargo"
package = "feature-fixture"
target = "--test contract"
features = ["test-support"]
expect_red = ["feature_gated_guard"]
timeout_s = 30
"#,
        )
        .unwrap()
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
        let lock = fs::read(self.root().join("Cargo.lock")).unwrap();
        let mut args = vec!["run", "--all", "--report", ".git/report.json"];
        if broad {
            args.push("--broad");
        }
        let out = self.cli(&args);
        let report =
            serde_json::from_slice(&fs::read(self.root().join(".git/report.json")).unwrap())
                .unwrap();
        assert_eq!(source, fs::read(self.root().join("src/lib.rs")).unwrap());
        assert_eq!(lock, fs::read(self.root().join("Cargo.lock")).unwrap());
        (out, report)
    }
}

#[test]
fn feature_gated_row_requires_features_and_is_caught() {
    let f = Fixture::new();
    let mut missing = f.row();
    missing.features = None;
    f.catalogue(vec![missing]);
    let (out, rows) = f.run(false);
    assert!(!out.status.success());
    assert_eq!(rows[0]["outcome"], "ERROR");
    assert!(
        rows[0]["reason"].as_str().unwrap().contains("E0432"),
        "{rows}"
    );

    f.catalogue(vec![f.row()]);
    let (out, rows) = f.run(false);
    assert!(out.status.success(), "{out:?}; {rows}");
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(rows[0]["red"], serde_json::json!(["feature_gated_guard"]));
    let checked = f.cli(&["check"]);
    assert!(checked.status.success(), "{checked:?}");
}

#[test]
fn same_target_different_features_do_not_share_baselines() {
    let f = Fixture::new();
    let first = f.row();
    let mut second = first.clone();
    second.id = "strict-feature-guard".into();
    second
        .features
        .as_mut()
        .unwrap()
        .push("strict-baseline".into());
    f.catalogue(vec![first, second]);
    let (out, rows) = f.run(false);
    assert!(out.status.success(), "{out:?}; {rows}");
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    assert_eq!(rows[1]["outcome"], "CAUGHT");
    assert_eq!(rows[0]["baseline_red"], serde_json::json!({}));
    assert_eq!(
        rows[1]["baseline_red"],
        serde_json::json!({"contract": ["feature_baseline"]})
    );
    assert_eq!(rows[1]["baseline_failures"].as_object().unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(f.root().join(".git/guard-runs"))
            .unwrap()
            .replace("\r\n", "\n"),
        "run\nrun\nrun\nrun\n"
    );
    for row in rows.as_array().unwrap() {
        assert!(row["baseline_build_ms"].as_u64().unwrap() > 0, "{row}");
        assert!(row["baseline_test_ms"].as_u64().unwrap() > 0, "{row}");
    }
}

#[test]
fn broad_runs_honour_features() {
    let f = Fixture::new();
    let (out, rows) = f.run(true);
    assert!(out.status.success(), "{out:?}; {rows}");
    assert_eq!(rows[0]["outcome"], "CAUGHT_BROADLY");
    assert_eq!(rows[0]["breadth_observed"], true);
    assert_eq!(
        rows[0]["collateral"]["targets"],
        serde_json::json!(["collateral"])
    );
}

#[test]
fn listing_and_name_resolution_preserve_the_target_selector() {
    let f = Fixture::new();
    fs::write(
        f.root().join("tests/unselected.rs"),
        "compile_error!(\"unselected target must not compile\");\n",
    )
    .unwrap();
    f.catalogue(vec![f.row()]);
    let checked = f.cli(&["check"]);
    assert!(checked.status.success(), "{checked:?}");
    let (out, rows) = f.run(false);
    assert!(out.status.success(), "{out:?}; {rows}");
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    // Calling `run_row` directly (no CLI session, so no shared baseline) must
    // also resolve test names within the row's own `--test` target and features.
    let row = run_row(f.root(), &f.row(), false, &AtomicBool::new(false), false).unwrap();
    assert_eq!(row.outcome, Outcome::Caught, "{row:?}");
}

#[test]
fn feature_fields_validate_and_round_trip_without_argument_injection() {
    let f = Fixture::new();
    for name in ["", "a b", "a\tb", "a\nb", "--all-features", "a,b", "a\0b"] {
        let mut row = f.row();
        row.features = Some(vec![name.into()]);
        let cat = Catalogue {
            control: vec![row],
            ..Catalogue::default()
        };
        let error = check(f.root(), &cat, &AtomicBool::new(false)).unwrap_err();
        assert!(
            error.contains("feature-guard: invalid features name"),
            "{error}"
        );
    }
    for runner in ["cargo", "nextest"] {
        let mut row = f.row();
        row.runner = runner.into();
        row.no_default_features = Some(true);
        let cat = Catalogue {
            control: vec![row.clone()],
            ..Catalogue::default()
        };
        validate(f.root(), &cat).unwrap();
        let decoded: Catalogue = toml::from_str(&toml::to_string(&cat).unwrap()).unwrap();
        assert_eq!(decoded.control, cat.control);
        row.all_features = Some(true);
        let error = check(
            f.root(),
            &Catalogue {
                control: vec![row.clone()],
                ..Catalogue::default()
            },
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(
            error.contains("all_features is mutually exclusive"),
            "{error}"
        );
        row.features = None;
        let error = check(
            f.root(),
            &Catalogue {
                control: vec![row.clone()],
                ..Catalogue::default()
            },
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.contains("no_default_features"), "{error}");
        row.no_default_features = None;
        validate(
            f.root(),
            &Catalogue {
                control: vec![row],
                ..Catalogue::default()
            },
        )
        .unwrap();
    }
    for field in [
        "features = []",
        "no_default_features = false",
        "all_features = false",
    ] {
        let row: Control = toml::from_str(&format!(
            r#"
id = "command-feature"
guards = "command rows own their argv"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "tests/contract.rs"
runner = "command"
command = ["runner", "{{test}}"]
test_count_pattern = "Ran {{count}} tests"
expect_red = ["guard"]
{field}
"#
        ))
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
        let name = field.split_whitespace().next().unwrap();
        assert!(
            error.contains(&format!("{name} must be absent for runner = command")),
            "{error}"
        );
    }
}

#[test]
fn explore_and_prove_preserve_feature_options_when_appending() {
    for action in ["explore", "prove"] {
        let f = Fixture::new();
        let mut args = vec![
            action,
            "--package",
            "feature-fixture",
            "--features",
            "test-support",
            "--no-default-features",
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
                "feature_gated_guard",
            ]);
        }
        let out = f.cli(&args);
        assert!(out.status.success(), "{action}: {out:?}");
        let cat = load(&f.root().join("mutations.toml")).unwrap();
        let row = cat.control.iter().find(|row| row.id == "appended").unwrap();
        assert_eq!(row.features, Some(vec!["test-support".into()]));
        assert_eq!(row.no_default_features, Some(true));
        let checked = f.cli(&["check"]);
        assert!(checked.status.success(), "{checked:?}");
    }
}

#[test]
fn all_features_and_nextest_feature_rows_are_caught() {
    let f = Fixture::new();
    let mut row = f.row();
    row.features = None;
    row.all_features = Some(true);
    f.catalogue(vec![row]);
    let (out, rows) = f.run(false);
    assert!(out.status.success(), "{out:?}; {rows}");
    assert_eq!(rows[0]["outcome"], "CAUGHT");
    if !Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|out| out.status.success())
    {
        assert!(
            std::env::var("CK_MUTATE_REQUIRE_NEXTEST").as_deref() != Ok("1"),
            "CI must install nextest"
        );
        eprintln!("SKIP nextest feature replay: nextest is not installed");
        return;
    }
    let mut row = f.row();
    row.runner = "nextest".into();
    f.catalogue(vec![row]);
    let checked = f.cli(&["check"]);
    assert!(checked.status.success(), "{checked:?}");
    for broad in [false, true] {
        let (out, rows) = f.run(broad);
        assert!(out.status.success(), "{out:?}; {rows}");
        assert_eq!(
            rows[0]["outcome"],
            if broad { "CAUGHT_BROADLY" } else { "CAUGHT" }
        );
    }
}
