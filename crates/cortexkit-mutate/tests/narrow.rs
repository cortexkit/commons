#![forbid(unsafe_code)]

mod support;

use cortexkit_mutate::*;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command, sync::atomic::AtomicBool};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        support::isolate_fixture_environment();
        let f = Self(tempfile::tempdir().unwrap());
        fs::create_dir(f.root().join("src")).unwrap();
        fs::write(f.root().join("Cargo.toml"), "[workspace]\n[package]\nname = 'narrow-fixture'\nversion = '0.1.0'\nedition = '2021'\n").unwrap();
        fs::write(f.root().join("src/lib.rs"), r#"
pub fn guarded(value: i32) -> bool { value > 0 }
#[cfg(test)] mod tests {
    fn record(name: &str) {
        use std::io::Write;
        let mutant = super::guarded(0);
        writeln!(std::fs::OpenOptions::new().create(true).append(true).open(".git/test-runs").unwrap(), "{name}:{mutant}").unwrap();
    }
    #[test] fn expected() { record("expected"); assert!(!super::guarded(0)); }
    #[test] fn sibling() { record("sibling"); assert!(super::guarded(1)); }
}
"#).unwrap();
        f.command("cargo", &["generate-lockfile", "--offline"]);
        f.command("git", &["init", "-q"]);
        f.command("git", &["config", "user.email", "fixture@example.invalid"]);
        f.command("git", &["config", "user.name", "Narrow fixture"]);
        f.catalogue(vec![f.row("broad")]);
        f
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn command(&self, program: &str, args: &[&str]) {
        let out = Command::new(program)
            .args(args)
            .current_dir(self.root())
            .output()
            .unwrap();
        assert!(out.status.success(), "{program} {args:?}: {out:?}");
    }

    fn row(&self, id: &str) -> Control {
        toml::from_str(&format!(
            r#"
id = "{id}"
guards = "reject zero"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "src/lib.rs"
runner = "cargo"
package = "narrow-fixture"
target = "--lib"
expect_red = ["tests::expected"]
"#
        ))
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
            &["add", "Cargo.toml", "Cargo.lock", "src", "mutations.toml"],
        );
        self.command("git", &["commit", "--allow-empty", "-qm", "fixture"]);
    }

    fn cli(&self, args: &[&str]) -> std::process::Output {
        let source = fs::read(self.root().join("src/lib.rs")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_ckdev-mutate"))
            .args(args)
            .current_dir(self.root())
            .output()
            .unwrap();
        assert_eq!(source, fs::read(self.root().join("src/lib.rs")).unwrap());
        out
    }

    fn run(&self, args: &[&str]) -> (std::process::Output, Value) {
        let mut flags = vec!["run", "--report", ".git/report.json"];
        flags.extend(args);
        let out = self.cli(&flags);
        let report = fs::read(self.root().join(".git/report.json"))
            .unwrap_or_else(|e| panic!("no report: {e}; {out:?}"));
        (out, serde_json::from_slice(&report).unwrap())
    }

    fn runs(&self, entry: &str) -> usize {
        fs::read_to_string(self.root().join(".git/test-runs"))
            .unwrap()
            .lines()
            .filter(|line| *line == entry)
            .count()
    }
}

#[test]
fn narrow_preserves_only_own_select_command_and_hub() {
    let f = Fixture::new();
    let mut only = f.row("only");
    only.only = true;
    let mut selected = f.row("selected");
    selected.select = Some(TestSelection::Expected);
    let mut command = f.row("command");
    command.runner = "command".into();
    command.package = None;
    command.target = None;
    command.expect_red.clear();
    command.catch_on = Some("output_differs".into());
    command.command = Some(vec![
        "git".into(),
        "diff".into(),
        "--".into(),
        "src/lib.rs".into(),
    ]);
    let mut hub = f.row("hub");
    hub.hub = Some("the whole library guards this property".into());
    hub.hub_targets = Some(vec!["narrow_fixture".into()]);
    f.catalogue(vec![f.row("broad"), only, selected, command, hub]);
    let (out, report) = f.run(&["--all", "--narrow"]);
    assert!(out.status.success(), "{out:?}; {report}");
    assert_eq!(report["narrow"], true);
    assert_eq!(
        report["selection_counts"],
        json!({
            "narrowed": 1, "own_select": 1, "only": 1, "command_runner": 1, "hub": 1, "not_run": 0
        })
    );
    assert_eq!(
        report["selections"],
        json!({
            "broad": "narrowed", "selected": "own_select", "only": "only", "command": "command_runner", "hub": "hub"
        })
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("NARROW run summary"), "{stdout}");
    assert!(
        stdout.contains("1 narrowed; unchanged: own select 1, only 1, command runner 1, hub 1"),
        "{stdout}"
    );
    let rows = report["rows"].as_array().unwrap();
    for row in rows {
        assert_eq!(row["outcome"], "CAUGHT", "{row}");
        if matches!(row["id"].as_str().unwrap(), "only" | "hub") {
            assert_eq!(row["green"], json!(["tests::sibling"]), "{row}");
        } else if row["id"] != "command" {
            assert_eq!(row["green"], json!([]), "{row}");
        }
    }
    // The clean-tree baseline runs tests::expected twice: with the full library
    // for the only/hub rows, then alone for the broad/selected rows.
    assert_eq!(f.runs("expected:false"), 2);
    assert_eq!(f.runs("sibling:false"), 1);
    assert_eq!(f.runs("expected:true"), 4);
    assert_eq!(f.runs("sibling:true"), 2);
}

