use cortexkit_resource_busy::{HolderKind, ResourceBusy, ValidationError, RESOURCE_BUSY};

#[test]
fn json_vectors_round_trip_byte_for_byte() {
    for vector in [
        include_str!("vectors/head-browser-profile.json"),
        include_str!("vectors/flow-browser-profile.json"),
        include_str!("vectors/pointer.json"),
        include_str!("vectors/unknown-holder-kind.json"),
    ] {
        let detail: ResourceBusy = serde_json::from_str(vector).unwrap();
        detail.validate().unwrap();
        assert_eq!(serde_json::to_string(&detail).unwrap(), vector);
    }
}

#[test]
fn unknown_holder_kind_vector_is_preserved_as_given() {
    let vector = include_str!("vectors/unknown-holder-kind.json");
    let detail: ResourceBusy = serde_json::from_str(vector).unwrap();
    assert_eq!(detail.holder.kind, HolderKind::Other("scheduler".into()));
    assert_eq!(serde_json::to_string(&detail).unwrap(), vector);
}

#[test]
fn refusal_uses_the_shared_resource_busy_code() {
    assert_eq!(RESOURCE_BUSY, "resource_busy");
}

#[test]
fn validation_rejects_invalid_holder_combinations() {
    let flow_without_id: ResourceBusy = serde_json::from_str(
        r#"{"resource":"browser_profile","holder":{"kind":"flow"},"since_ms":100,"retry_after_ms":250}"#,
    )
    .unwrap();
    assert_eq!(
        flow_without_id.validate(),
        Err(ValidationError::MissingFlowId)
    );

    let head_with_id: ResourceBusy = serde_json::from_str(
        r#"{"resource":"browser_profile","holder":{"kind":"head","flow_id":"flow-1"},"since_ms":100,"retry_after_ms":250}"#,
    )
    .unwrap();
    assert_eq!(
        head_with_id.validate(),
        Err(ValidationError::UnexpectedFlowId)
    );
}

#[test]
fn validation_rejects_bad_resource_names() {
    for resource in ["", "Browser", "pointer-name", "two words", "é"] {
        let json = format!(
            "{{\"resource\":{},\"holder\":{{\"kind\":\"head\"}},\"since_ms\":100,\"retry_after_ms\":250}}",
            serde_json::to_string(resource).unwrap()
        );
        let detail: ResourceBusy = serde_json::from_str(&json).unwrap();
        assert_eq!(
            detail.validate(),
            Err(ValidationError::InvalidResource),
            "{resource:?}"
        );
    }
}
