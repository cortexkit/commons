use cortexkit_role_compaction::{
    notice::NoticeRequest, provides_notes_tokens, tokens::SurfaceTokens,
    COMPACTION_NOTES_TOKENS_PROVIDES, COMPACTION_PROVIDES,
};
use serde_json::{json, Value};

const DECLARED: &str =
    include_str!("../test-vectors/compaction-v2/notice-notes-declared-request.json");
const LEGACY: &str = include_str!("../test-vectors/compaction-v2/notice-notes-legacy-request.json");

#[test]
fn notes_tokens_wire_vectors_round_trip() {
    for (fixture, notes, system) in [(DECLARED, Some(25), 100), (LEGACY, None, 125)] {
        let value: Value = serde_json::from_str(fixture).unwrap();
        let notice: NoticeRequest = serde_json::from_value(value["params"].clone()).unwrap();
        let encoded = serde_json::to_value(notice).unwrap();
        assert_eq!(encoded, value["params"]);
        assert_eq!(encoded["conditions"]["system_tokens"]["count"], system);
        assert_eq!(
            encoded["conditions"].get("notes_tokens"),
            notes
                .map(|count| json!({"count": count, "exact": true}))
                .as_ref()
        );
    }
}

#[test]
fn notes_tokens_rejects_invalid_count_or_exact() {
    for invalid in [
        json!({"count": -1, "exact": true}),
        json!({"count": 1.5, "exact": true}),
        json!({"count": 9_007_199_254_740_992_u64, "exact": true}),
        json!({"count": 25}),
        json!({"count": 25, "exact": "true"}),
    ] {
        let mut value: Value = serde_json::from_str(DECLARED).unwrap();
        value["params"]["conditions"]["notes_tokens"] = invalid.clone();
        assert!(
            serde_json::from_value::<NoticeRequest>(value["params"].clone()).is_err(),
            "accepted invalid notes_tokens: {invalid}"
        );
    }
}

#[test]
fn notes_tokens_projects_only_for_declaring_providers() {
    assert_eq!(
        COMPACTION_NOTES_TOKENS_PROVIDES,
        "compaction-notes-tokens/v1"
    );
    let value: Value = serde_json::from_str(LEGACY).unwrap();
    let notice: NoticeRequest = serde_json::from_value(value["params"].clone()).unwrap();
    assert!(notice.conditions.notes_tokens.is_none());
    for declared in [false, true] {
        let mut provides = vec![COMPACTION_PROVIDES.to_owned()];
        if declared {
            provides.push(COMPACTION_NOTES_TOKENS_PROVIDES.to_owned());
        }
        assert_eq!(provides_notes_tokens(&provides), declared);
        let count = SurfaceTokens::new(25, true);
        let conditions = notice
            .conditions
            .clone()
            .with_notes_tokens(count, provides_notes_tokens(&provides));
        assert_eq!(conditions.notes_tokens, declared.then_some(count));
        assert_eq!(
            serde_json::to_value(&conditions)
                .unwrap()
                .get("notes_tokens")
                .is_some(),
            declared
        );
        let cleared = conditions.with_notes_tokens(count, false);
        assert!(cleared.notes_tokens.is_none());
    }
    assert!(!provides_notes_tokens(&[
        "compaction-notes-tokens/v2".into()
    ]));
}
