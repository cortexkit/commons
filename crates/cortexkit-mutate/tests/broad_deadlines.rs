#![forbid(unsafe_code)]

mod support;

use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new(own: &str, other: &str) -> Self {
        support::isolate_fixture_environment();
        let f = Self(tempfile::tempdir().unwrap());
        fs::create_dir_all(f.root().join("src")).unwrap();
        fs::create_dir(f.root().join("tests")).unwrap();
        fs::write(f.root().join("Cargo.toml"), "[workspace]\n[package]\nname = 'deadline-fixture'\nversion = '0.1.0'\nedition = '2021'\n").unwrap();
        fs::write(
            f.root().join("src/lib.rs"),
            "pub fn guarded() -> bool { true }\n",
        )
        .unwrap();
        for (name, body) in [("own", own), ("other", other)] {
            fs::write(
                f.root().join(format!("tests/{name}.rs")),
                format!("#[test] fn {name}_guard() {{ {body} }}\n"),
            )
            .unwrap();
        }
        f.cmd("cargo", &["generate-lockfile", "--offline"]);
        f.cmd("git", &["init", "-q"]);
        f.cmd("git", &["config", "user.email", "fixture@example.invalid"]);
        f.cmd("git", &["config", "user.name", "Deadline fixture"]);
        f
    }

    fn root(&self) -> &Path {
        self.0.path()
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

    fn run(&self, timeouts: &[u64], runner: &str) -> serde_json::Value {
        self.run_selected(timeouts, runner, "--test own", "")
    }

    fn run_selected(
        &self,
        timeouts: &[u64],
        runner: &str,
        target: &str,
        settings: &str,
    ) -> serde_json::Value {
        self.run_configured(timeouts, runner, target, settings, |_| {})
    }

    fn run_configured(
        &self,
        timeouts: &[u64],
        runner: &str,
        target: &str,
        settings: &str,
        configure: impl FnOnce(&mut Command),
    ) -> serde_json::Value {
        let rows = timeouts
            .iter()
            .enumerate()
            .map(|(index, timeout)| {
                format!(
                    r#"
[[control]]
id = "row-{index}"
guards = "the guard stays enabled"
file = "src/lib.rs"
old = "{{ true }}"
new = "{{ false }}"
test_file = "tests/own.rs"
runner = "{runner}"
package = "deadline-fixture"
target = "{target}"
expect_red = ["own_guard"]
timeout_s = {timeout}
{settings}
"#
                )
            })
            .collect::<String>();
        fs::write(self.root().join("mutations.toml"), rows).unwrap();
        self.cmd(
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
        self.cmd("git", &["commit", "--allow-empty", "-qm", "fixture"]);
        let before = fs::read(self.root().join("src/lib.rs")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_ckdev-mutate"));
        command
            .args(["run", "--all", "--broad", "--report", ".git/report.json"])
            .current_dir(self.root());
        configure(&mut command);
        let output = command.output().unwrap();
        assert_eq!(before, fs::read(self.root().join("src/lib.rs")).unwrap());
        self.cmd("git", &["diff", "--exit-code"]);
        let report = fs::read(self.root().join(".git/report.json"))
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stderr)));
        serde_json::from_slice(&report).unwrap()
    }
}

fn runners() -> Vec<&'static str> {
    if Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|out| out.status.success())
    {
        vec!["cargo", "nextest"]
    } else {
        assert_ne!(
            std::env::var("CK_MUTATE_REQUIRE_NEXTEST").as_deref(),
            Ok("1"),
            "nextest is required"
        );
        vec!["cargo"]
    }
}

#[test]
fn broad_targets_get_independent_baseline_and_mutant_deadlines() {
    for runner in runners() {
        // Each target fits two seconds, but the pair cannot fit one deadline.
        let f = Fixture::new(
            "std::thread::sleep(std::time::Duration::from_millis(1100)); assert!(deadline_fixture::guarded());",
            "std::thread::sleep(std::time::Duration::from_millis(1100));",
        );
        let rows = f.run(&[2], runner);
        let row = &rows[0];
        assert_eq!(row["outcome"], "CAUGHT", "{runner}: {rows}");
        assert!(row["baseline_test_ms"].as_u64().unwrap() > 2000, "{row}");
        assert!(row["test_ms"].as_u64().unwrap() > 2000, "{row}");
        for phase in ["baseline_targets", "targets"] {
            let targets = row[phase].as_array().unwrap();
            assert_eq!(targets.len(), 3, "{runner}: {row}");
            assert!(
                targets
                    .iter()
                    .all(|target| target["outcome"] != "TIMED_OUT"),
                "{row}"
            );
            assert!(
                targets
                    .iter()
                    .any(|target| target["red"] == serde_json::json!(["own_guard"]))
                    || phase == "baseline_targets",
                "{row}"
            );
        }
    }
}

