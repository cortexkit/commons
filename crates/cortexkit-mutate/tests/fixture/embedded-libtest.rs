pub fn guarded(value: i32) -> bool {
    value > 0
}

#[cfg(test)]
mod tests {
    #[test]
    fn captures_child_run() {
        if std::env::var_os("CK_MUTATE_EMBEDDED_CHILD").is_some() {
            // These lines are captured by the child's harness and then embedded
            // again in the parent's assertion message, not harness outcomes.
            println!("test child_only ... ok\nfailures:\n---- nested stdout ----\ntest tests::shared_passes ... ok");
            assert!(!super::guarded(0), "child invariant");
            return;
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::captures_child_run", "--test-threads=1"])
            .env("CK_MUTATE_EMBEDDED_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "embedded child stdout:\n{}",
            String::from_utf8_lossy(&child.stdout)
        );
    }

    #[test]
    fn shared_passes() {
        assert!(super::guarded(1));
    }
}
