#![forbid(unsafe_code)]

mod support;

use cortexkit_mutate::TreeLock;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

const LF_SOURCE: &str = "pub fn guarded(value: i32) -> bool { value > 0 }\n#[cfg(test)] mod tests {\n    #[test] fn rejects_zero() { assert!(!super::guarded(0)); }\n}\n";

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        Self::with_autocrlf(false)
    }

    fn with_autocrlf(autocrlf: bool) -> Self {
        let fixture = Self(tempfile::tempdir().unwrap());
        fs::create_dir(fixture.root().join("src")).unwrap();
        fs::write(
            fixture.root().join("Cargo.toml"),
            "[workspace]\n[package]\nname = 'git-storage-fixture'\nversion = '0.1.0'\nedition = '2021'\n",
        )
        .unwrap();
        // Mixed line endings make restoration a byte-level contract, not a text comparison.
        fs::write(
            fixture.root().join("src/lib.rs"),
            if autocrlf {
                LF_SOURCE.as_bytes()
            } else {
                b"pub fn guarded(value: i32) -> bool { value > 0 }\r\n#[cfg(test)] mod tests {\n    #[test] fn rejects_zero() { assert!(!super::guarded(0)); }\r\n}\n"
            },
        )
        .unwrap();
        fs::write(
            fixture.root().join("mutations.toml"),
            r#"[[control]]
id = "rejects-zero"
guards = "zero is rejected"
file = "src/lib.rs"
old = "value > 0"
new = "value >= 0"
test_file = "src/lib.rs"
runner = "cargo"
package = "git-storage-fixture"
target = "--lib"
expect_red = ["tests::rejects_zero"]
only = true
"#,
        )
        .unwrap();
        fixture.command("cargo", &["generate-lockfile", "--offline"]);
        fixture.command("git", &["init", "-q"]);
        // Configure each repository explicitly so global Git settings cannot
        // change whether its source is mixed-ending or an autocrlf checkout.
        fixture.command(
            "git",
            &[
                "config",
                "core.autocrlf",
                if autocrlf { "true" } else { "false" },
            ],
        );
        // `git commit` starts `git maintenance run --auto --detach`, which keeps
        // creating and removing lock files in .git after the commit returns.
        // The read-only test walks .git and chmods every entry, so a lock file
        // vanishing mid-walk fails it; the fixture turns maintenance off.
        fixture.command("git", &["config", "maintenance.auto", "false"]);
        fixture.command("git", &["config", "user.email", "fixture@example.invalid"]);
        fixture.command("git", &["config", "user.name", "Git storage fixture"]);
        fixture.command(
            "git",
            &["add", "Cargo.toml", "Cargo.lock", "src", "mutations.toml"],
        );
        fixture.command("git", &["commit", "-qm", "fixture"]);
        if autocrlf {
            // Force a real checkout to convert the committed LF source to CRLF.
            fs::remove_file(fixture.root().join("src/lib.rs")).unwrap();
            fixture.command("git", &["checkout", "--", "src/lib.rs"]);
        }
        fixture
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn command(&self, program: &str, args: &[&str]) -> Vec<u8> {
        let output = Command::new(program)
            .args(args)
            .current_dir(self.root())
            .output()
            .unwrap();
        assert!(output.status.success(), "{program} {args:?}: {output:?}");
        output.stdout
    }

    fn cli(root: &Path, args: &[&str], temp: &Path) -> std::process::Output {
        support::isolate_fixture_environment();
        Command::new(env!("CARGO_BIN_EXE_ckdev-mutate"))
            .args(args)
            .current_dir(root)
            .env("TMPDIR", temp)
            .env("TMP", temp)
            .env("TEMP", temp)
            .output()
            .unwrap()
    }
}

#[test]
fn run_accepts_autocrlf_checkout_and_restores_crlf() {
    let fixture = Fixture::with_autocrlf(true);
    let temp = tempfile::tempdir().unwrap();
    let source = fs::read(fixture.root().join("src/lib.rs")).unwrap();
    assert_eq!(source, LF_SOURCE.replace('\n', "\r\n").as_bytes());
    assert_eq!(
        fixture.command("git", &["show", "HEAD:src/lib.rs"]),
        LF_SOURCE.as_bytes()
    );
    let output = Fixture::cli(
        fixture.root(),
        &["run", "--all", "--report", "report.json"],
        temp.path(),
    );
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.root().join("report.json")).unwrap()).unwrap();
    assert_eq!(report.as_array().unwrap().len(), 1, "{report}");
    assert_eq!(report[0]["outcome"], "CAUGHT", "{report}");
    assert_eq!(report[0]["red"], serde_json::json!(["tests::rejects_zero"]));
    assert_eq!(source, fs::read(fixture.root().join("src/lib.rs")).unwrap());
    assert!(fixture
        .command(
            "git",
            &["status", "--porcelain=v1", "-z", "--", "src/lib.rs"]
        )
        .is_empty());
}

