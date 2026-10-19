//! Deadlines on the test processes a row spawns, and the progress lines `run`
//! prints on stderr. The hanging tests start a helper in a new process group,
//! as nextest does for every test, so these tests can tell a kill of the whole
//! process tree from a kill of only the spawned command's own process group.
#![forbid(unsafe_code)]

mod support;

use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use tempfile::TempDir;

/// Deadline for the fixture row's test runs. Short, but long enough for an
/// already-built fixture's green tests to finish on a loaded machine.
const TIMEOUT_S: u64 = 4;

const LIB: &str = r#"pub fn guarded(value: i32) -> bool {
    value > 0
}

/// Start `sleep` in a new process group, the way nextest starts each test,
/// record this process's pid and the helper's pid, then sleep for longer than
/// any deadline the tests use.
#[cfg(unix)]
pub fn hang_with_regrouped_child() {
    use std::os::unix::process::CommandExt;
    let child = std::process::Command::new("sleep")
        .arg("60")
        .process_group(0)
        .spawn()
        .unwrap();
    std::fs::write(".git/hang-pids.part", format!("{} {}", std::process::id(), child.id())).unwrap();
    std::fs::rename(".git/hang-pids.part", ".git/hang-pids").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[cfg(test)]
mod tests {
    #[test]
    fn guard_rejects_zero() {
        #[cfg(unix)]
        if std::path::Path::new("hang-baseline").exists() {
            super::hang_with_regrouped_child();
        }
        assert!(!super::guarded(0));
    }
}
"#;

/// A second test target, `tests/collateral.rs`. The fixture's one catalogue
/// row names only the library test `tests::guard_rejects_zero`, so a normal
/// replay never runs this target; `run --broad` runs every test target of the
/// package, which makes this one collateral. It hangs only on the mutant, so
/// only a `--broad` mutant run hangs.
const COLLATERAL: &str = r#"#[test]
fn collateral_hangs_on_the_mutant() {
    #[cfg(unix)]
    if timeout_fixture::guarded(0) {
        timeout_fixture::hang_with_regrouped_child();
    }
}
"#;

const CATALOGUE: &str = r#"[[control]]
id = "collateral-hang"
guards = "the guard rejects zero"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "src/lib.rs"
runner = "cargo"
package = "timeout-fixture"
target = "--lib"
expect_red = ["tests::guard_rejects_zero"]
"#;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        support::isolate_fixture_environment();
        let fixture = Self(tempfile::tempdir().unwrap());
        let root = fixture.root();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\n[package]\nname = 'timeout-fixture'\nversion = '0.1.0'\nedition = '2021'\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), LIB).unwrap();
        fs::write(root.join("tests/collateral.rs"), COLLATERAL).unwrap();
        fs::write(
            root.join("mutations.toml"),
            format!("{CATALOGUE}timeout_s = {TIMEOUT_S}\n"),
        )
        .unwrap();
        fixture.cmd("cargo", &["generate-lockfile", "--offline"]);
        fixture.cmd("git", &["init", "-q"]);
        fixture.cmd("git", &["config", "user.email", "fixture@example.invalid"]);
        fixture.cmd("git", &["config", "user.name", "Timeout fixture"]);
        fixture.cmd("git", &["add", "."]);
        fixture.cmd("git", &["commit", "-qm", "fixture"]);
        fixture
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

    /// Run the CLI and return its stdout, stderr and JSON report row.
    fn run(&self, args: &[&str]) -> (String, String, serde_json::Value) {
        let output = Command::new(env!("CARGO_BIN_EXE_ckdev-mutate"))
            .args(args)
            .args(["--report", ".git/report.json"])
            .current_dir(self.root())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let report: serde_json::Value = serde_json::from_slice(
            &fs::read(self.root().join(".git/report.json"))
                .unwrap_or_else(|e| panic!("no report ({e}):\n{stdout}\n{stderr}")),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(self.root().join("src/lib.rs")).unwrap(),
            LIB
        );
        (stdout, stderr, report[0].clone())
    }

    /// The pids the hanging test recorded, each with what it is: the test
    /// process, then the helper it started in a new process group.
    fn hang_pids(&self) -> Vec<(&'static str, String)> {
        let pids = fs::read_to_string(self.root().join(".git/hang-pids"))
            .expect("the hanging test started");
        let pids: Vec<&str> = pids.split_whitespace().collect();
        assert_eq!(pids.len(), 2, "{pids:?}");
        vec![
            ("hanging test process", pids[0].to_owned()),
            ("its regrouped helper", pids[1].to_owned()),
        ]
    }
}

