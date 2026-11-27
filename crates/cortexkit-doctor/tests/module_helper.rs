#![cfg(any(unix, windows))]

use std::ffi::{OsStr, OsString};
use std::fs::OpenOptions;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::process::{self, Command, ExitCode};
use std::sync::Arc;
use std::time::Instant;

use cortexkit_doctor::module::{self, CheckError, FixOutcome, Hooks, Outcome, Timing};
use cortexkit_doctor::{
    CheckOutput, Finding, FindingId, Fix, FixOutput, FixResult, LogSource, Manifest, Mode,
    ModuleId, Severity,
};

const CHILD: &str = "CK_DOCTOR_MODULE_HELPER_CHILD";
const ID: &str = "history.backlog";

fn example_finding() -> Finding {
    let id = FindingId::new(ID).unwrap();
    let fix = Fix::safe(&id, true, vec!["Prune history".into()], 1_000).unwrap();
    Finding::new(
        id,
        Severity::Warning,
        "History needs attention.",
        vec![],
        fix,
    )
    .unwrap()
}

fn example_manifest() -> String {
    let manifest = Manifest::new(
        ModuleId::new("integration-module").unwrap(),
        "https://github.com/example/diagnostics",
        vec![LogSource::new("*.log").unwrap()],
        vec![],
    )
    .unwrap();
    String::from_utf8(manifest.to_canonical_bytes().unwrap()).unwrap()
}

#[test]
fn module_helper_fixture_child() {
    let Some(path) = std::env::var_os(CHILD) else {
        return;
    };
    let mode = std::env::var("CK_DOCTOR_MODULE_HELPER_MODE").unwrap();
    let decline = mode == "decline";
    let argc = std::env::var("CK_DOCTOR_MODULE_HELPER_ARGC")
        .unwrap()
        .parse()
        .unwrap();
    let args = (0..argc)
        .map(|i| std::env::var_os(format!("CK_DOCTOR_MODULE_HELPER_ARG_{i}")).unwrap())
        .collect::<Vec<_>>();

    // The harness has already written its banner. Capture only the command's
    // stdout, including anything that escapes from a callback or a late flush.
    let output = OpenOptions::new().write(true).open(path).unwrap();
    capture_stdout(output);

    let mut manifest = example_manifest();
    match mode.as_str() {
        "manifest-invalid" => manifest = "not json".into(),
        "manifest-noncanonical" => manifest.insert(0, ' '),
        "manifest-no-lf" => {
            manifest.pop();
        }
        _ => {}
    }
    let code = module::run_with(
        &args,
        |check_mode| match mode.as_str() {
            "full-only" if check_mode == Mode::Quick => Ok(vec![]),
            "println" => {
                println!("noisy callback");
                Ok(vec![example_finding()])
            }
            "print" => {
                print!("x");
                Ok(vec![example_finding()])
            }
            "check-error" => {
                print!("x");
                Err(CheckError)
            }
            "check-duplicates" => Ok(vec![example_finding(); 2]),
            "check-panic" => {
                println!("before panic");
                panic!("fixture callback panic")
            }
            "manifest" => panic!("manifest must not run check"),
            _ => Ok(vec![example_finding()]),
        },
        |_| Ok(()),
        |_| FixOutcome::fixed(vec![]).unwrap(),
        &manifest,
        Hooks {
            now: &Instant::now,
            presence: Arc::new(move |text, _| {
                assert_eq!(
                    text,
                    "CortexKit integration-module: History needs attention. Fix: Prune history"
                );
                if decline {
                    Outcome::Declined
                } else {
                    Outcome::Approved
                }
            }),
            open_terminal: &|| panic!("fixture must not open a terminal"),
            timing: Timing::default(),
        },
    );
    io::stdout().flush().unwrap();
    process::exit(if code == ExitCode::SUCCESS {
        0
    } else if code == ExitCode::FAILURE {
        1
    } else {
        assert_eq!(code, ExitCode::from(2));
        2
    });
}

#[cfg(unix)]
fn capture_stdout(output: std::fs::File) {
    // SAFETY: output is open and outlives dup2, which duplicates its descriptor.
    assert_eq!(
        unsafe { libc::dup2(output.as_raw_fd(), libc::STDOUT_FILENO) },
        1
    );
}

#[cfg(windows)]
fn capture_stdout(output: std::fs::File) {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{SetStdHandle, STD_OUTPUT_HANDLE};

    let output = output.into_raw_handle();
    // SAFETY: output stays open until this fixture process exits.
    assert_ne!(unsafe { SetStdHandle(STD_OUTPUT_HANDLE, output) }, 0);
}

