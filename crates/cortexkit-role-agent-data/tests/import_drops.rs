use std::collections::BTreeMap;

use cortexkit_role_agent_data::import::{
    ImportReply, ImportedItem, DROPPED_MAX_COUNT, DROPPED_MAX_REASONS, DROPPED_REASON_MAX_BYTES,
};
use serde_json::{json, Value};

#[test]
fn import_dropped_builder_matches_named_vector_refusals() {
    let file: Value = serde_json::from_str(include_str!(
        "../test-vectors/agent-data-v1/import-drops.json"
    ))
    .unwrap();
    let mut checked = 0;
    for case in file["refused"].as_array().unwrap() {
        // Negative and fractional counts cannot enter the builder's u64 map.
        if case["rule"] == "u64" {
            continue;
        }
        let dropped: BTreeMap<String, u64> =
            serde_json::from_value(case["reply"]["items"][0]["dropped"].clone()).unwrap();
        let err = ImportedItem::new("session", 0, "digest")
            .with_dropped(dropped)
            .expect_err(case["name"].as_str().unwrap());
        assert!(err.contains(case["rule"].as_str().unwrap()), "{err}");
        checked += 1;
    }
    assert_eq!(checked, 12);
}

#[test]
fn import_dropped_builder_and_decoder_accept_inclusive_boundaries_per_item() {
    assert_eq!(DROPPED_MAX_REASONS, 16);
    assert_eq!(DROPPED_REASON_MAX_BYTES, 64);
    assert_eq!(DROPPED_MAX_COUNT, 9_007_199_254_740_991);

    let mut dropped: BTreeMap<String, u64> = (0..15)
        .map(|n| (format!("r{n}"), 9_007_199_254_740_991))
        .collect();
    let longest_reason = format!("{}_9", "a".repeat(62));
    dropped.insert(longest_reason, 1);
    let with_drops = ImportedItem::new("session", 0, "digest")
        .with_dropped(dropped.clone())
        .unwrap();
    assert_eq!(with_drops.dropped, Some(dropped.clone()));
    let without_drops = ImportedItem::new("other", 1, "other-digest");
    assert_eq!(without_drops.dropped, None);

    let reply = ImportReply::new(vec![with_drops, without_drops]);
    let expected = json!({"items": [
        {"sessionRef": "session", "endOffset": 0, "digest": "digest", "dropped": dropped},
        {"sessionRef": "other", "endOffset": 1, "digest": "other-digest"}
    ]});
    assert_eq!(serde_json::to_value(&reply).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<ImportReply>(expected).unwrap(),
        reply
    );

    let shortest = ImportedItem::new("session", 0, "digest")
        .with_dropped(BTreeMap::from([("a".into(), 1)]))
        .unwrap();
    assert_eq!(shortest.dropped.unwrap()["a"], 1);
}
