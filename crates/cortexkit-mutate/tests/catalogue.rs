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
        f.git(&["init", "-q"]);
        f.git(&["config", "user.email", "fixture@example.invalid"]);
        f.git(&["config", "user.name", "Catalogue fixture"]);
        fs::write(f.root().join("guard.txt"), "guard").unwrap();
        f
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.root().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn commit(&self) -> String {
        self.git(&["add", "."]);
        self.git(&["commit", "-qm", "fixture"]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn load(&self, path: &str) -> Result<Catalogue> {
        load(&self.root().join(path))
    }

    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_ckdev-mutate"))
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap()
    }

    #[cfg(unix)]
    fn cli_with_tool_probe(&self, args: &[&str]) -> (std::process::Output, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let probe_dir = self.root().join(".git/tool-probe");
        fs::create_dir_all(&probe_dir).unwrap();
        let marker = self.root().join(".git/tool-spawned");
        let marker_quoted = marker.display().to_string().replace('\'', "'\\''");
        for tool in ["cargo", "rustc"] {
            let binary = probe_dir.join(tool);
            fs::write(
                &binary,
                format!("#!/bin/sh\nprintf '%s\\n' \"$0\" >> '{marker_quoted}'\nexit 91\n"),
            )
            .unwrap();
            let mut permissions = fs::metadata(&binary).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&binary, permissions).unwrap();
        }
        let mut search_path = vec![probe_dir];
        search_path.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let output = Command::new(env!("CARGO_BIN_EXE_ckdev-mutate"))
            .current_dir(self.root())
            .args(args)
            .env("PATH", std::env::join_paths(search_path).unwrap())
            .output()
            .unwrap();
        (output, marker)
    }
}

fn row(id: &str) -> String {
    format!(
        r#"[[control]]
id = "{id}"
guards = "the guard rejects invalid input"
file = "guard.txt"
old = "guard"
new = "changed"
test_file = "guard.txt"
runner = "cargo"
package = "unused"
expect_red = []
unreachable = "the edited function has no production caller"
"#
    )
}

fn error(result: Result<Catalogue>) -> String {
    result.err().expect("catalogue must be refused")
}

#[test]
fn directory_loads_all_rows_in_filename_byte_order() {
    let f = Fixture::new();
    f.write("mutations.d/z.toml", &row("last"));
    f.write(
        "mutations.d/a.toml",
        &format!("{}{}", row("second"), row("third")),
    );
    f.write("mutations.d/B.toml", &row("first"));
    let loaded = f.load("mutations.toml").unwrap();
    assert_eq!(
        loaded
            .control
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "third", "last"]
    );
}

#[test]
fn directory_settings_are_owned_by_one_fragment_even_when_equal() {
    let f = Fixture::new();
    f.write("mutations.d/00-settings.toml", "prebuild = []\n");
    f.write(
        "mutations.d/extra.toml",
        &format!("prebuild = []\n{}", row("one")),
    );
    let message = error(f.load("mutations.toml"));
    assert!(message.contains("prebuild"), "{message}");
    assert!(
        message.contains("mutations.d/00-settings.toml"),
        "{message}"
    );
    assert!(message.contains("mutations.d/extra.toml"), "{message}");
}

#[test]
fn duplicate_ids_across_fragments_name_every_file_and_line() {
    let f = Fixture::new();
    f.write("mutations.d/a.toml", &row("same"));
    f.write("mutations.d/b.toml", &format!("\n{}", row("same")));
    f.write("mutations.d/c.toml", &format!("\n\n{}", row("same")));
    let message = error(f.load("mutations.toml"));
    for location in [
        "mutations.d/a.toml:2",
        "mutations.d/b.toml:3",
        "mutations.d/c.toml:4",
    ] {
        assert!(message.contains(location), "{message}");
    }
    assert!(message.contains("duplicate id \"same\""), "{message}");
}

#[test]
fn single_file_duplicate_ids_name_every_line_and_all_commands_refuse() {
    let f = Fixture::new();
    let text = row("same");
    let second_line = text.lines().count() + 2;
    f.write("mutations.toml", &format!("{text}{text}"));
    let message = error(f.load("mutations.toml"));
    assert!(message.contains("mutations.toml:2"), "{message}");
    assert!(
        message.contains(&format!("mutations.toml:{second_line}")),
        "{message}"
    );
    for args in [
        vec!["check"],
        vec!["run", "--all"],
        vec![
            "explore",
            "--package",
            "unused",
            "--file",
            "guard.txt",
            "--old",
            "guard",
            "--new",
            "changed",
        ],
    ] {
        let out = f.cli(&args);
        assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("duplicate id"), "{args:?}: {stderr}");
    }
}

