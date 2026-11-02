use cortexkit_exec_remote_types::*;
use serde_json::{json, Value};
use uuid::Uuid;

fn terminal() -> TerminalRecord {
    TerminalRecord::new(Uuid::from_u128(1), Outcome::Exit { code: 0 }, 1000, 0, 0)
        .with_ran(Ran::Remote)
}

#[test]
fn source_read_only_is_omitted_when_not_reported() {
    let record = terminal();
    assert_eq!(record.source_read_only(), None);
    let wire = serde_json::to_value(&record).unwrap();
    assert!(wire.get("source_read_only").is_none(), "{wire}");
}

#[test]
fn source_read_only_true_and_false_round_trip() {
    for protected in [true, false] {
        let record = terminal().with_source_read_only(protected);
        assert_eq!(record.source_read_only(), Some(protected));
        let wire = serde_json::to_value(&record).unwrap();
        assert_eq!(wire["source_read_only"], Value::Bool(protected));
        let back: TerminalRecord = serde_json::from_value(wire).unwrap();
        assert_eq!(back, record);
    }
}

#[test]
fn a_terminal_without_the_field_decodes_as_not_reported() {
    let mut wire = serde_json::to_value(terminal().with_source_read_only(true)).unwrap();
    wire.as_object_mut().unwrap().remove("source_read_only");
    let record: TerminalRecord = serde_json::from_value(wire).unwrap();
    assert_eq!(record.source_read_only(), None);
}

#[test]
fn terminal_ignores_unknown_members() {
    // The decoder ignores a member it doesn't know and keeps the decoded
    // values of the fields it does. This checks additive decoding, not a
    // decoder from an older release.
    let mut wire = serde_json::to_value(terminal()).unwrap();
    wire.as_object_mut()
        .unwrap()
        .insert("a_member_from_a_newer_runner".into(), json!(true));
    let record: TerminalRecord = serde_json::from_value(wire).unwrap();
    assert_eq!(record, terminal());
}
