use cortexkit_role_compaction::{
    part::{body_digest, Part, PartBody},
    provides_numbers_unknown,
    read::{entry_digest, Entry, Role},
    view::ViewRefusal,
    wire::{codes, Refusal},
    COMPACTION_NUMBERS_UNKNOWN_PROVIDES, COMPACTION_PROVIDES,
};
use serde_json::{json, Value};

fn entry() -> Entry {
    Entry::new(
        1,
        "message-a",
        Role::User,
        1_700_000_000_000,
        vec![Part::new(
            "part-a",
            PartBody::Text {
                text: "hello".into(),
            },
        )],
    )
}

#[test]
fn numbers_unknown_entry_round_trip() {
    let plain = serde_json::to_value(entry()).unwrap();
    assert!(plain.get("numbers_unknown").is_none());
    for marker in [None, Some(false), Some(true)] {
        let mut wire = plain.clone();
        if let Some(marker) = marker {
            wire["numbers_unknown"] = json!(marker);
        }
        let decoded: Entry = serde_json::from_value(wire).unwrap();
        let encoded = serde_json::to_value(decoded).unwrap();
        assert_eq!(
            encoded.get("numbers_unknown"),
            marker.filter(|value| *value).map(|_| json!(true)).as_ref(),
        );
        assert_eq!(encoded["parts"], plain["parts"]);
    }
}

#[test]
fn numbers_unknown_marker_changes_no_digest() {
    let plain = entry();
    let mut wire = serde_json::to_value(&plain).unwrap();
    wire["numbers_unknown"] = json!(true);
    let marked: Entry = serde_json::from_value(wire).unwrap();
    assert_eq!(marked.digest, plain.digest);
    assert_eq!(marked.parts, plain.parts);
    assert_eq!(
        entry_digest(marked.role.clone(), marked.notice.as_ref(), &marked.parts),
        plain.digest,
    );
    for part in &marked.parts {
        assert_eq!(body_digest(part.seal.clone(), &part.body), part.digest);
    }
}

#[test]
fn numbers_unknown_capability_is_exact_and_provider_declared() {
    assert_eq!(
        COMPACTION_NUMBERS_UNKNOWN_PROVIDES,
        "compaction-numbers-unknown/v1"
    );
    for (capabilities, expected) in [
        (vec![], false),
        (vec![COMPACTION_PROVIDES.into()], false),
        (vec!["compaction-numbers-unknown/v2".into()], false),
        (vec!["compaction-numbers-unknown/v1-extra".into()], false),
        (
            vec![
                COMPACTION_PROVIDES.into(),
                COMPACTION_NUMBERS_UNKNOWN_PROVIDES.into(),
            ],
            true,
        ),
    ] {
        assert_eq!(provides_numbers_unknown(&capabilities), expected);
    }
}

#[test]
fn numbers_unknown_read_refusal_carries_requested_tags() {
    let refusal = Refusal::numbers_unknown(vec![7, 8]);
    assert_eq!(refusal.code, codes::NUMBERS_UNKNOWN);
    assert_eq!(refusal.code, "numbers_unknown");
    assert_eq!(refusal.detail, Some(json!({"tags": [7, 8]})));
    assert_ne!(refusal.code, codes::TRANSIENT);
    let decoded: Refusal = serde_json::from_value(serde_json::to_value(&refusal).unwrap()).unwrap();
    assert_eq!(decoded, refusal);
}

#[test]
fn numbers_unknown_view_refusal_has_typed_unknown_through() {
    let wire = json!({"code": "cut_before_numbers", "unknown_through": "message-a"});
    let refusal: ViewRefusal = serde_json::from_value(wire.clone()).unwrap();
    assert!(
        matches!(&refusal, ViewRefusal::CutBeforeNumbers { unknown_through } if unknown_through == "message-a")
    );
    assert_eq!(serde_json::to_value(refusal).unwrap(), wire);
}

#[test]
fn numbers_unknown_rejects_non_boolean_markers() {
    for marker in [Value::Null, json!(1), json!("true")] {
        let mut wire = serde_json::to_value(entry()).unwrap();
        wire["numbers_unknown"] = marker;
        assert!(serde_json::from_value::<Entry>(wire).is_err());
    }
}
