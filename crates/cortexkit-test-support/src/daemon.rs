use crate::{fence::fenced_env_with_roots, fenced_command};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::{Duration, Instant},
};

/// Command builder whose only spawn operation returns an owned `TestDaemon`.
/// It enforces scratch-root isolation and keeps the child tied to a lifetime pipe.
/// Unix consumers require `python3` on PATH to run the process-lifetime watcher.
pub struct TestDaemonCommand {
    command: tokio::process::Command,
    scratch: PathBuf,
    program: PathBuf,
    extra_roots: Vec<(String, String)>,
}

impl TestDaemonCommand {
    pub fn new(program: &Path, scratch: &Path) -> Self {
        #[cfg(unix)]
        let mut command = {
            let watcher =
                super::stable_executable("ckdev-daemon-watch", include_bytes!("daemon_watch.py"));
            let mut command = fenced_command("python3", scratch);
            command.arg(watcher).arg(program);
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            tokio::process::Command::from(command)
        };
        #[cfg(not(unix))]
        let mut command = tokio::process::Command::from(fenced_command(program, scratch));
        command.stdin(Stdio::piped()).kill_on_drop(true);
        Self {
            command,
            scratch: scratch.to_owned(),
            program: program.to_owned(),
            extra_roots: Vec::new(),
        }
    }

    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.command.env(key, value);
        self
    }
    /// Redirect an additional directory variable into a plain subdirectory of
    /// scratch. It is reapplied at spawn, so `.env` and `.env_remove` cannot
    /// bypass it. Panics for a variable the fence already sets (HOME, the XDG
    /// directories, TMPDIR) and for a directory name that is not a single plain
    /// path component.
    pub fn fenced_root(&mut self, variable: &str, directory: &str) -> &mut Self {
        fenced_env_with_roots(&self.scratch, &[(variable, directory)]);
        if let Some((_, current)) = self
            .extra_roots
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(variable))
        {
            *current = directory.to_owned();
        } else {
            self.extra_roots
                .push((variable.to_owned(), directory.to_owned()));
        }
        self
    }
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.command.arg(arg);
        self
    }
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.command.env_remove(key);
        self
    }
    pub fn stdout(&mut self, stdio: impl Into<Stdio>) -> &mut Self {
        self.command.stdout(stdio);
        self
    }
    pub fn stderr(&mut self, stdio: impl Into<Stdio>) -> &mut Self {
        self.command.stderr(stdio);
        self
    }
    pub fn kill_on_drop(&mut self, enabled: bool) -> &mut Self {
        assert!(enabled, "test daemons must die with their harness");
        self
    }
    pub fn spawn(&mut self) -> std::io::Result<TestDaemon> {
        if !self
            .program
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("ckdev-"))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "test daemon program must be named ckdev-*",
            ));
        }
        // Set HOME, the XDG directories, TMPDIR and caller-selected roots last,
        // just before spawn. These are the variables the daemon reads to decide
        // where it writes, so a later value always wins over anything a caller
        // set through `.env`, and the daemon writes only under its scratch tree.
        let roots: Vec<_> = self
            .extra_roots
            .iter()
            .map(|(name, directory)| (name.as_str(), directory.as_str()))
            .collect();
        self.command
            .envs(fenced_env_with_roots(&self.scratch, &roots));
        let result = self.command.spawn();
        #[cfg(unix)]
        let result = result.map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("spawn test daemon watcher python3 (requires python3 on PATH): {error}"),
            )
        });
        let child = result?;
        Ok(TestDaemon { child })
    }
}

/// Owns the parent pipe and process group. Drop is bounded and synchronous, so
/// when a test panics, the daemon is torn down before the test's scratch
/// directory is deleted.
pub struct TestDaemon {
    child: tokio::process::Child,
}