#[test]
fn narrow_baseline_is_distinct_from_broad_and_shared_with_expected() {
    let f = Fixture::new();
    let mut broad = f.row("broad");
    broad.only = true;
    let mut narrowed = f.row("narrowed");
    let mut expected = f.row("expected");
    expected.select = Some(TestSelection::Expected);
    assert_eq!(narrowed.narrow(), NarrowSelection::Narrowed);
    let stop = AtomicBool::new(false);
    let rows = [&broad, &narrowed, &expected];
    let mut session = ReplaySession::prepare(f.root(), &[], &rows, false, &stop).unwrap();
    session.baselines(f.root(), &rows, false, &stop).unwrap();
    assert_eq!(f.runs("expected:false"), 2);
    assert_eq!(f.runs("sibling:false"), 1);
    let mut reports = Vec::new();
    for row in rows {
        let report = session.run_row(f.root(), row, false, &stop, false).unwrap();
        assert_eq!(report.outcome, Outcome::Caught, "{report:?}");
        if row.id == "expected" {
            assert_eq!(report.baseline_build_ms, 0);
            assert_eq!(report.baseline_test_ms, 0);
        }
        reports.push(report);
    }
    session.finish(f.root(), &mut reports, false).unwrap();
}

#[test]
fn narrowed_rows_are_caught_or_survive_including_diff_replays() {
    let f = Fixture::new();
    let (out, report) = f.run(&["--all", "--narrow"]);
    assert!(out.status.success(), "{out:?}; {report}");
    assert_eq!(report["rows"][0]["outcome"], "CAUGHT");
    assert_eq!(report["rows"][0]["red"], json!(["tests::expected"]));
    assert_eq!(report["rows"][0]["green"], json!([]));
    assert_eq!(report["rows"][0]["breadth_observed"], false);
    assert_eq!(f.runs("sibling:false") + f.runs("sibling:true"), 0);

    let mut survivor = f.row("survivor");
    survivor.new = Some("value > 1".into());
    f.catalogue(vec![survivor]);
    let (out, report) = f.run(&["--diff", "HEAD~1", "--narrow"]);
    assert!(!out.status.success(), "{out:?}; {report}");
    assert_eq!(report["narrow"], true);
    assert_eq!(report["rows"][0]["id"], "survivor");
    assert_eq!(report["rows"][0]["outcome"], "SURVIVED");
    assert_eq!(report["rows"][0]["green"], json!(["tests::expected"]));
}

#[test]
fn narrow_shards_and_weight_reports_preserve_row_assignment() {
    let f = Fixture::new();
    f.catalogue(["a", "b", "c", "d"].map(|id| f.row(id)).to_vec());
    let (out, report) = f.run(&["--all", "--narrow", "--shard", "1/2"]);
    assert!(out.status.success(), "{out:?}; {report}");
    assert_eq!(report["selection_counts"]["narrowed"], 2);
    assert_eq!(report["rows"][0]["id"], "a");
    assert_eq!(report["rows"][1]["id"], "c");
    fs::copy(
        f.root().join(".git/report.json"),
        f.root().join(".git/weights.json"),
    )
    .unwrap();
    let flags = [
        "run",
        "--all",
        "--shard",
        "2/2",
        "--shard-weights",
        ".git/weights.json",
        "--list",
    ];
    let broad_list = f.cli(&flags);
    let mut narrow_flags = flags.to_vec();
    narrow_flags.push("--narrow");
    let narrow_list = f.cli(&narrow_flags);
    assert!(broad_list.status.success(), "{broad_list:?}");
    assert!(narrow_list.status.success(), "{narrow_list:?}");
    assert_eq!(broad_list.stdout, narrow_list.stdout);
    let listed: Vec<_> = String::from_utf8(narrow_list.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let (out, report) = f.run(&[
        "--all",
        "--narrow",
        "--shard",
        "2/2",
        "--shard-weights",
        ".git/weights.json",
    ]);
    assert!(out.status.success(), "{out:?}; {report}");
    assert_eq!(report["selection_counts"]["narrowed"], 2);
    let ran: Vec<_> = report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(ran, listed);
}

#[test]
fn empty_narrow_report_cannot_look_like_a_broad_pass() {
    let f = Fixture::new();
    let (out, report) = f.run(&["--diff", "HEAD", "--narrow"]);
    assert!(out.status.success(), "{out:?}; {report}");
    assert_eq!(report["narrow"], true);
    assert_eq!(report["rows"], json!([]));
    assert_eq!(report["selection_counts"]["narrowed"], 0);
    assert!(String::from_utf8_lossy(&out.stdout).contains("NARROW run summary"));

    let (out, report) = f.run(&["--all"]);
    assert!(out.status.success(), "{out:?}; {report}");
    assert!(
        report.is_array(),
        "default reports must keep their array format"
    );
    assert_eq!(report[0]["green"], json!(["tests::sibling"]));
}
