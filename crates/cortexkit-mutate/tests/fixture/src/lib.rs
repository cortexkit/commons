pub fn guarded(value: i32) -> bool {
    value > 0
}
pub fn vacuous() -> bool {
    true
}
pub fn unrelated() -> bool {
    true
}
pub fn wait_hook() {}
pub fn lock_hook() {}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs a daemon"]
    fn daemon_contract() {
        panic!("a daemon is not available in this fixture");
    }

    #[test]
    fn guard_rejects_zero() {
        assert!(!super::guarded(0));
    }
    #[test]
    fn guard_accepts_positive() {
        assert!(super::guarded(1));
    }
    #[test]
    fn vacuous_test() {
        let _ = super::vacuous();
    }
    #[test]
    fn unrelated_test() {
        assert!(super::unrelated());
    }
    #[test]
    fn waits() {
        super::wait_hook();
    }
    #[test]
    fn lock_integrity() {
        super::lock_hook();
    }
}
