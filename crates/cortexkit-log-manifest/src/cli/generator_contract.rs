use std::fs;

use super::*;
use crate::{event, generate_with_id};

#[test]
fn collision_exits_one_and_preserves_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scan");
    let out = dir.path().join("doctor-manifest.json");
    let original = include_bytes!("../../tests/fixtures/scan/expected.json");
    fs::write(&out, original).unwrap();
    for check in [false, true] {
        let args = Args {
            check,
            crate_dir: crate_dir.clone(),
            out: Some(out.clone()),
            module_id: None,
            repository: None,
            log_globs: Vec::new(),
        };
        let error = execute(
            args,
            |path, metadata| generate_with_id(path, metadata, |_, _| "0123456789ab".into()),
            Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), 1);
        assert!(error
            .message()
            .contains("template id collision (0123456789ab)"));
        let message = error.message().replace('\\', "/");
        assert!(message.contains("source/disk/mod.rs:2"));
        assert!(message.contains("source/disk/next.rs:1"));
        assert_eq!(fs::read(&out).unwrap(), original);
    }
}

#[test]
fn only_constant_event_levels_are_recorded() {
    use event::Level;
    for (name, expected) in [
        ("TRACE", Level::Trace),
        ("DEBUG", Level::Debug),
        ("INFO", Level::Info),
        ("WARN", Level::Warn),
        ("ERROR", Level::Error),
    ] {
        let tokens = format!("tracing::Level::{name}, \"literal\"")
            .parse()
            .unwrap();
        assert_eq!(
            event::Macro::Event.parse(tokens, "test").unwrap().level,
            expected
        );
    }
    let tokens = "level, \"literal\"".parse().unwrap();
    assert!(event::Macro::Event.parse(tokens, "test").is_err());
}
