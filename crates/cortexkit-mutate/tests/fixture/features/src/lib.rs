pub fn guarded(value: i32) -> bool {
    value > 0
}

#[cfg(feature = "test-support")]
pub mod test_support {
    pub use super::guarded;
}
