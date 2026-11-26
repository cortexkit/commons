#![cfg(all(unix, feature = "test-fixture"))]

use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use cortexkit_doctor::runner::{self, Options};
use cortexkit_doctor::{FindingId, FixResult, Mode};
use cortexkit_doctor_conformance::{run, Adapter, Liveness, Outcome, RuleResult};

#[path = "kit_core/fix_rules.rs"]
mod kit_fix;

struct Fixture {
    _program_dir: tempfile::TempDir,
    program: PathBuf,
    seeded: Vec<(PathBuf, PathBuf)>,
    fifo: bool,
    fixable: Option<FindingId>,
    full_fixable: Option<FindingId>,
    needs_stop: Option<FindingId>,
    liveness: Option<Liveness>,
    non_prompting: bool,
}

impl Fixture {
    fn new(mode: &str) -> Self {
        let program_dir = tempfile::tempdir().unwrap();
        let program = program_dir.path().join(mode);
        let binary = env!("CARGO_BIN_EXE_cortexkit-doctor-conformance-fixture");
        // A sibling fork can inherit a copy's write descriptor, so exec fails
        // with "Text file busy". A symlink keeps the mode in argv[0] without a copy.
        symlink(binary, &program).unwrap();
        Self {
            _program_dir: program_dir,
            program,
            seeded: Vec::new(),
            fifo: false,
            fixable: None,
            full_fixable: None,
            needs_stop: None,
            liveness: None,
            non_prompting: false,
        }
    }
}

impl Adapter for Fixture {
    fn program(&self) -> &Path {
        &self.program
    }

