use cortexkit_role_llm_runner::{
    display::{DisplayRow, ModelFallback},
    read::ReadMessage,
};
use serde_json::json;

#[test]
fn model_fallback_read_and_display_round_trip_and_absence() {
    let fallback = ModelFallback::new("provider/requested", "provider/answered").unwrap();
    let body = json!({"native":"opaque"});
    let read = ReadMessage::new(1, "reply", body.clone())
        .with_role("assistant")
        .with_model_fallback(fallback.clone());
    let row = DisplayRow::new("assistant", vec![]).with_model_fallback(fallback);
    read.check_model_fallback("assistant").unwrap();
    row.check_model_fallback().unwrap();
    let fact = json!({"requested":"provider/requested", "answered":"provider/answered"});
    let raw_read = serde_json::to_value(&read).unwrap();
    let raw_row = serde_json::to_value(&row).unwrap();
    assert_eq!(raw_read["model_fallback"], fact);
    assert_eq!(raw_row["model_fallback"], fact);
    assert_eq!(raw_read["message"], body);
    assert_eq!(
        serde_json::from_value::<ReadMessage>(raw_read).unwrap(),
        read
    );
    assert_eq!(serde_json::from_value::<DisplayRow>(raw_row).unwrap(), row);
    for value in [
        serde_json::to_value(ReadMessage::new(1, "reply", body)).unwrap(),
        serde_json::to_value(DisplayRow::new("assistant", vec![])).unwrap(),
    ] {
        assert!(value.get("model_fallback").is_none());
    }
}

#[test]
fn model_fallback_refuses_other_roles_and_mutated_values() {
    let fallback = ModelFallback::new("requested", "answered").unwrap();
    for role in [
        "user",
        "system",
        "tool",
        "synthetic",
        "developer",
        "future_role",
    ] {
        let read = ReadMessage::new(1, "m", json!({}))
            .with_role(role)
            .with_model_fallback(fallback.clone());
        assert!(read.check_model_fallback(role).is_err(), "{role}");
        assert!(read.check_model_fallback("assistant").is_err(), "{role}");
        assert!(
            ReadMessage::new(1, "m", json!({}))
                .with_role("assistant")
                .with_model_fallback(fallback.clone())
                .check_model_fallback(role)
                .is_err(),
            "{role}"
        );
        assert!(
            DisplayRow::new(role, vec![])
                .with_model_fallback(fallback.clone())
                .check_model_fallback()
                .is_err(),
            "{role}"
        );
        ReadMessage::new(1, "m", json!({}))
            .check_model_fallback(role)
            .unwrap();
        DisplayRow::new(role, vec![])
            .check_model_fallback()
            .unwrap();
    }
    for (requested, answered) in [("same", "same"), ("", "answered"), ("requested", "")] {
        assert!(ModelFallback::new(requested, answered).is_err());
        let mut invalid = fallback.clone();
        invalid.requested = requested.into();
        invalid.answered = answered.into();
        assert!(DisplayRow::new("assistant", vec![])
            .with_model_fallback(invalid.clone())
            .check_model_fallback()
            .is_err());
        assert!(ReadMessage::new(1, "m", json!({}))
            .with_role("assistant")
            .with_model_fallback(invalid)
            .check_model_fallback("assistant")
            .is_err());
    }
}

#[test]
fn model_fallback_decode_requires_two_distinct_non_empty_ids() {
    for invalid in [
        json!({}),
        json!({"requested":"a"}),
        json!({"answered":"b"}),
        json!({"requested":"same", "answered":"same"}),
        json!({"requested":"", "answered":"b"}),
        json!({"requested":"a", "answered":""}),
        json!({"requested":null, "answered":"b"}),
        json!({"requested":"a", "answered":1}),
    ] {
        assert!(
            serde_json::from_value::<ModelFallback>(invalid.clone()).is_err(),
            "{invalid}"
        );
        let mut read =
            serde_json::to_value(ReadMessage::new(1, "m", json!({})).with_role("assistant"))
                .unwrap();
        let mut row = serde_json::to_value(DisplayRow::new("assistant", vec![])).unwrap();
        for value in [&mut read, &mut row] {
            value["model_fallback"] = invalid.clone();
        }
        assert!(
            serde_json::from_value::<ReadMessage>(read).is_err(),
            "{invalid}"
        );
        assert!(
            serde_json::from_value::<DisplayRow>(row).is_err(),
            "{invalid}"
        );
    }
    let open: ModelFallback =
        serde_json::from_value(json!({"requested":"a", "answered":"b", "future":true})).unwrap();
    assert_eq!(
        serde_json::to_value(open).unwrap(),
        json!({"requested":"a", "answered":"b"})
    );
}
