#![forbid(unsafe_code)]

use cortexkit_mutate::*;
use std::{fs, path::Path, process::Command, sync::atomic::AtomicBool};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        let fixture = Self(tempfile::tempdir().unwrap());
        fs::create_dir_all(fixture.root().join("src/bin")).unwrap();
        fs::write(fixture.root().join("Cargo.toml"), "[workspace]\n[package]\nname = 'session-fixture'\nversion = '0.1.0'\nedition = '2021'\n").unwrap();
        fs::write(fixture.root().join("src/lib.rs"), r#"
pub fn guarded(value: i32) -> bool { value > 0 }
#[cfg(test)] mod tests {
    #[test] fn fixture_guard() {
        assert_eq!(std::fs::read_to_string("fixture-output").expect("prebuild must produce the fixture"), "false");
    }
}
"#).unwrap();
        fs::write(fixture.root().join("src/bin/prerequisite.rs"), r#"
fn main() {
    let value = session_fixture::guarded(0).to_string();
    std::fs::write("fixture-output", &value).unwrap();
    use std::io::Write;
    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(".git/prebuild-runs").unwrap();
    writeln!(log, "{value}").unwrap();
    println!("fixture binary refreshed: {value}");
    if value == "true" && std::path::Path::new("create-fail-restore").exists() {
        std::fs::write("fail-baseline", "").unwrap();
    }
    if std::path::Path::new("fail-baseline").exists() || (value == "true" && std::path::Path::new("fail-mutant").exists()) {
        std::process::exit(23);
    }
}
"#).unwrap();
        fixture.cmd("cargo", &["generate-lockfile", "--offline"]);
        fixture.cmd("git", &["init", "-q"]);
        fixture.cmd("git", &["config", "user.email", "fixture@example.invalid"]);
        fixture.cmd("git", &["config", "user.name", "Session fixture"]);
        fixture.catalogue(fixture.row());
        fixture.commit();
        fixture
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn row(&self) -> Control {
        toml::from_str(
            r#"
id = "fixture-guard"
guards = "the fixture binary rejects zero"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "src/lib.rs"
runner = "cargo"
package = "session-fixture"
target = "--lib"
expect_red = ["tests::fixture_guard"]
only = true
"#,
        )
        .unwrap()
    }

    fn prebuild(&self) -> Vec<Prebuild> {
        vec![Prebuild {
            name: "fixture-binary".into(),
            command: ["cargo", "run", "--bin", "prerequisite", "--locked"]
                .map(str::to_owned)
                .to_vec(),
            timeout_s: 60,
        }]
    }

    fn catalogue(&self, row: Control) {
        fs::write(
            self.root().join("mutations.toml"),
            toml::to_string(&Catalogue {
                control: vec![row],
                prebuild: self.prebuild(),
            })
            .unwrap(),
        )
        .unwrap();
    }

    fn cmd(&self, program: &str, args: &[&str]) {
        let output = Command::new(program)
            .args(args)
            .current_dir(self.root())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program} {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit(&self) {
        self.cmd(
            "git",
            &["add", "Cargo.toml", "Cargo.lock", "src", "mutations.toml"],
        );
        self.cmd("git", &["commit", "-qm", "fixture"]);
    }

    fn cli(&self, args: &[&str]) -> std::process::Output {
        let before = fs::read(self.root().join("src/lib.rs")).unwrap();
        let lock = fs::read(self.root().join("Cargo.lock")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ck-mutate"))
            .args(args)
            .current_dir(self.root())
            .output()
            .unwrap();
        assert_eq!(before, fs::read(self.root().join("src/lib.rs")).unwrap());
        assert_eq!(lock, fs::read(self.root().join("Cargo.lock")).unwrap());
        output
    }

    fn report(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.root().join(".git/report.json")).unwrap()).unwrap()
    }
}

fn other_os() -> &'static str {
    if std::env::consts::OS == "macos" {
        "linux"
    } else {
        "macos"
    }
}