#[test]
fn run_refuses_unstaged_edit_without_overwriting() {
    let fixture = Fixture::with_autocrlf(true);
    let temp = tempfile::tempdir().unwrap();
    let source = LF_SOURCE.replace('\n', "\r\n") + "// local edit\r\n";
    fs::write(fixture.root().join("src/lib.rs"), &source).unwrap();
    assert_eq!(
        fixture.command(
            "git",
            &["status", "--porcelain=v1", "-z", "--", "src/lib.rs"]
        ),
        b" M src/lib.rs\0"
    );
    let output = Fixture::cli(
        fixture.root(),
        &["run", "--all", "--report", "report.json"],
        temp.path(),
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("dirty target refused: src/lib.rs (use --allow-dirty)"),
        "{output:?}"
    );
    assert_eq!(
        source.as_bytes(),
        fs::read(fixture.root().join("src/lib.rs")).unwrap()
    );
    assert_eq!(
        fixture.command("git", &["show", ":src/lib.rs"]),
        LF_SOURCE.as_bytes()
    );
}

#[test]
fn run_refuses_staged_only_edit_without_overwriting() {
    let fixture = Fixture::with_autocrlf(true);
    let temp = tempfile::tempdir().unwrap();
    let staged = LF_SOURCE.to_owned() + "// staged edit\n";
    let source = staged.replace('\n', "\r\n");
    fs::write(fixture.root().join("src/lib.rs"), &source).unwrap();
    fixture.command("git", &["add", "src/lib.rs"]);
    assert_eq!(
        fixture.command(
            "git",
            &["status", "--porcelain=v1", "-z", "--", "src/lib.rs"]
        ),
        b"M  src/lib.rs\0"
    );
    let output = Fixture::cli(
        fixture.root(),
        &["run", "--all", "--report", "report.json"],
        temp.path(),
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("dirty target refused: src/lib.rs (use --allow-dirty)"),
        "{output:?}"
    );
    assert_eq!(
        source.as_bytes(),
        fs::read(fixture.root().join("src/lib.rs")).unwrap()
    );
    assert_eq!(
        fixture.command("git", &["show", ":src/lib.rs"]),
        staged.as_bytes()
    );
}

fn assert_only_lock_remains(temp: &Path) {
    let entries: Vec<_> = fs::read_dir(temp)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries.len(), 1, "capture files leaked: {entries:?}");
    let name = entries[0].to_str().unwrap();
    let digest = name
        .strip_prefix("ck-mutate-")
        .unwrap()
        .strip_suffix(".lock")
        .unwrap();
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[cfg(unix)]
struct ReadOnlyGit(Vec<(std::path::PathBuf, fs::Permissions)>);

#[cfg(unix)]
impl ReadOnlyGit {
    fn new(path: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;

        fn collect(path: &Path, entries: &mut Vec<(std::path::PathBuf, fs::Permissions)>) {
            let metadata = fs::metadata(path).unwrap();
            entries.push((path.to_owned(), metadata.permissions()));
            if metadata.is_dir() {
                for entry in fs::read_dir(path).unwrap() {
                    collect(&entry.unwrap().path(), entries);
                }
            }
        }

        let mut guard = Self(vec![]);
        collect(path, &mut guard.0);
        for (path, permissions) in &guard.0 {
            fs::set_permissions(
                path,
                fs::Permissions::from_mode(permissions.mode() & !0o222),
            )
            .unwrap();
        }
        guard
    }
}

#[cfg(unix)]
impl Drop for ReadOnlyGit {
    fn drop(&mut self) {
        for (path, permissions) in &self.0 {
            fs::set_permissions(path, permissions.clone()).expect("restore Git permissions");
        }
    }
}

