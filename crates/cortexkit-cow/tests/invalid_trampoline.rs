#![forbid(unsafe_code)]

#[cfg(target_os = "macos")]
#[test]
fn invalid_trampoline_is_rejected_without_setting_configuration() {
    let root = tempfile::tempdir().unwrap();
    assert!(cortexkit_cow::configure_spawn_trampoline(root.path().join("missing")).is_err());
    // /usr/bin/true exits successfully but does not implement the protocol.
    assert!(cortexkit_cow::configure_spawn_trampoline("/usr/bin/true").is_err());
    cortexkit_cow::configure_spawn_trampoline(env!("CARGO_BIN_EXE_cortexkit-cow-spawn-fixture"))
        .unwrap();
}
