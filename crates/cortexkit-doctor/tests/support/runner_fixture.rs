// The test-only Cargo target is invoked under different names so no
// fixture-only environment is needed.
#[cfg(not(any(unix, windows)))]
fn main() {}

#[cfg(any(unix, windows))]
fn main() -> std::process::ExitCode {
    fixture::main()
}

#[cfg(any(unix, windows))]
mod fixture {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    #[cfg(unix)]
    use std::mem::MaybeUninit;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    #[cfg(unix)]
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{self, ExitCode};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use cortexkit_doctor::module::{self, FixOutcome, Hooks, Outcome};
    use cortexkit_doctor::{
        CheckOutput, Finding, FindingId, Fix, LogSource, Manifest, Mode, ModuleId, Severity,
        MAX_OUTPUT_BYTES,
    };

    pub(super) fn main() -> ExitCode {
        let mut args = std::env::args_os();
        let executable = PathBuf::from(args.next().unwrap());
        let mode = executable.file_stem().unwrap().to_str().unwrap();
        let command = args.next().unwrap();
        #[cfg(windows)]
        if command == "descendant" {
            if mode == "grace-large" {
                thread::sleep(Duration::from_millis(300));
                io::stdout()
                    .write_all(&[b' '; MAX_OUTPUT_BYTES + 1])
                    .unwrap();
            }
            sleep();
        }
        assert_eq!(command, "doctor");
        let args: Vec<_> = args.collect();
        let home = PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap());
        report_start(&home);
        match mode {
            "abort" => abort(),
            "exit2" => return ExitCode::from(2),
            "exit2-text" => {
                print!("test-only text");
                return ExitCode::from(2);
            }
            "exit3" => return ExitCode::from(3),
            "exit3-text" => {
                print!("test-only text");
                return ExitCode::from(3);
            }
            "exit101" => return ExitCode::from(101),
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
                assert!(args == ["check", "--json"] || args == ["check", "--full", "--json"]);
                return with_grandchild(mode, &home, &executable);
            }
            "fix-hang" => sleep(),
            #[cfg(windows)]
            "fix-break" => return with_grandchild(mode, &home, &executable),
            #[cfg(windows)]
            "terminal-partial" => {}
            "quick-only" if args == ["check", "--full", "--json"] => return ExitCode::from(2),
            "probe" | "terminal" | "slow" | "quick-only" | "full-only" => {}
            _ => panic!("unknown fixture mode"),
        }
        run_module(mode, &args)
    }

    #[cfg(unix)]
    fn abort() -> ! {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: limit is initialized and lives for the syscall.
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) }, 0);
        process::abort()
    }

    #[cfg(windows)]
    fn abort() -> ! {
        // This deliberately returns the same DWORD as an access violation.
        process::exit(0xc000_0005u32 as i32)
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
        let terminal = matches!(mode, "terminal" | "terminal-partial");
        let mut timing = module::Timing::default();
        if mode == "terminal-partial" {
            timing.terminal_limit = Duration::from_millis(200);
        }
        let mut first = true;
        module::run_with(
            args,
            |check_mode| {
                if mode == "full-only" {
                    if check_mode == Mode::Quick {
                        return Ok(vec![]);
                    }
                    if args[0] == "check" {
                        thread::sleep(Duration::from_millis(300));
                    } else if first {
                        // Spend more time rechecking than the planned runtime
                        // so the runner must allow time for the recheck itself.
                        let budget = finding().fix().plan().unwrap().timeout_ms();
                        thread::sleep(Duration::from_millis(budget + 200));
                    }
                }
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
                timing,
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
        report_stdio(home);
        assert_eq!(io::stdin().read(&mut [0]).unwrap(), 0);
        eprintln!("test-only stderr");
    }

    #[cfg(unix)]
    fn report_stdio(home: &Path) {
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
    }

    #[cfg(windows)]
    fn report_stdio(home: &Path) {
        use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_CHAR};
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        };

        let console = File::open(r"\\.\CONIN$").is_ok();
        fs::write(home.join("start"), format!("{} {console}", process::id())).unwrap();
        for which in [STD_INPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: the fixture has not replaced or closed its standard handles.
            assert_eq!(unsafe { GetFileType(GetStdHandle(which)) }, FILE_TYPE_CHAR);
        }
    }

    fn write_check() {
        let out = CheckOutput::new(vec![finding()]).unwrap();
        println!("{}", serde_json::to_string(&out).unwrap());
        io::stdout().flush().unwrap();
    }

    #[cfg(unix)]
    fn with_grandchild(mode: &str, home: &Path, _executable: &Path) -> ExitCode {
        let (mut parent_death, parent_life) = UnixStream::pair().unwrap();
        if mode == "term-grandchild" {
            // SAFETY: set SIG_IGN before fork so TERM cannot kill the grandchild
            // before it starts observing the direct child's death.
            unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
        }
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
                fs::write(home.join("ready"), []).unwrap();
                sleep();
            }
            "term-grandchild" => {
                // SAFETY: the default disposition terminates the direct child.
                unsafe { libc::signal(libc::SIGTERM, libc::SIG_DFL) };
                fs::write(home.join("ready"), []).unwrap();
                sleep();
            }
            _ => unreachable!(),
        }
    }

    #[cfg(windows)]
    #[allow(
        clippy::zombie_processes,
        reason = "the descendant must outlive its parent so the runner has to tear it down"
    )]
    fn with_grandchild(mode: &str, home: &Path, executable: &Path) -> ExitCode {
        use std::os::windows::process::CommandExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Instant;
        use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_BREAK_EVENT};
        use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

        static INTERRUPTED: AtomicBool = AtomicBool::new(false);
        unsafe extern "system" fn on_control(event: u32) -> i32 {
            if event == CTRL_BREAK_EVENT {
                INTERRUPTED.store(true, Ordering::Relaxed);
                1
            } else {
                0
            }
        }
        if mode == "fix-break" {
            // SAFETY: the handler is a static function that only touches an atomic.
            assert_ne!(unsafe { SetConsoleCtrlHandler(Some(on_control), 1) }, 0);
        }
        // A separate console group still belongs to the runner's job.
        let mut child = process::Command::new(executable)
            .arg("descendant")
            .creation_flags(CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .unwrap();
        record_job_membership(home, &child);
        fs::write(home.join("grandchild"), child.id().to_string()).unwrap();
        match mode {
            "hold" | "late-hold" => {
                write_check();
                if mode == "late-hold" {
                    thread::sleep(Duration::from_millis(750));
                }
                ExitCode::SUCCESS
            }
            "grace-large" => ExitCode::from(3),
            "hang" | "term-grandchild" | "fix-break" => {
                fs::write(home.join("ready"), []).unwrap();
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(15) {
                    if INTERRUPTED.load(Ordering::Relaxed) {
                        fs::write(home.join("direct-exited"), []).unwrap();
                        return ExitCode::from(3);
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                let _ = child.wait();
                ExitCode::from(97)
            }
            _ => unreachable!(),
        }
    }

    #[cfg(windows)]
    fn record_job_membership(home: &Path, child: &process::Child) {
        use std::mem::size_of_val;
        use std::os::windows::io::AsRawHandle;
        use std::ptr::null_mut;

        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::JobObjects::{
            IsProcessInJob, JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        };
        use windows_sys::Win32::System::Threading::GetProcessTimes;

        // JobObjectBasicProcessIdList has a variable-length tail of process IDs.
        #[repr(C)]
        #[derive(Default)]
        struct MemberList {
            assigned: u32,
            listed: u32,
            pids: [usize; 16],
        }
        let mut members = MemberList::default();
        // SAFETY: NULL queries this process's immediate job. The aligned buffer
        // provides the native header and room for up to 16 member PIDs.
        let members_ok = unsafe {
            QueryInformationJobObject(
                null_mut(),
                JobObjectBasicProcessIdList,
                (&mut members as *mut MemberList).cast(),
                size_of_val(&members) as u32,
                null_mut(),
            )
        };
        let members_error = (members_ok == 0).then(|| io::Error::last_os_error().raw_os_error());
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // SAFETY: NULL selects the same job and limits has the requested layout.
        let limits_ok = unsafe {
            QueryInformationJobObject(
                null_mut(),
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
                null_mut(),
            )
        };
        let limits_error = (limits_ok == 0).then(|| io::Error::last_os_error().raw_os_error());
        let mut in_job = 0;
        // SAFETY: child still owns the spawned process handle; NULL means any job.
        let membership_ok =
            unsafe { IsProcessInJob(child.as_raw_handle(), null_mut(), &mut in_job) };
        let membership_error =
            (membership_ok == 0).then(|| io::Error::last_os_error().raw_os_error());
        let [mut created, mut exited, mut kernel, mut user] = [FILETIME::default(); 4];
        // SAFETY: child owns the live process handle and all four times are writable.
        let times_ok = unsafe {
            GetProcessTimes(
                child.as_raw_handle(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        let times_error = (times_ok == 0).then(|| io::Error::last_os_error().raw_os_error());
        let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let pids = &members.pids[..(members.listed as usize).min(members.pids.len())];
        fs::write(
            home.join("grandchild-job"),
            format!(
                "leader={} grandchild={} creation_time={created} times_error={times_error:?} \
             immediate_job_flags={:#x} limits_error={limits_error:?} \
             immediate_job_assigned={} immediate_job_pids={pids:?} members_error={members_error:?} \
             grandchild_in_any_job={} membership_error={membership_error:?}",
                process::id(),
                child.id(),
                limits.BasicLimitInformation.LimitFlags,
                members.assigned,
                in_job != 0,
            ),
        )
        .unwrap();
    }

    fn sleep() -> ! {
        // Never leak a fixture indefinitely if group cleanup regresses.
        thread::sleep(Duration::from_secs(15));
        process::exit(97);
    }
}
