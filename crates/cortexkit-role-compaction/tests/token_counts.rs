use cortexkit_role_compaction::{
    digest::content_digest,
    notice::NoticeRequest,
    part::{body_digest, PartBody},
    read::{entry_digest, Entry, ReadPage},
    tokens::{PartTokens, ReasoningTokens, SurfaceTokens},
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

mod support;

#[test]
fn token_counts_round_trip() {
    let page = json!({
        "token_family": "claude-v5/1",
        "token_family_exact": true,
        "entries": [{
            "ordinal": 1, "mid": "m1", "role": "assistant", "written_at": 1000,
            "digest": "entry-digest",
            "reasoning_tokens": {"count": 10, "measured": false},
            "parts": [
                {"id": "text", "kind": "text", "text": "Hello", "digest": "text-digest",
                 "tokens": {"count": 2}},
                {"id": "tool", "kind": "tool", "call_id": "call1", "name": "read",
                 "input": {}, "status": "completed", "output_bytes": 5,
                 "digest": "tool-digest", "tokens": {"input": 3, "output": 4}},
                {"id": "file", "kind": "file", "mime": "image/png", "transport": "inline",
                 "digest": "file-digest", "tokens": {"count": 100}},
                {"id": "reasoning", "kind": "reasoning", "text_bytes": 20,
                 "digest": "reasoning-digest"}
            ]
        }, {
            "ordinal": 2, "mid": "m2", "role": "assistant", "written_at": 2000,
            "digest": "empty-digest", "parts": [],
            "reasoning_tokens": {"count": 8, "measured": true}
        }],
        "head": {}
    });
    let decoded: ReadPage = serde_json::from_value(page.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), page);

    let notice = json!({
        "seq": 1, "token_family": "claude-v5/1", "entries": page["entries"],
        "token_family_exact": false,
        "conditions": {
            "model": {"provider": "anthropic", "model": "claude-v5"},
            "context_window": 200000, "output_reserve": 10000,
            "rules": {"signed_parts": "bound", "history_edit_cost": "suffix"},
            "system_tokens": {"count": 20, "exact": true},
            "tools_tokens": {"count": 30, "exact": false}
        }
    });
    let decoded: NoticeRequest = serde_json::from_value(notice.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), notice);
}

fn without_counts(mut response: Value) -> Value {
    response.as_object_mut().unwrap().remove("token_family");
    response
        .as_object_mut()
        .unwrap()
        .remove("token_family_exact");
    if let Some(conditions) = response.get_mut("conditions") {
        let conditions = conditions.as_object_mut().unwrap();
        conditions.remove("system_tokens");
        conditions.remove("tools_tokens");
    }
    for entry in response["entries"].as_array_mut().unwrap() {
        entry.as_object_mut().unwrap().remove("reasoning_tokens");
        for part in entry["parts"].as_array_mut().unwrap() {
            part.as_object_mut().unwrap().remove("tokens");
        }
    }
    response
}

#[test]
fn token_counts_do_not_change_embedded_entry_or_part_digests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/compaction-v2");
    let page: Value =
        serde_json::from_slice(&std::fs::read(root.join("entries-token-counts.json")).unwrap())
            .unwrap();
    let notice: Value = serde_json::from_slice(
        &std::fs::read(root.join("notice-token-counts-request.json")).unwrap(),
    )
    .unwrap();
    for counted in [page, notice["params"].clone()] {
        let plain = without_counts(counted.clone());
        let counted_entries: Vec<Entry> =
            serde_json::from_value(counted["entries"].clone()).unwrap();
        let plain_entries: Vec<Entry> = serde_json::from_value(plain["entries"].clone()).unwrap();
        for ((counted, plain), full) in counted_entries
            .iter()
            .zip(&plain_entries)
            .zip(support::counted_entries())
        {
            assert_eq!(counted.digest, plain.digest);
            let mut full_plain = full.clone();
            full_plain.reasoning_tokens = None;
            for part in &mut full_plain.parts {
                part.tokens = None;
            }
            // Compute the expected W7 entry digest from the stripped full JSON,
            // independently of entry_digest's metadata filtering.
            let full_wire = serde_json::to_value(&full_plain).unwrap();
            let expected = content_digest(&json!({
                "role": full_wire["role"], "parts": full_wire["parts"]
            }));
            assert_eq!(counted.digest, expected);
            for entry in [&full, &full_plain] {
                assert_eq!(
                    entry_digest(entry.role.clone(), entry.notice.as_ref(), &entry.parts),
                    expected
                );
            }
            for ((counted_part, plain_part), full_part) in
                counted.parts.iter().zip(&plain.parts).zip(&full.parts)
            {
                assert_eq!(counted_part.digest, plain_part.digest);
                let mut body = serde_json::to_value(full_part).unwrap();
                for key in ["id", "tag", "digest", "tokens"] {
                    body.as_object_mut().unwrap().remove(key);
                }
                let expected = content_digest(&body);
                assert_eq!(counted_part.digest, expected);
                assert_eq!(
                    body_digest(full_part.seal.clone(), &full_part.body),
                    expected
                );
                assert_eq!(counted_part.tokens, full_part.tokens);
                assert_eq!(plain_part.tokens, None);
            }
            assert_eq!(plain.reasoning_tokens, None);
        }
        if counted.get("conditions").is_some() {
            let decoded: NoticeRequest = serde_json::from_value(plain.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), plain);
        } else {
            let decoded: ReadPage = serde_json::from_value(plain.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), plain);
        }
    }
}