    fn seed(&mut self, data: &Path, config: &Path) -> io::Result<()> {
        self.seeded.push((data.to_owned(), config.to_owned()));
        fs::create_dir(data.join("nested"))?;
        fs::create_dir(data.join("empty"))?;
        fs::write(data.join("nested/item"), "old")?;
        fs::write(config.join("settings"), "old")?;
        symlink("nested/item", data.join("link"))?;
        symlink("missing-target", config.join("dangling"))?;
        if self.fifo {
            let fifo = CString::new(config.join("fifo").as_os_str().as_bytes()).unwrap();
            // SAFETY: fifo is NUL-terminated and remains alive for the call.
            if unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    fn fixable_id(&self) -> Option<&FindingId> {
        self.fixable.as_ref()
    }

    fn full_fixable_id(&self) -> Option<&FindingId> {
        self.full_fixable.as_ref()
    }

    fn needs_stop_id(&self) -> Option<&FindingId> {
        self.needs_stop.as_ref()
    }

    fn liveness(&self) -> Option<&Liveness> {
        self.liveness.as_ref()
    }

    fn non_prompting(&self) -> bool {
        self.non_prompting
    }
}

fn outcome(results: &[RuleResult], name: &str) -> Outcome {
    results
        .iter()
        .find(|rule| rule.name == name)
        .unwrap()
        .outcome
}

fn failures(results: &[RuleResult]) -> Vec<(&str, Outcome)> {
    results
        .iter()
        .filter(|rule| matches!(rule.outcome, Outcome::Failed { .. }))
        .map(|rule| (rule.name, rule.outcome))
        .collect()
}

#[test]
fn kit_core_passes_a_correct_program_with_independent_homes() {
    let mut fixture = Fixture::new("correct");
    let runs = run(&mut fixture);
    for rule in [
        "output.valid",
        "check.read_only",
        "check.full",
        "manifest.valid",
        "manifest.digest",
    ] {
        assert_eq!(outcome(&runs, rule), Outcome::Passed, "{rule}");
    }
    assert_eq!(fixture.seeded.len(), 4);
    assert_eq!(fixture.seeded.iter().collect::<HashSet<_>>().len(), 4);
    for (data, config) in fixture.seeded {
        assert!(data.is_absolute() && config.is_absolute());
        assert!(!data.exists() && !config.exists());
    }
}

#[test]
fn kit_core_rejects_invalid_check_without_output_text() {
    let runs = run(&mut Fixture::new("invalid-check"));
    assert_eq!(
        failures(&runs),
        [("output.valid", Outcome::Failed { reason: "not_json" })]
    );
    assert!(!format!("{runs:?}").contains("child-output-canary"));
}

#[test]
fn kit_core_rejects_invalid_full_output_or_writes_during_full_check() {
    for (mode, reason) in [
        ("invalid-full-check", "not_json"),
        ("write-config-in-full-check", "home_changed"),
    ] {
        let runs = run(&mut Fixture::new(mode));
        assert_eq!(
            failures(&runs),
            [("check.full", Outcome::Failed { reason })]
        );
        assert!(!format!("{runs:?}").contains("child-output-canary"));
    }
}

#[test]
fn kit_core_reports_missing_doctor_for_check_and_manifest() {
    let runs = run(&mut Fixture::new("no-doctor"));
    assert_eq!(
        failures(&runs),
        [
            (
                "output.valid",
                Outcome::Failed {
                    reason: "no_doctor"
                }
            ),
            (
                "check.full",
                Outcome::Failed {
                    reason: "no_doctor"
                }
            ),
            (
                "manifest.valid",
                Outcome::Failed {
                    reason: "no_doctor"
                }
            ),
        ]
    );
}

#[test]
fn kit_core_rejects_check_changes_to_either_home() {
    for mode in [
        "write-data",
        "write-config",
        "retarget-symlink",
        "remove-empty-dir",
    ] {
        let runs = run(&mut Fixture::new(mode));
        assert_eq!(
            failures(&runs),
            [(
                "check.read_only",
                Outcome::Failed {
                    reason: "home_changed"
                }
            )],
            "{mode}"
        );
    }
}

#[test]
fn kit_core_rejects_fifo_in_a_snapshot() {
    let mut fixture = Fixture::new("correct");
    fixture.fifo = true;
    let runs = run(&mut fixture);
    assert_eq!(
        failures(&runs),
        ["check.read_only", "check.full"].map(|name| (
            name,
            Outcome::Failed {
                reason: "unsupported_entry"
            }
        ))
    );
}

#[test]
fn kit_core_reports_manifest_shape_and_digest_independently() {
    let runs = run(&mut Fixture::new("invalid-manifest"));
    assert_eq!(
        failures(&runs),
        [("manifest.valid", Outcome::Failed { reason: "shape" })]
    );
    assert_eq!(
        outcome(&runs, "manifest.digest"),
        Outcome::Skipped {
            reason: "prerequisite_failed"
        }
    );

    let runs = run(&mut Fixture::new("stale-manifest"));
    assert_eq!(outcome(&runs, "manifest.valid"), Outcome::Passed);
    assert_eq!(
        failures(&runs),
        [(
            "manifest.digest",
            Outcome::Failed {
                reason: "templates_digest"
            }
        )]
    );
}

#[test]
fn kit_core_reports_invalid_manifest_repository() {
    let runs = run(&mut Fixture::new("invalid-repository"));
    assert_eq!(
        failures(&runs),
        [(
            "manifest.valid",
            Outcome::Failed {
                reason: "repository"
            }
        )]
    );
    assert_eq!(
        outcome(&runs, "manifest.digest"),
        Outcome::Skipped {
            reason: "prerequisite_failed"
        }
    );
}

#[test]
fn kit_core_fixture_injects_presence_only_under_both_switches() {
    let fixture = Fixture::new("correct");
    let scratch = tempfile::tempdir().unwrap();
    let options = Options::new()
        .with_data_home(scratch.path())
        .with_config_home(scratch.path());
    let check = runner::check(fixture.program(), Mode::Quick, &options).unwrap();
    let finding = &check.findings()[0];
    let plan = finding.fix().plan().unwrap();
    let env = [
        ("CORTEXKIT_DOCTOR_CONFORMANCE", "1"),
        ("CORTEXKIT_DOCTOR_TEST_PRESENCE", "no_presence"),
    ];
    for one_switch in env {
        let output = Command::new(fixture.program())
            .args([
                "doctor",
                "fix",
                finding.id().as_str(),
                "--digest",
                plan.digest(),
            ])
            .env_clear()
            .env("XDG_DATA_HOME", scratch.path())
            .env("XDG_CONFIG_HOME", scratch.path())
            .env(one_switch.0, one_switch.1)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
    let fix = runner::fix(
        fixture.program(),
        finding.id(),
        plan.digest(),
        plan.timeout_ms(),
        Mode::Quick,
        &options.with_conformance_fix_marker(),
    )
    .unwrap();
    assert_eq!(fix.result(), FixResult::Refused);
    assert_eq!(fix.code(), Some("no_terminal"));
}
