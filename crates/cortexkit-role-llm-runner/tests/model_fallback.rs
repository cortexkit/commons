use cortexkit_role_llm_runner::{
    display::{DisplayRow, ModelFallback},
    read::ReadMessage,
    step_events::Fallback,
};
use serde_json::json;

#[test]
fn model_fallback_preserves_old_fact_bytes() {
    let old = r#"{"requested":"provider/requested","answered":"provider/answered"}"#;
    let decoded: ModelFallback = serde_json::from_str(old).unwrap();
    let built = ModelFallback::new("provider/requested", "provider/answered").unwrap();
    assert_eq!(decoded, built);
    assert_eq!(
        (decoded.by, decoded.category, decoded.cause),
        (None, None, None)
    );
    assert_eq!(serde_json::to_string(&built).unwrap(), old);
}

#[test]
fn model_fallback_reason_decodes_null_as_absent_and_omits_it_on_write() {
    let mut wire = json!({
        "requested": "requested", "answered": "answered", "by": null, "category": null, "cause": null,
    });
    let fallback: ModelFallback = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(fallback).unwrap(),
        json!({"requested": "requested", "answered": "answered"})
    );
    wire["by"] = json!("client");
    assert!(serde_json::from_value::<ModelFallback>(wire)
        .unwrap_err()
        .to_string()
        .contains("requires by and cause"));
}

#[test]
fn model_fallback_read_and_display_round_trip_the_step_reason() {
    let step = Fallback::new("provider", "requested", "sensitive_1-a", "round");
    let fact = ModelFallback::new("provider/requested", "provider/answered")
        .unwrap()
        .with_reason(&step);
    let read = ReadMessage::new(1, "reply", json!({"native":"opaque"}))
        .with_role("assistant")
        .with_model_fallback(fact.clone());
    let row = DisplayRow::new("assistant", vec![]).with_model_fallback(fact.clone());
    assert_eq!(read.check_model_fallback("assistant"), Ok(()));
    assert_eq!(row.check_model_fallback(), Ok(()));
    let expected = json!({
        "requested": "provider/requested", "answered": "provider/answered",
        "by": "provider", "category": "sensitive_1-a", "cause": "round"
    });
    let raw_read = serde_json::to_value(&read).unwrap();
    let raw_row = serde_json::to_value(&row).unwrap();
    assert_eq!(raw_read["model_fallback"], expected);
    assert_eq!(raw_row["model_fallback"], expected);
    assert_eq!(
        serde_json::from_value::<ReadMessage>(raw_read).unwrap(),
        read
    );
    assert_eq!(serde_json::from_value::<DisplayRow>(raw_row).unwrap(), row);

    let mut with_future = expected;
    with_future["future"] = json!(true);
    assert_eq!(
        serde_json::from_value::<ModelFallback>(with_future).unwrap(),
        fact
    );
}

#[test]
fn model_fallback_read_and_display_project_a_step_without_category() {
    let step = Fallback::new("client", "requested", "", "refused_here");
    let fact = ModelFallback::new("requested", "answered")
        .unwrap()
        .with_reason(&step);
    assert_eq!(fact.category, None);
    let read = ReadMessage::new(1, "reply", json!({"native":"opaque"}))
        .with_role("assistant")
        .with_model_fallback(fact.clone());
    let row = DisplayRow::new("assistant", vec![]).with_model_fallback(fact);
    assert_eq!(read.check_model_fallback("assistant"), Ok(()));
    assert_eq!(row.check_model_fallback(), Ok(()));
    let expected = json!({
        "requested": "requested", "answered": "answered", "by": "client", "cause": "refused_here"
    });
    let raw_read = serde_json::to_value(&read).unwrap();
    let raw_row = serde_json::to_value(&row).unwrap();
    assert_eq!(raw_read["model_fallback"], expected);
    assert_eq!(raw_row["model_fallback"], expected);
    assert_eq!(
        serde_json::from_value::<ReadMessage>(raw_read).unwrap(),
        read
    );
    assert_eq!(serde_json::from_value::<DisplayRow>(raw_row).unwrap(), row);
}

#[test]
fn model_fallback_read_and_display_refuse_partial_or_invalid_reasons() {
    let step = Fallback::new("client", "requested", "z".repeat(64), "refused_here");
    let valid = ModelFallback::new("requested", "answered")
        .unwrap()
        .with_reason(&step);
    assert_eq!(valid.check("assistant"), Ok(()));
    assert_eq!(
        serde_json::from_value::<ModelFallback>(serde_json::to_value(&valid).unwrap()).unwrap(),
        valid
    );
    let assert_refused = |fact: ModelFallback, field| {
        let read = ReadMessage::new(1, "reply", json!({}))
            .with_role("assistant")
            .with_model_fallback(fact.clone());
        let row = DisplayRow::new("assistant", vec![]).with_model_fallback(fact);
        assert!(read
            .check_model_fallback("assistant")
            .unwrap_err()
            .contains(field));
        assert!(row.check_model_fallback().unwrap_err().contains(field));
        let read_error =
            serde_json::from_value::<ReadMessage>(serde_json::to_value(&read).unwrap())
                .unwrap_err();
        let row_error =
            serde_json::from_value::<DisplayRow>(serde_json::to_value(&row).unwrap()).unwrap_err();
        assert!(read_error.to_string().contains(field), "{read_error}");
        assert!(row_error.to_string().contains(field), "{row_error}");
    };
    // A reason without a category is valid; every other partial group still fails.
    for present in [1, 2, 3, 4, 6] {
        let mut partial = valid.clone();
        for (index, field) in [&mut partial.by, &mut partial.category, &mut partial.cause]
            .into_iter()
            .enumerate()
        {
            if present & (1 << index) == 0 {
                *field = None;
            }
        }
        assert_refused(partial, "requires by and cause");
    }

    let mut empty = valid.clone();
    empty.category = Some(String::new());
    assert_refused(empty, "model_fallback.category");
    for (by, category, cause, field) in [
        ("host", "valid", "refused_here", "model_fallback.by"),
        (
            "client",
            "bad category",
            "refused_here",
            "model_fallback.category",
        ),
        ("client", "valid", "later", "model_fallback.cause"),
    ] {
        let step = Fallback::new(by, "requested", category, cause);
        assert_refused(valid.clone().with_reason(&step), field);
    }
}

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
