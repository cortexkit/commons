#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    unix::main()
}

#[cfg(unix)]
mod unix {
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;
    use std::sync::Arc;

    use cortexkit_doctor::module::{self, FixOutcome, FixRefusal, Hooks, Outcome, PreflightError};
    use cortexkit_doctor::{Finding, FindingId, Fix, LogSource, Manifest, ModuleId, Severity};
    use rustix::{fs::FlockOperation, io::Errno};

    pub(super) fn main() -> ExitCode {
        let mut args = std::env::args_os();
        let executable = PathBuf::from(args.next().unwrap());
        let mode = executable.file_name().unwrap().to_str().unwrap();
        assert_eq!(args.next().unwrap(), "doctor");
        if mode == "no-doctor" {
            return ExitCode::from(2);
        }
        let mut args: Vec<_> = args.collect();
        let data = PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap());
        let config = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap());
        let manifest = Manifest::new(
            ModuleId::new("fixture-module").unwrap(),
            "https://github.com/example/diagnostics",
            vec![LogSource::new("logs/*.log").unwrap()],
            vec![],
        )
        .unwrap();
        let canonical = String::from_utf8(manifest.to_canonical_bytes().unwrap()).unwrap();

        match (args[0].to_str().unwrap(), mode) {
            ("check", "invalid-check") => {
                println!("child-output-canary");
                return ExitCode::SUCCESS;
            }
            ("check", "write-data") => fs::write(data.join("nested/item"), "new").unwrap(),
            ("check", "write-config") => fs::write(config.join("settings"), "new").unwrap(),
            ("check", "retarget-symlink") => {
                let link = data.join("link");
                fs::remove_file(&link).unwrap();
                symlink("nested//item", link).unwrap();
            }
            ("check", "remove-empty-dir") => fs::remove_dir(data.join("empty")).unwrap(),
            ("manifest", "invalid-manifest") => {
                print!(
                    "{}",
                    canonical.replacen("\"dir\":\"data\"", "\"dir\":\"state\"", 1)
                );
                return ExitCode::SUCCESS;
            }
            ("manifest", "invalid-repository") => {
                print!("{}", canonical.replacen("github.com", "example.com", 1));
                return ExitCode::SUCCESS;
            }
            ("manifest", "stale-manifest") => {
                let stale = format!("sha256:{}", "0".repeat(64));
                print!(
                    "{}",
                    canonical.replacen(manifest.templates_digest(), &stale, 1)
                );
                return ExitCode::SUCCESS;
            }
            ("fix", _) => {
                if let Ok(mut spy) = OpenOptions::new()
                    .append(true)
                    .open(executable.with_file_name("observe-fixes"))
                {
                    writeln!(spy, "{}\t{}", data.display(), config.display()).unwrap();
                }
                if std::env::var("CORTEXKIT_DOCTOR_CONFORMANCE").as_deref() != Ok("1")
                    || std::env::var("CORTEXKIT_DOCTOR_TEST_PRESENCE").as_deref()
                        != Ok("no_presence")
                {
                    return ExitCode::from(2);
                }
                let fixable = finding("history.backlog", false, "Prune history");
                let current_digest = fixable.fix().plan().unwrap().digest();
                let scenario = if args[1] == "history.compact" {
                    "running"
                } else if args[3] != current_digest {
                    "stale"
                } else {
                    "no-terminal"
                };
                match (mode, scenario) {
                    ("ignores-stale", "stale") => args[3] = current_digest.into(),
                    ("applies-without-terminal", "no-terminal") => {
                        println!(r#"{{"schema":"ck.doctor-fix/v1","result":"fixed"}}"#);
                        return ExitCode::SUCCESS;
                    }
                    ("fails-with-no-terminal-code", "no-terminal") => {
                        println!(
                            r#"{{"schema":"ck.doctor-fix/v1","result":"failed","code":"no_terminal"}}"#
                        );
                        return ExitCode::SUCCESS;
                    }
                    ("mutates-before-crash", "no-terminal") => {
                        fs::write(config.join("settings"), "new").unwrap();
                        return ExitCode::from(3);
                    }
                    _ if mode.strip_prefix("mutates-before-") == Some(scenario) => {
                        fs::write(config.join("settings"), "new").unwrap();
                    }
                    _ => {}
                }
                let preflight = |finding: &Finding| {
                    if finding.fix().needs_stop() && module_running(&data) {
                        return Err(PreflightError::ModuleRunning);
                    }
                    Ok(())
                };
                let defer_preflight = mode == "confirmation-before-preflight";
                return module::run_with(
                    &args,
                    || Ok(findings(&data, mode)),
                    |finding| {
                        if defer_preflight {
                            Ok(())
                        } else {
                            preflight(finding)
                        }
                    },
                    |finding| {
                        if defer_preflight && preflight(&finding).is_err() {
                            return FixOutcome::refused(FixRefusal::ModuleRunning);
                        }
                        panic!("the fixture must never apply a fix");
                    },
                    &canonical,
                    Hooks {
                        presence: Arc::new(|_, _| Outcome::NoPresence),
                        ..Hooks::default()
                    },
                );
            }
            _ => {}
        }
        module::run(
            &args,
            || Ok(findings(&data, mode)),
            |_| Ok(()),
            |_| panic!("the fixture must never apply a fix"),
            &canonical,
        )
    }

    fn module_running(data: &Path) -> bool {
        let pid_marker = fs::read_to_string(data.join("nested/doctor.pid"));
        pid_marker
            .ok()
            .and_then(|text| text.strip_suffix('\n')?.parse::<u32>().ok())
            .is_some_and(|pid| pid > 0)
            || File::open(data.join("nested/doctor.lock")).is_ok_and(|file| {
                rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive)
                    == Err(Errno::WOULDBLOCK)
            })
    }

    fn findings(data: &Path, mode: &str) -> Vec<Finding> {
        let mut findings = vec![
            finding("history.backlog", false, "Prune history"),
            informational("history.info"),
        ];
        let running =
            matches!(mode, "store-in-use" | "different-plan-while-running") && module_running(data);
        if mode == "store-in-use" && running {
            findings.push(informational("store.in_use"));
        } else {
            let step = if mode == "different-plan-while-running" && running {
                "Rewrite history"
            } else {
                "Compact history"
            };
            findings.push(finding("history.compact", true, step));
        }
        findings
    }

    fn informational(id: &str) -> Finding {
        Finding::new(
            FindingId::new(id).unwrap(),
            Severity::Info,
            "No repair available.",
            vec![],
            Fix::none(false),
        )
        .unwrap()
    }

    fn finding(id: &str, needs_stop: bool, step: &str) -> Finding {
        let id = FindingId::new(id).unwrap();
        let fix = Fix::safe(&id, needs_stop, vec![step.into()], 1_000).unwrap();
        Finding::new(
            id,
            Severity::Warning,
            "History needs attention.",
            vec![],
            fix,
        )
        .unwrap()
    }
}
