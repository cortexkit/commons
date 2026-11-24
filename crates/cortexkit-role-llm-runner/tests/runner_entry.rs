use cortexkit_role_llm_runner::{
    display::{DisplayRow, RunnerEntry},
    read::ReadMessage,
};
use serde_json::json;

#[test]
fn runner_entry_round_trips_on_read_and_display() {
    let fact = RunnerEntry::new(RunnerEntry::HOST_STORED_NO_REPLY).unwrap();
    let body = json!({"native": "opaque"});
    let read = ReadMessage::new(1, "entry", body.clone())
        .with_role("assistant")
        .with_runner_entry(fact.clone());
    let row = DisplayRow::new("assistant", vec![]).with_runner_entry(fact);
    assert_eq!(read.check_runner_entry(), Ok(()));
    assert_eq!(row.check_runner_entry(), Ok(()));
    let read_wire = serde_json::to_value(&read).unwrap();
    let row_wire = serde_json::to_value(&row).unwrap();
    let expected = json!({"code": "host_stored_no_reply"});
    assert_eq!(read_wire["runner_entry"], expected);
    assert_eq!(row_wire["runner_entry"], expected);
    assert_eq!(read_wire["message"], body);
    assert_eq!(
        serde_json::from_value::<ReadMessage>(read_wire).unwrap(),
        read
    );
    assert_eq!(serde_json::from_value::<DisplayRow>(row_wire).unwrap(), row);
}

#[test]
fn runner_entry_accepts_unknown_codes_at_the_grammar_boundaries() {
    for code in ["0".to_owned(), "future_9".repeat(8)] {
        let wire = json!({"code": code});
        let fact: RunnerEntry = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(fact.code, code);
        assert_eq!(serde_json::to_value(fact).unwrap(), wire);
    }
}

#[test]
fn runner_entry_refuses_invalid_codes_on_both_surfaces() {
    for code in ["", "Upper", "a-b", "a b", "a\n", "é", &"a".repeat(65)] {
        let mut fact = RunnerEntry::new(RunnerEntry::HOST_STORED_NO_REPLY).unwrap();
        fact.code = code.into();
        let read = ReadMessage::new(1, "entry", json!({})).with_runner_entry(fact.clone());
        let row = DisplayRow::new("assistant", vec![]).with_runner_entry(fact);
        assert!(RunnerEntry::new(code).is_err(), "{code:?}");
        assert!(read
            .check_runner_entry()
            .unwrap_err()
            .contains("runner_entry.code"));
        assert!(row
            .check_runner_entry()
            .unwrap_err()
            .contains("runner_entry.code"));
        let read_error =
            serde_json::from_value::<ReadMessage>(serde_json::to_value(&read).unwrap())
                .unwrap_err();
        let row_error =
            serde_json::from_value::<DisplayRow>(serde_json::to_value(&row).unwrap()).unwrap_err();
        assert!(
            read_error.to_string().contains("runner_entry.code"),
            "{read_error}"
        );
        assert!(
            row_error.to_string().contains("runner_entry.code"),
            "{row_error}"
        );
    }
}

#[test]
fn runner_entry_preserves_read_and_display_bytes_when_absent() {
    let old_read =
        r#"{"ordinal":1,"mid":"entry","message":{"native":"opaque"},"role":"assistant"}"#;
    let old_row = r#"{"role":"assistant","text":"","parts":[]}"#;
    let read: ReadMessage = serde_json::from_str(old_read).unwrap();
    let row: DisplayRow = serde_json::from_str(old_row).unwrap();
    assert!(read.runner_entry.is_none());
    assert!(row.runner_entry.is_none());
    assert_eq!(read.check_runner_entry(), Ok(()));
    assert_eq!(row.check_runner_entry(), Ok(()));
    assert_eq!(serde_json::to_string(&read).unwrap(), old_read);
    assert_eq!(serde_json::to_string(&row).unwrap(), old_row);
}