#[test]
fn both_forms_are_refused_by_name() {
    let f = Fixture::new();
    f.write("mutations.toml", &row("single"));
    f.write("mutations.d/a.toml", &row("directory"));
    for path in ["mutations.toml", "mutations.d"] {
        let message = error(f.load(path));
        assert!(
            message.contains("mutations.toml") && message.contains("mutations.d"),
            "{message}"
        );
        let out = f.cli(&["--catalogue", path, "check"]);
        assert!(!out.status.success());
    }
}

#[test]
fn explicit_catalogue_directory_works_from_a_repository_subdirectory() {
    let f = Fixture::new();
    f.write("proofs/one.toml", &row("one"));
    fs::create_dir(f.root().join("subdir")).unwrap();
    f.commit();
    let out = Command::new(env!("CARGO_BIN_EXE_ckdev-mutate"))
        .current_dir(f.root().join("subdir"))
        .args(["--catalogue", "proofs", "check"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        f.load("proofs").unwrap().control[0].source.file,
        "proofs/one.toml"
    );
}

#[test]
fn directory_ignores_hidden_non_toml_and_nested_files() {
    let f = Fixture::new();
    f.write("mutations.d/one.toml", &row("one"));
    for path in [
        "mutations.d/.hidden.toml",
        "mutations.d/notes.txt",
        "mutations.d/upper.TOML",
        "mutations.d/nested/two.toml",
    ] {
        f.write(path, "not valid TOML");
    }
    fs::create_dir(f.root().join("mutations.d/folder.toml")).unwrap();
    let catalogue = f.load("mutations.toml").unwrap();
    assert_eq!(catalogue.control.len(), 1);
    assert_eq!(catalogue.control[0].id, "one");
}

#[test]
fn reports_include_each_source_file_in_json_and_text_in_both_forms() {
    let f = Fixture::new();
    f.write("mutations.d/a.toml", &row("first"));
    f.write("mutations.d/b.toml", &row("second"));
    f.commit();
    for (args, expected) in [
        (
            vec!["run", "--all", "--report", "out.json"],
            vec!["mutations.d/a.toml", "mutations.d/b.toml"],
        ),
        (
            vec![
                "--catalogue",
                "single.toml",
                "run",
                "--all",
                "--report",
                "out.json",
            ],
            vec!["single.toml"],
        ),
    ] {
        f.write("single.toml", &row("single"));
        let out = f.cli(&args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let reports: serde_json::Value =
            serde_json::from_slice(&fs::read(f.root().join("out.json")).unwrap()).unwrap();
        for (index, path) in expected.iter().enumerate() {
            assert_eq!(reports[index]["source"]["file"], *path);
            assert_eq!(reports[index]["source"]["line"], 2);
            assert!(stdout.contains(path), "{stdout}");
        }
    }
}

#[test]
fn directory_settings_need_no_reserved_filename_and_rows_keep_their_defaults() {
    let f = Fixture::new();
    f.write("mutations.d/z-settings.toml", "prebuild = []\n");
    f.write("mutations.d/one.toml", &row("one"));
    let c = f.load("mutations.toml").unwrap().control.remove(0);
    assert_eq!(c.timeout_s, 600);
    assert_eq!(c.build_timeout_s, 1800);
    assert_eq!(c.platforms, None);
}

#[test]
fn row_errors_and_coverage_errors_include_source_file() {
    let f = Fixture::new();
    f.write(
        "mutations.d/one.toml",
        &row("one").replace("runner = \"cargo\"", "runner = \"unknown\""),
    );
    let message = error(f.load("mutations.toml"));
    assert!(
        message.contains("one (mutations.d/one.toml:2)"),
        "{message}"
    );
    f.write(
        "mutations.d/one.toml",
        &row("one").replace("old = \"guard\"", "old = \"missing\""),
    );
    let c = f.load("mutations.toml").unwrap();
    let message = check(f.root(), &c, &AtomicBool::new(false)).unwrap_err();
    assert!(message.contains("mutations.d/one.toml"), "{message}");
}

#[test]
fn id_line_numbers_come_from_toml_not_text_search() {
    let f = Fixture::new();
    let text = format!("# id = \"same\"\n{}", row("same"));
    f.write("mutations.toml", &format!("{text}{text}"));
    let message = error(f.load("mutations.toml"));
    assert!(message.contains("mutations.toml:3"), "{message}");
    assert!(
        message.contains(&format!("mutations.toml:{}", text.lines().count() + 3)),
        "{message}"
    );
}

#[test]
fn append_creates_an_id_named_fragment_and_refuses_duplicate_ids() {
    let f = Fixture::new();
    f.write("mutations.d/settings.toml", "prebuild = []\n");
    let c: Control = toml::from_str::<Catalogue>(&row("new-row"))
        .unwrap()
        .control
        .remove(0);
    append_control(&f.root().join("mutations.toml"), &c).unwrap();
    assert!(f.root().join("mutations.d/new-row.toml").is_file());
    assert_eq!(f.load("mutations.toml").unwrap().control[0].id, "new-row");
    let message = append_control(&f.root().join("mutations.toml"), &c).unwrap_err();
    assert!(message.contains("duplicate id"), "{message}");
}

#[test]
fn diff_reads_directory_rows_and_selects_only_changed_definitions() {
    let f = Fixture::new();
    f.write("mutations.d/a.toml", &row("first"));
    f.write("mutations.d/b.toml", &row("second"));
    let base = f.commit();
    f.write(
        "mutations.d/b.toml",
        &row("second").replace("the guard rejects", "a guard rejects"),
    );
    f.write("mutations.d/c.toml", &row("third"));
    f.commit();
    let catalogue = f.load("mutations.toml").unwrap();
    assert_eq!(
        select_diff(f.root(), "mutations.d", &catalogue, &base).unwrap(),
        ["second".to_owned(), "third".to_owned()]
            .into_iter()
            .collect()
    );
}

#[test]
fn diff_accepts_manual_migration_without_selecting_unchanged_rows() {
    let f = Fixture::new();
    f.write("mutations.toml", &row("one"));
    let base = f.commit();
    fs::remove_file(f.root().join("mutations.toml")).unwrap();
    f.write("mutations.d/one.toml", &row("one"));
    f.commit();
    let catalogue = f.load("mutations.toml").unwrap();
    assert!(select_diff(f.root(), "mutations.d", &catalogue, &base)
        .unwrap()
        .is_empty());
}

#[test]
fn dirty_fragment_mutation_targets_are_still_refused() {
    let f = Fixture::new();
    f.write("mutations.d/one.toml", &row("one"));
    f.commit();
    let mut c = f.load("mutations.toml").unwrap().control.remove(0);
    c.file = Some("mutations.d/one.toml".into());
    c.old = Some("rejects".into());
    c.new = Some("accepts".into());
    c.unreachable = None;
    c.runner = "command".into();
    c.package = None;
    c.command = Some(vec!["false".into(), "{test}".into()]);
    c.test_count_pattern = Some("Ran {count} test".into());
    c.expect_red = vec!["test".into()];
    f.write(
        "mutations.d/one.toml",
        &format!("{}\n# local edit\n", row("one")),
    );
    let before = fs::read(f.root().join("mutations.d/one.toml")).unwrap();
    let message = run_row(f.root(), &c, false, &AtomicBool::new(false), false).unwrap_err();
    assert!(message.contains("dirty target refused"), "{message}");
    assert_eq!(
        before,
        fs::read(f.root().join("mutations.d/one.toml")).unwrap()
    );
}

#[test]
fn single_file_retains_row_order_defaults_and_prebuild() {
    let f = Fixture::new();
    f.write(
        "mutations.toml",
        &format!("prebuild = []\n{}{}", row("second"), row("first")),
    );
    let catalogue = f.load("mutations.toml").unwrap();
    assert!(catalogue.prebuild.is_empty());
    assert_eq!(
        catalogue
            .control
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
    assert_eq!(catalogue.control[0].source.file, "mutations.toml");
    assert_eq!(catalogue.control[0].timeout_s, 600);
}

#[test]
fn prove_appends_to_the_default_directory_and_reports_the_destination() {
    let f = Fixture::new();
    f.write("mutations.d/00-settings.toml", "prebuild = []\n");
    f.commit();
    let out = f.cli(&[
        "prove",
        "--id",
        "proof",
        "--guards",
        "guard contract",
        "--file",
        "guard.txt",
        "--old",
        "guard",
        "--new",
        "changed",
        "--test-file",
        "guard.txt",
        "--package",
        "unused",
        "--expect-red",
        "test",
        "--desk-only",
        "requires a desktop",
        "--report",
        "out.json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let catalogue = f.load("mutations.toml").unwrap();
    assert_eq!(catalogue.control[0].source.file, "mutations.d/proof.toml");
    assert!(!f.root().join("mutations.toml").exists());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root().join("out.json")).unwrap()).unwrap();
    assert_eq!(report[0]["source"]["file"], "mutations.d/proof.toml");
}

fn anchored_row(id: &str, file: &str, old: &str) -> String {
    format!(
        r#"[[control]]
id = "{id}"
guards = "the guard rejects invalid input"
file = "{file}"
old = "{old}"
new = "changed"
test_file = "guard.txt"
runner = "cargo"
package = "unused"
expect_red = []
unreachable = "the edited function has no production caller"
"#
    )
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn check_reports_every_broken_row_in_file_order() {
    let f = Fixture::new();
    f.write("target.txt", "twice twice once\n");
    let first = anchored_row("matches-twice", "target.txt", "twice");
    let clean = anchored_row("matches-once", "target.txt", "once");
    let clean_line = first.lines().count() + 2;
    let last_line = clean_line + clean.lines().count();
    f.write(
        "mutations.toml",
        &format!(
            "{first}{clean}{}",
            anchored_row("matches-nothing", "target.txt", "absent")
        ),
    );
    let out = f.cli(&["check"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        format!(
            "ckdev-mutate: matches-twice (mutations.toml:2): ANCHOR_MISSING target.txt: replaced 2 occurrences\n\
             ckdev-mutate: matches-nothing (mutations.toml:{last_line}): ANCHOR_MISSING target.txt: replaced 0 occurrences\n\
             ckdev-mutate check: 2 rows failed\n"
        )
    );
    // The library check refuses the same catalogue and names both rows.
    let message = check(
        f.root(),
        &f.load("mutations.toml").unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(message.lines().count(), 2, "{message}");
    assert!(message.contains("matches-twice"), "{message}");
    assert!(message.contains("matches-nothing"), "{message}");
}

#[test]
fn check_reports_a_single_broken_row_with_its_usual_line() {
    let f = Fixture::new();
    f.write(
        "mutations.toml",
        &format!(
            "{}{}",
            anchored_row("missing-file", "deleted.txt", "guard"),
            row("clean")
        ),
    );
    let out = f.cli(&["check"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let text = stderr(&out);
    let mut lines = text.lines();
    let first = lines.next().unwrap();
    assert!(
        first.starts_with(
            "ckdev-mutate: missing-file (mutations.toml:2): ANCHOR_MISSING deleted.txt: "
        ),
        "{text}"
    );
    assert_eq!(
        lines.next(),
        Some("ckdev-mutate check: 1 row failed"),
        "{text}"
    );
    assert_eq!(lines.next(), None, "{text}");
}

#[test]
fn check_passes_a_clean_catalogue() {
    let f = Fixture::new();
    f.write("mutations.toml", &format!("{}{}", row("one"), row("two")));
    let out = f.cli(&["check"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "");
    assert_eq!(
        out.stdout,
        b"one: test names not verified: unreachable row\ntwo: test names not verified: unreachable row\nCatalogue anchors checked; verified 0 rows; 2 rows not verified: one, two\n"
    );
}

#[cfg(unix)]
fn active_cargo_row(id: &str, old: &str) -> String {
    format!(
        r#"[[control]]
id = "{id}"
guards = "the guard rejects invalid input"
file = "guard.txt"
old = "{old}"
new = "changed"
test_file = "guard.txt"
runner = "cargo"
package = "unused"
expect_red = ["tests::guard_rejects_invalid_input"]
"#
    )
}

#[test]
fn check_help_documents_fast_and_platform_modes() {
    let f = Fixture::new();
    let out = f.cli(&["check", "--help"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("--anchors-only"), "{help}");
    assert!(help.contains("--platform <PLATFORM>"), "{help}");
    assert!(help.contains("in CI"), "{help}");
}

#[cfg(unix)]
#[test]
fn check_anchors_only_does_not_start_cargo() {
    let f = Fixture::new();
    f.write("mutations.toml", &active_cargo_row("active-row", "guard"));
    let (out, tool_marker) = f.cli_with_tool_probe(&["check", "--anchors-only"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!tool_marker.exists(), "Cargo or rustc was started");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Test names were not checked (--anchors-only); catalogue anchors checked for 1 rows.\n"
    );
    assert_eq!(stderr(&out), "");
}

#[cfg(unix)]
#[test]
fn check_anchors_only_catches_a_broken_anchor_without_cargo() {
    let f = Fixture::new();
    f.write("mutations.toml", &active_cargo_row("broken-row", "absent"));
    let (out, tool_marker) = f.cli_with_tool_probe(&["check", "--anchors-only"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(!tool_marker.exists(), "Cargo or rustc was started");
    let error = stderr(&out);
    assert!(error.contains("broken-row"), "{error}");
    assert!(
        error.contains("ANCHOR_MISSING guard.txt: replaced 0 occurrences"),
        "{error}"
    );
}