/// Assert each pid is gone within a second of the run returning. The runner
/// waits for the processes it killed, so they must already be gone, not merely
/// about to go. The check is a signal-0 probe, not `ps`: macOS sandboxes may
/// refuse to run the setuid `ps`, and a check that cannot run must not pass.
#[cfg(unix)]
fn assert_gone(pids: &[(&str, String)]) {
    use rustix::process::{kill_process, test_kill_process, Pid, Signal};
    let start = Instant::now();
    for (role, pid) in pids {
        let target = Pid::from_raw(pid.parse().unwrap()).unwrap();
        while test_kill_process(target).is_ok() {
            if start.elapsed() > Duration::from_secs(1) {
                // An unreaped zombie has exited; anything else survived.
                let state = Command::new("ps")
                    .args(["-p", pid, "-o", "stat="])
                    .output()
                    .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
                    .unwrap_or_default();
                if state.starts_with('Z') {
                    break;
                }
                // Do not leave the survivor running after the failed test.
                let _ = kill_process(target, Signal::KILL);
                panic!("{role} {pid} survived the deadline (state {state:?})");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Assert `ms` is at least the row's deadline but not far beyond it: the
/// deadline, the SIGTERM grace period and slack for a loaded machine.
fn assert_within_budget(ms: &serde_json::Value) {
    let ms = ms.as_u64().unwrap();
    assert!(
        (TIMEOUT_S * 1000..(TIMEOUT_S + 2 + 3) * 1000).contains(&ms),
        "run took {ms} ms for timeout_s = {TIMEOUT_S}"
    );
}

#[cfg(unix)]
#[test]
fn hung_broad_collateral_target_times_out_and_leaves_no_process() {
    let f = Fixture::new();
    let (stdout, stderr, row) = f.run(&["run", "--all", "--broad"]);
    assert_eq!(row["outcome"], "TIMED_OUT", "{row}\n{stderr}");
    assert_eq!(row["timed_out_phase"], "test");
    assert_within_budget(&row["test_ms"]);
    // The hanging test and the helper it put in its own process group.
    assert_gone(&f.hang_pids());
    let reason = row["reason"].as_str().unwrap();
    assert!(
        reason.starts_with(&format!("test run exceeded timeout_s = {TIMEOUT_S}")),
        "{reason}"
    );
    // The reason names the target that hung and what was still running.
    assert!(
        reason.contains("last test target started: tests/collateral.rs"),
        "{reason}"
    );
    assert!(
        reason.contains("still running: collateral-") && reason.contains("sleep 60 (pid "),
        "{reason}"
    );
    assert!(
        stderr.contains("progress: row 1/1 collateral-hang: started\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("progress: row 1/1 collateral-hang: TIMED_OUT ("),
        "{stderr}"
    );
    assert!(!stdout.contains("progress:"), "{stdout}");
}

#[cfg(unix)]
#[test]
fn hung_baseline_times_out_and_leaves_no_process() {
    let f = Fixture::new();
    fs::write(f.root().join("hang-baseline"), "").unwrap();
    let (_, stderr, row) = f.run(&["run", "--all"]);
    assert_eq!(row["outcome"], "TIMED_OUT", "{row}\n{stderr}");
    assert_eq!(row["timed_out_phase"], "test");
    assert_within_budget(&row["baseline_test_ms"]);
    assert_gone(&f.hang_pids());
    // The mutant was never built or run.
    assert_eq!(row["build_ms"], 0);
    assert_eq!(row["test_ms"], 0);
    let reason = row["reason"].as_str().unwrap();
    assert!(
        reason.starts_with(&format!(
            "baseline test run exceeded timeout_s = {TIMEOUT_S}"
        )),
        "{reason}"
    );
    assert!(
        reason.contains("last test target started: unittests src/lib.rs"),
        "{reason}"
    );
    assert!(reason.contains("sleep 60 (pid "), "{reason}");
    assert!(
        stderr.contains("progress: baseline 1/1 collateral-hang: TIMED_OUT ("),
        "{stderr}"
    );
    assert!(
        stderr.contains("progress: row 1/1 collateral-hang: TIMED_OUT ("),
        "{stderr}"
    );
}

#[test]
fn finished_rows_print_progress_on_stderr_only() {
    let f = Fixture::new();
    let (stdout, stderr, row) = f.run(&["run", "--all"]);
    assert_eq!(row["outcome"], "CAUGHT", "{row}\n{stderr}");
    let progress: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("progress: "))
        .collect();
    assert_eq!(progress.len(), 4, "{stderr}");
    assert_eq!(
        progress[0],
        "progress: baseline 1/1 collateral-hang: started"
    );
    assert_eq!(progress[2], "progress: row 1/1 collateral-hang: started");
    let seconds = regex::Regex::new(r"^\((\d+\.\d) s\)$").unwrap();
    for (line, grade) in [
        (progress[1], "baseline 1/1 collateral-hang: ready"),
        (progress[3], "row 1/1 collateral-hang: CAUGHT"),
    ] {
        let rest = line
            .strip_prefix(&format!("progress: {grade} "))
            .unwrap_or_else(|| panic!("{line:?} does not report {grade:?}"));
        assert!(seconds.is_match(rest), "{line:?}");
    }
    assert!(!stdout.contains("progress"), "{stdout}");
    assert!(
        stdout.contains("collateral-hang (") && stdout.contains("): CAUGHT"),
        "{stdout}"
    );
}