impl TestDaemon {
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
    pub fn start_kill(&mut self) -> std::io::Result<()> {
        self.child.stdin.take();
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // On Unix the spawned child is the watcher script, which leads its own
            // process group, so this TERM reaches it. It reacts by killing its
            // daemon and every descendant, including supervised children that
            // ignore TERM themselves.
            let group = rustix::process::Pid::from_raw(pid as i32).expect("child PID");
            let _ = rustix::process::kill_process(group, rustix::process::Signal::TERM);
        }
        #[cfg(not(unix))]
        self.child.start_kill()?;
        Ok(())
    }
    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        // Tokio closes a child's stdin when waiting. The watcher's stdin is a
        // lifetime pipe, not input, so keep it owned until the watcher exits.
        let pipe = self.child.stdin.take();
        let result = self.child.wait().await;
        drop(pipe);
        result
    }
    pub async fn kill(&mut self) -> std::io::Result<()> {
        self.start_kill()?;
        self.wait().await.map(|_| ())
    }
    pub fn stop(&mut self) {
        let _ = self.start_kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::park_timeout(Duration::from_millis(10));
        }
        if self.child.id().is_some() {
            #[cfg(unix)]
            if let Some(group) = rustix::process::Pid::from_raw(self.child.id().unwrap() as i32) {
                let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            }
            let _ = self.child.start_kill();
        }
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_python3_error_names_the_watcher_requirement() {
        let scratch = crate::ScratchDir::new("missing-python");
        let empty_path = scratch.join("empty-path");
        std::fs::create_dir(&empty_path).unwrap();
        let program = crate::stable_executable("ckdev-python-requirement", b"#!/bin/sh\nexit 0\n");
        let result = TestDaemonCommand::new(&program, &scratch)
            .env("PATH", &empty_path)
            .spawn();
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("watcher spawned without python3 on PATH"),
        };
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        let text = error.to_string();
        assert!(text.contains("python3") && text.contains("PATH"), "{text}");
    }

    #[tokio::test]
    async fn spawned_child_sees_enforced_caller_selected_roots() {
        let scratch = crate::ScratchDir::new("daemon-extra-roots");
        let output = scratch.join("extra-env");
        let program = crate::stable_executable(
            "ckdev-extra-root-probe",
            b"#!/bin/sh\nprintf '%s\\n' \"$EXAMPLE_STORAGE_ROOT\" \"$EXAMPLE_WORKTREE_ROOT\" > \"$1\"\n",
        );
        let mut command = TestDaemonCommand::new(&program, &scratch);
        command
            .fenced_root("EXAMPLE_STORAGE_ROOT", "storage")
            .fenced_root("EXAMPLE_WORKTREE_ROOT", "worktrees")
            .arg(&output)
            .env("EXAMPLE_STORAGE_ROOT", "/outside/storage")
            .env_remove("EXAMPLE_WORKTREE_ROOT");
        let mut daemon = command.spawn().unwrap();
        assert!(daemon.wait().await.unwrap().success());
        let expected = ["storage", "worktrees"]
            .map(|dir| scratch.join(dir).display().to_string())
            .join("\n")
            + "\n";
        assert_eq!(std::fs::read_to_string(output).unwrap(), expected);
        assert!(scratch.join("storage").is_dir() && scratch.join("worktrees").is_dir());
    }

    #[tokio::test]
    async fn non_ckdev_program_is_refused() {
        let scratch = crate::ScratchDir::new("daemon-refusal");
        let program = crate::stable_executable("not-a-dev-daemon", b"#!/bin/sh\nexit 0\n");
        let result = TestDaemonCommand::new(&program, &scratch).spawn();
        assert!(
            matches!(result, Err(ref error) if error.kind()==std::io::ErrorKind::InvalidInput),
            "non-ckdev daemon was not refused"
        );
    }

    #[tokio::test]
    async fn spawned_child_sees_enforced_xdg_roots() {
        let scratch = crate::ScratchDir::new("daemon-xdg");
        let output = scratch.join("xdg-env");
        let program = crate::stable_executable("ckdev-xdg-probe", b"#!/bin/sh\nprintf '%s\\n' \"$XDG_DATA_HOME\" \"$XDG_RUNTIME_DIR\" \"$XDG_CONFIG_HOME\" > \"$1\"\n");
        let mut command = TestDaemonCommand::new(&program, &scratch);
        command
            .arg(&output)
            .env("XDG_DATA_HOME", "/outside/data")
            .env_remove("XDG_RUNTIME_DIR")
            .env("XDG_CONFIG_HOME", "/outside/config");
        let mut daemon = command.spawn().unwrap();
        assert!(daemon.wait().await.unwrap().success());
        let expected = ["data", "runtime", "config"]
            .map(|d| scratch.join(d).display().to_string())
            .join("\n")
            + "\n";
        assert_eq!(std::fs::read_to_string(output).unwrap(), expected);
    }

    #[tokio::test]
    #[should_panic(expected = "test daemons must die with their harness")]
    async fn kill_on_drop_false_is_refused() {
        let scratch = crate::ScratchDir::new("daemon-kill-guard");
        TestDaemonCommand::new(Path::new("ckdev-fixture"), &scratch).kill_on_drop(false);
    }

    fn alive(pid: i32) -> bool {
        subc_os::process_identity::liveness(pid as u32, None)
            == subc_os::process_identity::Liveness::Alive
    }

    async fn fixture() -> (super::super::ScratchDir, TestDaemon, i32) {
        let scratch = super::super::ScratchDir::new("daemon-lifetime");
        let pid_file = scratch.join("daemon.pid");
        let program = super::super::stable_executable(
            "ckdev-lifetime-fixture",
            b"#!/bin/sh\ntrap '' TERM\nexec sleep 600\n",
        );
        let daemon = TestDaemonCommand::new(&program, scratch.path())
            .env("CORTEXKIT_TEST_DAEMON_PID_FILE", &pid_file)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !pid_file.exists() {
            assert!(Instant::now() < deadline, "fixture daemon did not start");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        assert!(alive(pid), "positive control: daemon is running");
        (scratch, daemon, pid)
    }

    async fn assert_gone(pid: i32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!alive(pid), "dropped harness leaked daemon pid {pid}");
    }

    #[tokio::test]
    async fn dropping_harness_mid_run_reaps_its_daemon() {
        let (_scratch, daemon, pid) = fixture().await;
        drop(daemon);
        assert_gone(pid).await;
    }

    #[tokio::test]
    async fn panic_unwinding_reaps_its_daemon() {
        let (_scratch, daemon, pid) = fixture().await;
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _daemon = daemon;
            panic!("intentional harness assertion");
        }));
        assert!(panic.is_err());
        assert_gone(pid).await;
    }

    #[tokio::test]
    async fn parent_pipe_eof_reaps_its_daemon_without_drop() {
        let (_scratch, mut daemon, pid) = fixture().await;
        daemon.child.stdin.take();
        tokio::time::timeout(Duration::from_secs(5), daemon.wait())
            .await
            .unwrap()
            .unwrap();
        assert_gone(pid).await;
    }

    #[test]
    fn fleet_binary_is_copied_to_ckdev_scratch_before_exec() {
        let scratch = super::super::ScratchDir::new("fleet-name");
        let source = scratch.join("ck-name-probe");
        std::fs::write(&source, b"#!/bin/sh\nprintf '%s\\n' \"$1\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        let staged = crate::stage_test_binary(&source);
        assert_eq!(staged.file_name().unwrap(), "ckdev-name-probe");
        assert!(staged.starts_with("/tmp"));
        assert_eq!(
            std::fs::read(source).unwrap(),
            std::fs::read(&staged).unwrap()
        );
        let output = super::super::fenced_command(&staged, scratch.path())
            .arg("live")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"live\n");
    }
}