#[test]
fn broad_baseline_sharing_separates_deadlines_in_either_order() {
    for runner in runners() {
        for timeouts in [[1, 3, 3], [3, 1, 3]] {
            let f = Fixture::new(
                "std::thread::sleep(std::time::Duration::from_millis(1100)); assert!(deadline_fixture::guarded());",
                "",
            );
            let rows = f.run(&timeouts, runner);
            for (index, timeout) in timeouts.into_iter().enumerate() {
                let row = &rows[index];
                assert_eq!(
                    row["outcome"],
                    if timeout == 1 { "TIMED_OUT" } else { "CAUGHT" },
                    "{runner}: {rows}"
                );
                if timeout == 1 {
                    assert_eq!(row["timed_out_phase"], "test");
                    let timed_out = row["baseline_targets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|target| target["outcome"] == "TIMED_OUT")
                        .collect::<Vec<_>>();
                    assert_eq!(timed_out.len(), 1, "{row}");
                    let target = timed_out[0]["target"].as_str().unwrap();
                    assert!(
                        row["reason"]
                            .as_str()
                            .unwrap()
                            .contains(&format!("target {target} timed out")),
                        "{row}"
                    );
                    assert!(
                        row.get("targets").is_none(),
                        "mutant ran after baseline timeout: {row}"
                    );
                }
            }
            assert_eq!(
                rows[2]["baseline_test_ms"], 0,
                "compatible deadlines should still share: {rows}"
            );
        }
    }
}

#[test]
fn broad_cross_target_only_catch_keeps_collateral_and_wrong_test_grading() {
    for runner in runners() {
        let f = Fixture::new("", "assert!(deadline_fixture::guarded());");
        let rows = f.run(&[3], runner);
        assert_eq!(rows[0]["outcome"], "WRONG_TEST", "{runner}: {rows}");
        assert_eq!(rows[0]["red"], serde_json::json!(["other_guard"]));
        assert_eq!(rows[0]["collateral"]["count"], 1);
        assert!(rows[0]["collateral"]["targets"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("other"));
        assert!(rows[0]["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |target| target["target"].as_str().unwrap().contains("other")
                    && target["red"] == serde_json::json!(["other_guard"])
            ));
    }
}

#[test]
fn broad_mutant_timeout_names_its_target_and_phase() {
    for runner in runners() {
        let f = Fixture::new(
            "assert!(deadline_fixture::guarded());",
            "if !deadline_fixture::guarded() { std::thread::sleep(std::time::Duration::from_secs(10)); }",
        );
        let rows = f.run(&[1], runner);
        let row = &rows[0];
        assert_eq!(row["outcome"], "TIMED_OUT", "{runner}: {rows}");
        assert_eq!(row["timed_out_phase"], "test");
        assert!(row["reason"].as_str().unwrap().contains("other"), "{row}");
        assert!(
            row["targets"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |target| target["target"].as_str().unwrap().contains("other")
                        && target["outcome"] == "TIMED_OUT"
                ),
            "{row}"
        );
    }
}

#[test]
fn broad_retains_explicit_examples_benches_and_qualifies_duplicate_names() {
    for runner in runners() {
        let f = Fixture::new("assert!(deadline_fixture::guarded());", "");
        let manifest = f.root().join("Cargo.toml");
        let source = fs::read_to_string(&manifest).unwrap();
        fs::write(manifest, format!("{source}\n[features]\naudit = []\n[[example]]\nname = 'extra'\npath = 'extra.rs'\n[[bench]]\nname = 'audit'\npath = 'audit.rs'\n")).unwrap();
        for path in ["extra.rs", "audit.rs"] {
            fs::write(f.root().join(path), "#[cfg(not(feature = \"audit\"))] compile_error!(\"missing feature\");\n#[test] #[ignore] fn duplicate() { assert!(deadline_fixture::guarded()); }\n").unwrap();
        }
        f.cmd("git", &["add", "extra.rs", "audit.rs"]);
        let rows = f.run_selected(
            &[3],
            runner,
            "--test own --example extra --bench audit",
            "features = ['audit']\nignored = 'include'",
        );
        let row = &rows[0];
        assert_eq!(row["outcome"], "CAUGHT_BROADLY", "{runner}: {rows}");
        assert_eq!(row["collateral"]["count"], 2);
        let red = row["red"].as_array().unwrap();
        assert_eq!(red.len(), 3, "{row}");
        assert!(
            red.iter()
                .any(|name| name.as_str().unwrap().ends_with("extra::duplicate")),
            "{row}"
        );
        assert!(
            red.iter()
                .any(|name| name.as_str().unwrap().ends_with("audit::duplicate")),
            "{row}"
        );
        assert_eq!(row["targets"].as_array().unwrap().len(), 5, "{row}");
    }
}

#[test]
fn broad_keeps_custom_harness_test_targets() {
    let f = Fixture::new("assert!(deadline_fixture::guarded());", "");
    let manifest = f.root().join("Cargo.toml");
    let source = fs::read_to_string(&manifest).unwrap();
    fs::write(
        manifest,
        format!("{source}\n[[test]]\nname = 'custom'\npath = 'custom.rs'\nharness = false\n"),
    )
    .unwrap();
    fs::write(f.root().join("custom.rs"), r#"
fn main() {
    println!("running 1 test");
    let green = deadline_fixture::guarded();
    println!("test custom_guard ... {}", if green { "ok" } else { "FAILED" });
    println!("test result: {}. {} passed; {} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s", if green { "ok" } else { "FAILED" }, u8::from(green), u8::from(!green));
    std::process::exit(i32::from(!green));
}
"#).unwrap();
    f.cmd("git", &["add", "custom.rs"]);
    let rows = f.run(&[3], "cargo");
    assert_eq!(rows[0]["outcome"], "CAUGHT_BROADLY", "{rows}");
    assert!(
        rows[0]["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["target"] == "custom"
                && target["red"] == serde_json::json!(["custom_guard"])),
        "{rows}"
    );
}

#[cfg(unix)]
#[test]
fn broad_nextest_metadata_timeout_is_a_build_timeout() {
    use std::os::unix::fs::PermissionsExt;
    if !runners().contains(&"nextest") {
        return;
    }
    let f = Fixture::new("assert!(deadline_fixture::guarded());", "");
    let bin = f.root().join(".git/wrapper-bin");
    fs::create_dir(&bin).unwrap();
    let wrapper = bin.join("cargo");
    fs::write(
        &wrapper,
        format!(
            r#"#!/usr/bin/env python3
import os, sys, time
if sys.argv[1:2] == ["metadata"]:
    time.sleep(10)
real = {}
os.execv(real, [real] + sys.argv[1:])
"#,
            serde_json::to_string(env!("CARGO")).unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let rows = f.run_configured(
        &[1],
        "nextest",
        "--test own",
        "build_timeout_s = 4",
        |command| {
            command.env("PATH", path);
        },
    );
    let row = &rows[0];
    assert_eq!(row["outcome"], "TIMED_OUT", "{row}");
    assert_eq!(row["timed_out_phase"], "build", "{row}");
    assert!(
        row["baseline_failure"]["command"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "metadata"),
        "{row}"
    );
    assert!(row.get("targets").is_none(), "{row}");
}

#[cfg(unix)]
#[test]
fn broad_cargo_freshness_guard_rejects_rebuild_and_names_target() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("assert!(deadline_fixture::guarded());", "");
    let bin = f.root().join(".git/freshness-wrapper-bin");
    fs::create_dir(&bin).unwrap();
    let wrapper = bin.join("cargo");
    fs::write(&wrapper, r#"#!/usr/bin/env python3
import json, sys
artifact = {
    "reason": "compiler-artifact", "profile": {"test": True},
    "target": {"name": "own", "kind": ["test"], "test": True},
    "executable": "/unused/own-0123456789abcdef", "fresh": True,
}
if "--no-run" in sys.argv:
    print(json.dumps(artifact))
elif "--list" in sys.argv:
    print("     Running tests/own.rs (/unused/own-0123456789abcdef)")
    if "--ignored" not in sys.argv:
        print("own_guard: test")
else:
    artifact["fresh"] = False
    artifact["target"]["name"] = "rebuilt-dependency"
    print(json.dumps(artifact))
    print("     Running tests/own.rs (/unused/own-0123456789abcdef)")
    print("running 1 test")
    print("test own_guard ... ok")
    print("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s")
"#).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let rows = f.run_configured(&[3], "cargo", "--test own", "", |command| {
        command.env("PATH", path);
    });
    let row = &rows[0];
    assert_eq!(row["outcome"], "ERROR", "{row}");
    assert_eq!(
        row["reason"], "own: Cargo rebuilt a target after the broad build (rebuilt-dependency)",
        "{row}"
    );
    assert!(
        row.get("targets").is_none(),
        "mutant ran after a rebuild: {row}"
    );
}
