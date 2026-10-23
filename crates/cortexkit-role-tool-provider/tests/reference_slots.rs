use cortexkit_role_tool_provider::catalog::{check_reference_slots, CatalogTool, ReferenceSlot};
use serde_json::json;

#[test]
fn reference_slots_preserve_absent_catalog_bytes() {
    let tool = CatalogTool::new("read", "digest", 1, json!({"type":"object"}));
    let before = serde_json::to_vec(&tool).unwrap();
    assert!(!serde_json::to_value(&tool)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("reference_slots"));
    assert_eq!(
        before,
        serde_json::to_vec(&tool.with_reference_slots(vec![])).unwrap()
    );
}

#[test]
fn reference_slots_validate_by_name() {
    let slots = vec![
        ReferenceSlot::new("data", "token"),
        ReferenceSlot::new("program", "source:rich-ui/1"),
    ];
    assert!(check_reference_slots(&slots).is_ok());
    assert_eq!(
        check_reference_slots(&vec![ReferenceSlot::new("a", "token"); 9])
            .unwrap_err()
            .field,
        "reference_slots"
    );
    for invalid in [
        vec![ReferenceSlot::new("", "token")],
        vec![
            ReferenceSlot::new("data", "token"),
            ReferenceSlot::new("data", "token"),
        ],
    ] {
        assert_eq!(
            check_reference_slots(&invalid).unwrap_err().field,
            "reference_slots[].name"
        );
    }
    for kind in ["", "source:", "source", "Token", "source-rich-ui/1"] {
        let error = check_reference_slots(&[ReferenceSlot::new("data", kind)]).unwrap_err();
        assert_eq!(error.field, "reference_slots[].kind");
        assert_eq!(error.name.as_deref(), Some("data"));
    }
    assert_eq!(
        serde_json::to_value(&slots).unwrap(),
        json!([
            {"name":"data","kind":"token"}, {"name":"program","kind":"source:rich-ui/1"}
        ])
    );
}

#[cfg(feature = "test-vectors")]
#[test]
fn reference_slots_vectors() {
    let v: serde_json::Value = serde_json::from_str(
        cortexkit_role_tool_provider::vectors::file("reference-slots.json").unwrap(),
    )
    .unwrap();
    for slots in v["valid"].as_array().unwrap() {
        let slots: Vec<ReferenceSlot> = serde_json::from_value(slots.clone()).unwrap();
        assert!(check_reference_slots(&slots).is_ok());
    }
    for case in v["invalid"].as_array().unwrap() {
        let slots: Vec<ReferenceSlot> = serde_json::from_value(case["slots"].clone()).unwrap();
        assert_eq!(
            check_reference_slots(&slots).unwrap_err().field,
            case["field"]
        );
    }
    let tool: CatalogTool = serde_json::from_value(v["show_ui"].clone()).unwrap();
    assert_eq!(serde_json::to_value(tool).unwrap(), v["show_ui"]);
}