#[test]
fn token_counts_are_optional_and_survive_compact_detail() {
    let page = support::counted_page();
    let notice = support::counted_notice();
    assert_eq!(page.entries, notice.entries);
    assert_eq!(page.token_family.as_deref(), Some("claude-v5/1"));
    assert_eq!(notice.token_family, page.token_family);
    assert_eq!(page.token_family_exact, Some(true));
    assert_eq!(notice.token_family_exact, Some(false));
    assert!(notice.conditions.system_tokens.unwrap().exact);
    assert!(!notice.conditions.tools_tokens.unwrap().exact);
    assert!(!page.entries[0].reasoning_tokens.unwrap().measured);
    assert!(page.entries[1].reasoning_tokens.unwrap().measured);
    assert!(matches!(page.entries[0].parts[1].body, PartBody::Tool(ref t) if t.output.is_none()));
    assert!(matches!(
        page.entries[0].parts[3].body,
        PartBody::Reasoning { text: None, .. }
    ));
    assert_eq!(page.entries[0].parts[3].tokens, None);
    assert_eq!(page.entries[1].parts[0].tokens, None);

    let plain_page = ReadPage::new(vec![], cortexkit_role_compaction::read::Head::new());
    assert!(!plain_page.token_family_exact.unwrap_or(false));
    assert!(serde_json::to_value(&plain_page)
        .unwrap()
        .get("token_family_exact")
        .is_none());
    assert!(serde_json::to_value(plain_page)
        .unwrap()
        .get("token_family")
        .is_none());
    let mut plain_notice = NoticeRequest::new(1, notice.conditions);
    plain_notice.conditions.system_tokens = None;
    plain_notice.conditions.tools_tokens = None;
    assert!(!plain_notice.token_family_exact.unwrap_or(false));
    let wire = serde_json::to_value(plain_notice).unwrap();
    assert!(wire.get("token_family").is_none());
    assert!(wire.get("token_family_exact").is_none());
    assert!(wire["conditions"].get("system_tokens").is_none());
    assert!(wire["conditions"].get("tools_tokens").is_none());
}

fn check_integer<T: DeserializeOwned>(mut value: Value, field: &str) {
    for valid in [0, 9_007_199_254_740_991_u64] {
        value[field] = json!(valid);
        assert!(serde_json::from_value::<T>(value.clone()).is_ok());
    }
    for invalid in [
        json!(9_007_199_254_740_992_u64),
        json!(-1),
        json!(1.5),
        json!("1"),
        Value::Null,
    ] {
        value[field] = invalid;
        assert!(serde_json::from_value::<T>(value.clone()).is_err());
    }
}

#[test]
fn token_count_decoding_checks_integer_and_boolean_shapes() {
    check_integer::<PartTokens>(json!({"count": 1}), "count");
    check_integer::<PartTokens>(json!({"input": 1, "output": 2}), "input");
    check_integer::<PartTokens>(json!({"input": 1, "output": 2}), "output");
    check_integer::<ReasoningTokens>(json!({"count": 1, "measured": false}), "count");
    check_integer::<SurfaceTokens>(json!({"count": 1, "exact": true}), "count");
    for invalid in [Value::Null, json!(0), json!("true")] {
        assert!(serde_json::from_value::<ReasoningTokens>(
            json!({"count": 1, "measured": invalid})
        )
        .is_err());
        assert!(
            serde_json::from_value::<SurfaceTokens>(json!({"count": 1, "exact": invalid})).is_err()
        );
    }
    assert!(serde_json::from_value::<ReasoningTokens>(json!({"count": 1})).is_err());
    assert!(serde_json::from_value::<SurfaceTokens>(json!({"count": 1})).is_err());
    assert!(serde_json::from_value::<PartTokens>(json!({"input": 1})).is_err());
    assert!(serde_json::from_value::<PartTokens>(json!({"output": 1})).is_err());

    for exact in [true, false] {
        let mut page = support::counted_page();
        page.token_family_exact = Some(exact);
        let decoded: ReadPage =
            serde_json::from_value(serde_json::to_value(&page).unwrap()).unwrap();
        assert_eq!(decoded, page);
        let mut notice = support::counted_notice();
        notice.token_family_exact = Some(exact);
        let decoded: NoticeRequest =
            serde_json::from_value(serde_json::to_value(&notice).unwrap()).unwrap();
        assert_eq!(decoded, notice);
    }
    let mut page = serde_json::to_value(support::counted_page()).unwrap();
    let mut notice = serde_json::to_value(support::counted_notice()).unwrap();
    page.as_object_mut().unwrap().remove("token_family_exact");
    notice.as_object_mut().unwrap().remove("token_family_exact");
    assert!(!serde_json::from_value::<ReadPage>(page.clone())
        .unwrap()
        .token_family_exact
        .unwrap_or(false));
    assert!(!serde_json::from_value::<NoticeRequest>(notice.clone())
        .unwrap()
        .token_family_exact
        .unwrap_or(false));
    for invalid in [json!(0), json!("true"), json!({}), json!([])] {
        page["token_family_exact"] = invalid.clone();
        notice["token_family_exact"] = invalid;
        assert!(serde_json::from_value::<ReadPage>(page.clone()).is_err());
        assert!(serde_json::from_value::<NoticeRequest>(notice.clone()).is_err());
    }
}

#[test]
fn token_metadata_does_not_hide_opaque_content_changes() {
    let mut entry = support::counted_entries().remove(0);
    let original = entry.digest.clone();
    entry.parts[0].tokens = Some(PartTokens::count(99));
    entry.reasoning_tokens = Some(ReasoningTokens::new(21, false));
    assert_eq!(
        entry_digest(entry.role.clone(), None, &entry.parts),
        original
    );
    let PartBody::Tool(tool) = &mut entry.parts[1].body else {
        panic!("expected tool")
    };
    let mut input = tool.input.parse();
    input["tokens"]["count"] = json!(8);
    tool.input = input.into();
    assert_ne!(entry_digest(entry.role, None, &entry.parts), original);
}