#[cfg(unix)]
#[test]
fn run_with_read_only_git_restores_source_and_cleans_captures() {
    let fixture = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let source = fs::read(fixture.root().join("src/lib.rs")).unwrap();
    let cargo_lock = fs::read(fixture.root().join("Cargo.lock")).unwrap();
    let git_dir = fixture.root().join(".git");
    let permissions = ReadOnlyGit::new(&git_dir);
    // Fail loudly under root or any environment that bypasses chmod; otherwise
    // the read-only fixture would not exercise the storage restriction at all.
    assert!(
        fs::write(git_dir.join("write-probe"), b"forbidden").is_err(),
        "Git directory must actually refuse writes"
    );
    let output = Fixture::cli(
        fixture.root(),
        &["run", "--all", "--report", "report.json"],
        temp.path(),
    );
    assert_eq!(source, fs::read(fixture.root().join("src/lib.rs")).unwrap());
    assert_eq!(
        cargo_lock,
        fs::read(fixture.root().join("Cargo.lock")).unwrap()
    );
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.root().join("report.json")).unwrap()).unwrap();
    assert_eq!(report[0]["outcome"], "CAUGHT", "{report}");
    assert_eq!(report[0]["red"], serde_json::json!(["tests::rejects_zero"]));
    assert_only_lock_remains(temp.path());
    drop(permissions);
}

#[test]
fn sessions_on_one_worktree_exclude_each_other_and_release() {
    let fixture = Fixture::new();
    // The child must use the same temporary directory as this process's lock.
    let lock = TreeLock::acquire(fixture.root()).unwrap();
    let output = Fixture::cli(fixture.root(), &["check"], &std::env::temp_dir());
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("mutation tree is locked:"));
    drop(lock);
    let output = Fixture::cli(fixture.root(), &["check"], &std::env::temp_dir());
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn sessions_on_linked_worktrees_do_not_block_each_other() {
    let fixture = Fixture::new();
    let linked = tempfile::tempdir().unwrap();
    let root = linked.path().join("linked");
    fixture.command(
        "git",
        &[
            "worktree",
            "add",
            "--detach",
            root.to_str().unwrap(),
            "HEAD",
        ],
    );
    let _lock = TreeLock::acquire(fixture.root()).unwrap();
    let _linked_lock = TreeLock::acquire(&root).unwrap();
    assert!(TreeLock::acquire(&root).is_err());
    drop(_linked_lock);
    let output = Fixture::cli(&root, &["check"], &std::env::temp_dir());
    assert!(output.status.success(), "{output:?}");
}

#[cfg(unix)]
#[test]
fn canonical_worktree_aliases_share_one_lock() {
    let fixture = Fixture::new();
    let aliases = tempfile::tempdir().unwrap();
    let alias = aliases.path().join("alias");
    std::os::unix::fs::symlink(fixture.root(), &alias).unwrap();
    let lock = TreeLock::acquire(fixture.root()).unwrap();
    assert!(TreeLock::acquire(&alias).is_err());
    drop(lock);
    assert!(TreeLock::acquire(&alias).is_ok());
}

#[test]
fn spawn_error_cleans_captures_and_restores_source() {
    let fixture = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let source = fs::read(fixture.root().join("src/lib.rs")).unwrap();
    let catalogue = fs::read_to_string(fixture.root().join("mutations.toml"))
        .unwrap()
        .replace("runner = \"cargo\"", "runner = \"command\"")
        .replace("package = \"git-storage-fixture\"\n", "")
        .replace("target = \"--lib\"\n", "")
        .replace("only = true", "only = false");
    // Exercise both merged output and the separate stdout/stderr capture path.
    for separate in [false, true] {
        let (catalogue, args, fields) = if separate {
            (
                catalogue.replace("expect_red = [\"tests::rejects_zero\"]", "expect_red = []"),
                "",
                "catch_on = \"output_differs\"",
            )
        } else {
            (
                catalogue.clone(),
                ", \"{test}\"",
                "test_count_pattern = \"Ran {count} tests\"",
            )
        };
        fs::write(
            fixture.root().join("mutations.toml"),
            format!(
                "{catalogue}\ncommand = [{:?}{args}]\n{fields}\n",
                fixture.root().join("nonexistent-runner").to_str().unwrap()
            ),
        )
        .unwrap();
        let output = Fixture::cli(
            fixture.root(),
            &["run", "--all", "--report", "report.json"],
            temp.path(),
        );
        assert!(!output.status.success(), "{output:?}");
        let report: serde_json::Value = serde_json::from_slice(
            &fs::read(fixture.root().join("report.json"))
                .unwrap_or_else(|error| panic!("{output:?}: {error}")),
        )
        .unwrap();
        assert_eq!(report[0]["outcome"], "ERROR", "{report}");
        assert!(
            report[0]["reason"].as_str().unwrap().contains("os error"),
            "{report}"
        );
        assert_eq!(source, fs::read(fixture.root().join("src/lib.rs")).unwrap());
        assert_only_lock_remains(temp.path());
    }
}
