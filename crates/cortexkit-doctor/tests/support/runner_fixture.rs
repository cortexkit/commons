// The test-only Cargo target is copied under different names so no
// fixture-only environment is needed.
#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    unix::main()
}

#[cfg(unix)]
mod unix {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::mem::MaybeUninit;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{self, ExitCode};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use cortexkit_doctor::module::{self, FixOutcome, Hooks, Outcome};
    use cortexkit_doctor::{
        CheckOutput, Finding, FindingId, Fix, LogSource, Manifest, ModuleId, Severity,
        MAX_OUTPUT_BYTES,
    };

    pub(super) fn main() -> ExitCode {
        let mut args = std::env::args_os();
        let executable = PathBuf::from(args.next().unwrap());
        let mode = executable.file_name().unwrap().to_str().unwrap();
        assert_eq!(args.next().unwrap(), "doctor");
        let args: Vec<_> = args.collect();
        let home = PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap());
        report_start(&home);
        match mode {
            "abort" => {
                let limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                // SAFETY: limit is initialized and lives for the syscall.
                assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) }, 0);
                process::abort();
            }
            "exit3" => return ExitCode::from(3),
            "exit3-text" => {
                print!("test-only text");
                return ExitCode::from(3);
            }
            "extra-output" => {
                write_check();
                print!("test-only text");
                return ExitCode::SUCCESS;
            }
            "from-file" => {
                assert_eq!(args, ["manifest"]);
                io::copy(
                    &mut File::open(home.join("manifest.json")).unwrap(),
                    &mut io::stdout(),
                )
                .unwrap();
                return ExitCode::SUCCESS;
            }
            "oversized" => {
                write_check();
                io::stdout().write_all(&[b' '; MAX_OUTPUT_BYTES]).unwrap();
                sleep();
            }
            "hold" | "late-hold" | "grace-large" | "hang" | "term-grandchild" => {
                assert_eq!(args, ["check", "--json"]);
                return with_grandchild(mode, &home);
            }
            "fix-hang" => sleep(),
            "probe" | "terminal" | "slow" => {}
            _ => panic!("unknown fixture mode"),
        }
        run_module(mode, &args)
    }

    fn finding() -> Finding {
        let id = FindingId::new("history.backlog").unwrap();
        let fix = Fix::safe(&id, false, vec!["Prune history".into()], 2_000).unwrap();
        Finding::new(
            id,
            Severity::Warning,
            "History needs attention.",
            vec![],
            fix,
        )
        .unwrap()
    }

    fn run_module(mode: &str, args: &[OsString]) -> ExitCode {
        let manifest = Manifest::new(
            ModuleId::new("fixture-module").unwrap(),
            "https://github.com/example/diagnostics",
            vec![LogSource::new("*.log").unwrap()],
            vec![],
        )
        .unwrap();
        let slow = mode == "slow" && args[0] == "fix";
        let terminal = mode == "terminal";
        let mut first = true;
        module::run_with(
            args,
            || {
                if slow && first {
                    thread::sleep(Duration::from_secs(9));
                }
                first = false;
                Ok(vec![finding()])
            },
            |_| Ok(()),
            |finding| {
                if slow {
                    let budget = finding.fix().plan().unwrap().timeout_ms();
                    thread::sleep(Duration::from_millis(budget - 1_000));
                }
                FixOutcome::fixed(vec![]).unwrap()
            },
            &String::from_utf8(manifest.to_canonical_bytes().unwrap()).unwrap(),
            Hooks {
                presence: Arc::new(move |_, _| {
                    if slow {
                        thread::sleep(Duration::from_secs(119));
                    }
                    if terminal {
                        Outcome::NoPresence
                    } else {
                        Outcome::Approved
                    }
                }),
                ..Hooks::default()
            },
        )
    }

    fn report_start(home: &Path) {
        let environment: BTreeMap<_, _> = std::env::vars().collect();
        fs::write(home.join("env"), serde_json::to_vec(&environment).unwrap()).unwrap();
        // SAFETY: none of the getter arguments or results are pointers.
        let (pid, pgid, sid) = unsafe { (libc::getpid(), libc::getpgrp(), libc::getsid(0)) };
        let tty = File::open("/dev/tty").is_ok();
        let mut mask = MaybeUninit::uninit();
        // SAFETY: no new mask is installed; mask has storage for the old one.
        let blocked = unsafe {
            assert_eq!(
                libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), mask.as_mut_ptr()),
                0
            );
            let mask = mask.assume_init();
            (1..=64).any(|signal| libc::sigismember(&mask, signal) == 1)
        };
        fs::write(
            home.join("start"),
            format!("{pid} {pgid} {sid} {tty} {blocked}"),
        )
        .unwrap();
        let null = fs::metadata("/dev/null").unwrap();
        for fd in [libc::STDIN_FILENO, libc::STDERR_FILENO] {
            let mut stat = MaybeUninit::uninit();
            // SAFETY: fstat fills stat on success, with sufficient writable space.
            let stat = unsafe {
                assert_eq!(libc::fstat(fd, stat.as_mut_ptr()), 0);
                stat.assume_init()
            };
            assert_eq!(i128::from(stat.st_rdev), i128::from(null.rdev()));
            assert_eq!(stat.st_mode & libc::S_IFMT, libc::S_IFCHR);
        }
        assert_eq!(io::stdin().read(&mut [0]).unwrap(), 0);
        eprintln!("test-only stderr");
    }

    fn write_check() {
        let out = CheckOutput::new(vec![finding()]).unwrap();
        println!("{}", serde_json::to_string(&out).unwrap());
        io::stdout().flush().unwrap();
    }

    fn with_grandchild(mode: &str, home: &Path) -> ExitCode {
        let (mut parent_death, parent_life) = UnixStream::pair().unwrap();
        // SAFETY: this fixture is single-threaded here. The child uses only
        // inherited descriptors and pauses until the runner signals the group.
        let child = unsafe { libc::fork() };
        assert_ne!(child, -1);
        if child == 0 {
            drop(parent_life);
            // SAFETY: installing SIG_IGN changes this grandchild only.
            unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
            if mode == "grace-large" {
                thread::sleep(Duration::from_millis(300));
                io::stdout()
                    .write_all(&[b' '; MAX_OUTPUT_BYTES + 1])
                    .unwrap();
            }
            if mode == "term-grandchild" {
                // This socket reaches EOF on direct-child death. The runner
                // should leave the grandchild alive to record it until SIGKILL.
                assert_eq!(parent_death.read(&mut [0]).unwrap(), 0);
                fs::write(home.join("direct-exited"), []).unwrap();
            }
            sleep();
        }
        fs::write(home.join("grandchild"), child.to_string()).unwrap();
        match mode {
            "hold" | "late-hold" => {
                write_check();
                if mode == "late-hold" {
                    thread::sleep(Duration::from_millis(750));
                }
                ExitCode::SUCCESS
            }
            "grace-large" => ExitCode::from(3),
            "hang" => {
                // SAFETY: SIG_IGN keeps the direct child alive until SIGKILL.
                unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
                sleep();
            }
            "term-grandchild" => {
                // SAFETY: the default disposition terminates the direct child.
                unsafe { libc::signal(libc::SIGTERM, libc::SIG_DFL) };
                sleep();
            }
            _ => unreachable!(),
        }
    }

    fn sleep() -> ! {
        // Never leak a fixture indefinitely if group cleanup regresses.
        thread::sleep(Duration::from_secs(15));
        process::exit(97);
    }
}