#[test]
fn platform_gate_skips_without_running_or_counting_a_row() {
    let f = Fixture::new();
    let mut c = f.row();
    c.platforms = Some(vec![other_os().into()]);
    // This expected test does not exist: check must not try listing it here.
    c.expect_red = vec!["macos_only_test".into()];
    c.hub = Some("desktop tests share one platform guard".into());
    c.hub_targets = Some(vec!["desktop".into()]);
    f.catalogue(c.clone());
    f.commit();
    assert!(f.cli(&["check"]).status.success());
    let output = f.cli(&["run", "--all", "--broad", "--report", ".git/report.json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = f.report();
    assert_eq!(report[0]["outcome"], "SKIPPED_PLATFORM");
    assert_eq!(report[0]["build_ms"], 0);
    assert_eq!(report[0]["test_ms"], 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 SKIPPED_PLATFORM"), "{stdout}");
    assert!(
        stdout.contains("0 CAUGHT") && stdout.contains("0 HUB"),
        "{stdout}"
    );
    assert!(!f.root().join(".git/prebuild-runs").exists());
    assert!(!f.root().join("target").exists());
    for disposition in [None, Some("only test callers exist".to_owned())] {
        c.hub = None;
        c.hub_targets = None;
        c.unreachable = disposition;
        let report = explore_row(f.root(), &c, false, &AtomicBool::new(false), false).unwrap();
        assert_eq!(report.outcome, Outcome::SkippedPlatform);
        assert!(report.passes() && !report.outcome.is_caught());
    }
}

#[test]
fn prebuild_refreshes_mutated_fixture_and_records_separate_timing() {
    for mode in ["run", "broad", "explore", "prove"] {
        let f = Fixture::new();
        let args = match mode {
            "run" => vec!["run", "--all", "--report", ".git/report.json"],
            "broad" => vec!["run", "--all", "--broad", "--report", ".git/report.json"],
            "explore" => vec![
                "explore",
                "--package",
                "session-fixture",
                "--file",
                "src/lib.rs",
                "--old",
                "value > 0",
                "--new",
                "value >= 0",
                "--append",
                "--id",
                "discovered",
                "--guards",
                "fixture binary rejects zero",
                "--test-file",
                "src/lib.rs",
                "--report",
                ".git/report.json",
            ],
            _ => vec![
                "prove",
                "--id",
                "proved",
                "--guards",
                "fixture binary rejects zero",
                "--package",
                "session-fixture",
                "--file",
                "src/lib.rs",
                "--old",
                "value > 0",
                "--new",
                "value >= 0",
                "--test-file",
                "src/lib.rs",
                "--target=--lib",
                "--expect-red",
                "tests::fixture_guard",
                "--report",
                ".git/report.json",
            ],
        };
        let output = f.cli(&args);
        assert!(
            output.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let report = f.report();
        assert_eq!(report[0]["outcome"], "CAUGHT", "{mode}: {report}");
        assert!(report[0]["prebuild_ms"].as_u64().unwrap() > 0);
        assert!(report[0]["baseline_prebuild_ms"].as_u64().unwrap() > 0);
        assert!(report[0]["prebuild_tail"]
            .as_str()
            .unwrap()
            .contains("fixture binary refreshed: true"));
        assert!(report[0]["baseline_prebuild_tail"]
            .as_str()
            .unwrap()
            .contains("fixture binary refreshed: false"));
        assert_eq!(
            fs::read_to_string(f.root().join(".git/prebuild-runs")).unwrap(),
            "false\ntrue\n",
            "baseline once, then mutated fixture rebuilt, including append's check"
        );
        assert_eq!(
            fs::read_to_string(f.root().join("fixture-output")).unwrap(),
            "true"
        );
    }
}

#[test]
fn failing_unmutated_prebuild_aborts_replay_by_step_name() {
    let f = Fixture::new();
    fs::write(f.root().join("fail-baseline"), "").unwrap();
    let output = f.cli(&["run", "--all", "--report", ".git/report.json"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unmutated prebuild \"fixture-binary\": failed"),
        "{stderr}"
    );
    assert!(stderr.contains("before grading rows"), "{stderr}");
    assert!(!f.root().join(".git/report.json").exists());
    assert_eq!(
        fs::read_to_string(f.root().join(".git/prebuild-runs")).unwrap(),
        "false\n"
    );
}

#[test]
fn restored_prebuild_prevents_stale_mutant_fixtures_in_the_next_row() {
    let f = Fixture::new();
    let path = f.root().join("src/lib.rs");
    let source = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!(
            "{source}\n\
        pub fn unrelated() -> bool {{ true }}\n\
        #[cfg(test)] mod freshness {{\n\
        #[test] fn unrelated_is_guarded() {{ assert!(super::unrelated()); }}\n\
        }}\n"
        ),
    )
    .unwrap();
    let mut first = f.row();
    first.id = "a-mutates-fixture".into();
    let mut second = f.row();
    second.id = "b-needs-clean-fixture".into();
    second.old = Some("unrelated() -> bool { true }".into());
    second.new = Some("unrelated() -> bool { false }".into());
    second.expect_red = vec!["freshness::unrelated_is_guarded".into()];
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![first, second],
            prebuild: f.prebuild(),
        })
        .unwrap(),
    )
    .unwrap();
    // The next mutant's build consumes the fixture before its mutated prebuild
    // gets a chance to refresh it. Source restoration cannot refresh this file.
    fs::write(f.root().join("build.rs"), r#"
fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    let source = std::fs::read_to_string("src/lib.rs").unwrap();
    if source.contains("unrelated() -> bool { false }") {
        assert_eq!(std::fs::read_to_string("fixture-output").unwrap(), "false", "build must not consume a previous mutant's fixture");
    }
}
"#).unwrap();
    f.cmd("git", &["add", "build.rs"]);
    f.commit();
    let output = f.cli(&["run", "--all", "--report", ".git/report.json"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = f.report();
    assert_eq!(report[0]["outcome"], "CAUGHT");
    assert_eq!(
        report[1]["outcome"], "CAUGHT",
        "the next build must not consume the previous row's mutant fixture"
    );
    assert_eq!(report[0]["restore_prebuild_ms"], 0);
    assert!(report[1]["restore_prebuild_ms"].as_u64().unwrap() > 0);
    assert_eq!(report[1]["baseline_prebuild_ms"], 0);
    assert_eq!(
        fs::read_to_string(f.root().join(".git/prebuild-runs")).unwrap(),
        "false\ntrue\nfalse\nfalse\n"
    );
}

#[test]
fn failing_mutated_prebuild_is_error_not_a_catch() {
    let f = Fixture::new();
    fs::write(f.root().join("fail-mutant"), "").unwrap();
    let output = f.cli(&["run", "--all", "--report", ".git/report.json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = f.report();
    assert_eq!(report[0]["outcome"], "ERROR");
    assert!(report[0]["reason"]
        .as_str()
        .unwrap()
        .contains("prebuild \"fixture-binary\": failed"));
    assert_eq!(report[0]["test_ms"], 0);
    assert_eq!(report[0]["red"], serde_json::json!([]));
    assert!(report[0]["prebuild_ms"].as_u64().unwrap() > 0);
}

#[test]
fn declared_prebuild_respects_a_listed_lockfile_edit_and_restores_it() {
    let f = Fixture::new();
    let lock = fs::read_to_string(f.root().join("Cargo.lock")).unwrap();
    let mut c = f.row();
    c.edits = c.edits().unwrap();
    c.edits.push(Edit {
        file: "Cargo.lock".into(),
        old: lock.clone(),
        new: format!("{lock}\n# intentional mutation fixture\n"),
    });
    c.file = None;
    c.old = None;
    c.new = None;
    f.catalogue(c);
    f.commit();
    let output = f.cli(&["run", "--all", "--report", ".git/report.json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(f.report()[0]["outcome"], "CAUGHT");
    assert_eq!(
        fs::read_to_string(f.root().join("Cargo.lock")).unwrap(),
        lock
    );
}

#[test]
fn failing_restored_prebuild_aborts_later_rows_by_name() {
    let f = Fixture::new();
    fs::write(f.root().join("create-fail-restore"), "").unwrap();
    let mut second = f.row();
    second.id = "later-row".into();
    fs::write(
        f.root().join("mutations.toml"),
        toml::to_string(&Catalogue {
            control: vec![f.row(), second],
            prebuild: f.prebuild(),
        })
        .unwrap(),
    )
    .unwrap();
    f.commit();
    let output = f.cli(&["run", "--all", "--report", ".git/report.json"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("restore prebuild before later-row")
            && stderr.contains("prebuild \"fixture-binary\": failed"),
        "{stderr}"
    );
    assert!(!f.root().join(".git/report.json").exists());
    assert_eq!(
        fs::read_to_string(f.root().join(".git/prebuild-runs")).unwrap(),
        "false\ntrue\nfalse\n"
    );
}

#[test]
fn desk_only_is_reasoned_separate_and_never_executed() {
    let f = Fixture::new();
    let mut c = f.row();
    c.desk_only = Some("TCC requires a real Mac with physical input".into());
    c.expect_red.clear();
    for os in [std::env::consts::OS, other_os()] {
        c.platforms = Some(vec![os.into()]);
        f.catalogue(c.clone());
        f.commit();
        assert!(f.cli(&["check"]).status.success());
        let output = f.cli(&["run", "--all", "--broad", "--report", ".git/report.json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(f.report()[0]["outcome"], "DESK_ONLY");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("1 DESK_ONLY")
                && stdout.contains("0 CAUGHT")
                && stdout.contains("0 SKIPPED_PLATFORM"),
            "{stdout}"
        );
    }
    assert!(!f.root().join(".git/prebuild-runs").exists());
    assert!(!f.root().join("target").exists());
    assert_eq!(
        explore_row(f.root(), &c, false, &AtomicBool::new(false), true)
            .unwrap()
            .outcome,
        Outcome::DeskOnly
    );
}

#[test]
fn new_catalogue_fields_validate_and_roundtrip() {
    let f = Fixture::new();
    let c = f.row();
    let valid = Catalogue {
        control: vec![c.clone()],
        prebuild: f.prebuild(),
    };
    validate(f.root(), &valid).unwrap();
    let path = f.root().join("mutations.toml");
    assert_eq!(load(&path).unwrap().prebuild, valid.prebuild);
    for os in ["MacOS", "darwin", "definitely-not-an-os"] {
        let mut invalid = c.clone();
        invalid.platforms = Some(vec![os.into()]);
        f.catalogue(invalid);
        let error = load(&path).err().unwrap();
        assert!(
            error.contains("fixture-guard") && error.contains("unknown target_os"),
            "{error}"
        );
    }
    let mut invalid = c.clone();
    invalid.platforms = Some(vec![]);
    f.catalogue(invalid);
    assert!(load(&path)
        .err()
        .unwrap()
        .contains("platforms must be nonempty"));
    let mut invalid = c.clone();
    invalid.desk_only = Some(" \n".into());
    f.catalogue(invalid);
    assert!(load(&path).err().unwrap().contains("non-empty reason"));
    for other in ["equivalent", "unreachable", "hub"] {
        let mut invalid = c.clone();
        invalid.desk_only = Some("requires physical input".into());
        match other {
            "equivalent" => invalid.equivalent = Some("same result".into()),
            "unreachable" => invalid.unreachable = Some("no caller".into()),
            _ => {
                invalid.hub = Some("shared intentional guard across targets".into());
                invalid.hub_targets = Some(vec!["other".into()]);
            }
        }
        f.catalogue(invalid);
        assert!(load(&path).err().unwrap().contains("exclusive"));
    }
    for command in [
        vec![],
        vec!["".into()],
        vec!["cargo".into(), "build".into()],
    ] {
        let cat = Catalogue {
            control: vec![c.clone()],
            prebuild: vec![Prebuild {
                name: "bad-step".into(),
                command,
                timeout_s: 5,
            }],
        };
        assert!(validate(f.root(), &cat).unwrap_err().contains("bad-step"));
    }
    let mut steps = f.prebuild();
    steps[0].timeout_s = 0;
    assert!(validate(
        f.root(),
        &Catalogue {
            control: vec![c.clone()],
            prebuild: steps
        }
    )
    .is_err());
    let step = f.prebuild()[0].clone();
    assert!(validate(
        f.root(),
        &Catalogue {
            control: vec![c],
            prebuild: vec![step.clone(), step]
        }
    )
    .unwrap_err()
    .contains("duplicate prebuild"));
}

#[test]
fn explore_append_records_a_visible_whole_file_platform_gate() {
    let f = Fixture::new();
    let os = std::env::consts::OS;
    let path = f.root().join("src/lib.rs");
    let source = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("#![cfg(target_os = \"{os}\")]\n{source}")).unwrap();
    f.commit();
    let output = f.cli(&[
        "explore",
        "--package",
        "session-fixture",
        "--file",
        "src/lib.rs",
        "--old",
        "value > 0",
        "--new",
        "value >= 0",
        "--append",
        "--id",
        "platform-discovered",
        "--guards",
        "fixture binary rejects zero",
        "--test-file",
        "src/lib.rs",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cat = load(&f.root().join("mutations.toml")).unwrap();
    assert_eq!(cat.control.last().unwrap().platforms, Some(vec![os.into()]));
}

#[test]
fn platform_inference_accepts_direct_modules_but_does_not_guess_compound_or_item_gates() {
    let f = Fixture::new();
    let edit = Edit {
        file: "src/desktop.rs".into(),
        old: "true".into(),
        new: "false".into(),
    };
    fs::write(
        f.root().join("src/desktop.rs"),
        "pub fn guard() -> bool { true }",
    )
    .unwrap();
    for (source, expected) in [
        (
            "#[cfg(target_os = \"macos\")]\nmod desktop;",
            Some(vec!["macos".into()]),
        ),
        ("// #[cfg(target_os = \"macos\")]\nmod desktop;", None),
        (
            "#[cfg(any(target_os = \"macos\", target_os = \"linux\"))]\nmod desktop;",
            None,
        ),
        (
            "#[cfg(target_os = \"macos\")] fn unrelated() {}\nmod desktop;",
            None,
        ),
    ] {
        fs::write(f.root().join("src/lib.rs"), source).unwrap();
        assert_eq!(
            infer_platforms(f.root(), std::slice::from_ref(&edit)),
            expected
        );
    }
}