fn invoke(mode: &str, args: &[impl AsRef<OsStr>], exit: i32) -> Vec<u8> {
    let captured = tempfile::NamedTempFile::new().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "module_helper_fixture_child", "--nocapture"])
        .env(CHILD, captured.path())
        .env("CK_DOCTOR_MODULE_HELPER_MODE", mode)
        .env("CK_DOCTOR_MODULE_HELPER_ARGC", args.len().to_string());
    for (i, arg) in args.iter().enumerate() {
        command.env(format!("CK_DOCTOR_MODULE_HELPER_ARG_{i}"), arg.as_ref());
    }
    let child = command.output().unwrap();
    assert_eq!(
        child.status.code(),
        Some(exit),
        "{mode}: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    std::fs::read(captured.path()).unwrap()
}

#[test]
fn module_helper_process_check_owns_stdout_after_callback_and_return() {
    let expected = CheckOutput::new(vec![example_finding()]).unwrap();
    let mut expected_line = serde_json::to_vec(&expected).unwrap();
    expected_line.push(b'\n');
    for noise in ["println", "print"] {
        let bytes = invoke(noise, &["check", "--json"], 0);
        assert_eq!(bytes, expected_line, "{noise}");
        assert_eq!(CheckOutput::decode(&bytes).unwrap(), expected, "{noise}");
    }
    for bad in ["check-error", "check-duplicates", "check-panic"] {
        assert!(invoke(bad, &["check", "--json"], 1).is_empty(), "{bad}");
    }
}

#[test]
fn module_helper_process_usage_errors_do_not_write() {
    let finding = example_finding();
    let digest = finding.fix().plan().unwrap().digest();
    for args in [
        &[][..],
        &["check"],
        &["check", "--json", "extra"],
        &["check", "--full"],
        &["check", "--full", "--json", "extra"],
        &["check", "--full", "--full", "--json"],
        &["check", "--other"],
        &["fix", ID],
        &["fix", ID, "--digest", "sha256:bad"],
        &["fix", "Invalid!", "--digest", digest],
        &["fix", "sk-secret", "--digest", digest],
        &[
            "fix",
            "abcdef0123456789abcdef0123456789abcdef01",
            "--digest",
            digest,
        ],
        &["fix", ID, "--other", digest],
        &["fix", ID, "--digest", digest, "extra"],
        &["fix", ID, "--digest", digest, "--full", "--full"],
        &["manifest", "extra"],
        &["unknown"],
    ] {
        assert!(invoke("usage", args, 2).is_empty(), "{args:?}");
    }
    #[cfg(unix)]
    let invalid = OsString::from_vec(vec![0xff]);
    #[cfg(windows)]
    let invalid = OsString::from_wide(&[0xd800]);
    let non_utf8 = ["fix".into(), invalid, "--digest".into(), digest.into()];
    assert!(invoke("usage", &non_utf8, 2).is_empty());
}

#[test]
fn module_helper_process_manifest_requires_and_echoes_canonical_bytes() {
    assert_eq!(
        invoke("manifest", &["manifest"], 0),
        example_manifest().as_bytes()
    );
    for bad in [
        "manifest-invalid",
        "manifest-noncanonical",
        "manifest-no-lf",
    ] {
        for args in [&["manifest"][..], &["check", "--json"]] {
            assert!(invoke(bad, args, 1).is_empty(), "{bad}: {args:?}");
        }
    }
}

#[test]
fn module_helper_process_fix_exit_zero_for_every_document() {
    let finding = example_finding();
    let args = [
        "fix",
        ID,
        "--digest",
        finding.fix().plan().unwrap().digest(),
    ];
    for (mode, result, code) in [
        ("approve", FixResult::Fixed, None),
        ("decline", FixResult::Refused, Some("not_confirmed")),
        ("check-error", FixResult::Failed, Some("check_failed")),
    ] {
        let bytes = invoke(mode, &args, 0);
        let output = FixOutput::decode(&bytes).unwrap();
        assert_eq!(
            (output.result(), output.code(), output.evidence()),
            (result, code, None)
        );
        let mut expected = serde_json::to_vec(&output).unwrap();
        expected.push(b'\n');
        assert_eq!(bytes, expected);
    }
}

#[test]
fn module_helper_full_check_passes_full_mode_to_callback() {
    let check = |args: &[&str]| CheckOutput::decode(&invoke("full-only", args, 0)).unwrap();
    assert!(check(&["check", "--json"]).findings().is_empty());
    assert_eq!(
        check(&["check", "--full", "--json"]).findings(),
        &[example_finding()]
    );
}

#[test]
fn module_helper_full_fix_uses_full_mode_for_both_rechecks() {
    let finding = example_finding();
    let args = [
        "fix",
        ID,
        "--digest",
        finding.fix().plan().unwrap().digest(),
        "--full",
    ];
    let fix = |args: &[&str]| FixOutput::decode(&invoke("full-only", args, 0)).unwrap();
    assert_eq!(fix(&args[..4]).code(), Some("unknown_finding"));
    assert_eq!(fix(&args).result(), FixResult::Fixed);
}
