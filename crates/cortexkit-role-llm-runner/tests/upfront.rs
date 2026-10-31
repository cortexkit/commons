use cortexkit_role_llm_runner::{content_digest::sha256_hex, errors, upfront};
use serde_json::{json, Value};

#[test]
fn tool_upfront_vectors_check_every_sample() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/llm-runner-v1/tool-upfront");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let sample: Value = serde_json::from_slice(&bytes).unwrap();
        let plan = &sample["plan"];
        let original = serde_json::to_vec(plan).unwrap();
        let result = upfront::check_plan(plan, sample["declared"].as_bool().unwrap());
        let refusal = sample.get("refusal");
        match result {
            Err(field) => {
                assert_eq!(refusal.unwrap()["code"], errors::INVALID_PARAMS, "{path:?}");
                assert_eq!(
                    refusal.unwrap()["detail"],
                    json!({"field": field}),
                    "{path:?}"
                );
            }
            Ok(()) => {
                if let Some(list) = plan["tool_items"][0].get("upfront") {
                    let list: Vec<String> = serde_json::from_value(list.clone()).unwrap();
                    let offered: Vec<String> =
                        serde_json::from_value(sample["offered"].clone()).unwrap();
                    let result = upfront::resolve(
                        plan["tool_items"][0]["provider"].as_str().unwrap(),
                        &list,
                        offered.iter().map(String::as_str),
                    );
                    if let Some(refusal) = refusal {
                        let stale = result.unwrap_err();
                        assert_eq!(stale.code(), refusal["code"], "{path:?}");
                        assert_eq!(stale.detail(), refusal["detail"], "{path:?}");
                    } else {
                        assert_eq!(json!(result.unwrap()), sample["resolved"], "{path:?}");
                    }
                } else {
                    assert!(refusal.is_none(), "{path:?}");
                    assert!(upfront::check_plan(plan, true).is_ok());
                }
            }
        }
        assert_eq!(serde_json::to_vec(plan).unwrap(), original, "{path:?}");
        checked += 1;
    }
    assert_eq!(checked, 8);
}

#[test]
fn tool_upfront_refusals_name_the_item_even_when_optional() {
    for value in [
        json!([""]),
        json!(null),
        json!(false),
        json!({"read": true}),
    ] {
        let plan = json!({"tool_items": [{"provider": "example.first"}, {"provider": "example.second", "optional": true, "upfront": value}]});
        assert_eq!(
            upfront::check_plan(&plan, true),
            Err("plan.tool_items[1].upfront".into())
        );
    }
    let plan = json!({"tool_items": [{}, {"upfront": ["read"]}]});
    assert_eq!(
        upfront::check_plan(&plan, false),
        Err("plan.tool_items[1].upfront".into())
    );
}

#[test]
fn tool_upfront_uses_exact_names_and_keeps_declaration_order() {
    let list = vec![
        "Read".into(),
        " read ".into(),
        "読み取り".into(),
        "read".into(),
    ];
    assert!(upfront::check_plan(&json!({"tool_items": [{"upfront": list}]}), true).is_ok());
    assert_eq!(
        upfront::resolve(
            "example.tools",
            &list,
            ["read", "Read", "読み取り", " read ", "extra"]
        ),
        Ok(list.clone())
    );
    let stale = upfront::resolve("example.tools", &list, ["read"]).unwrap_err();
    assert_eq!(
        stale.detail(),
        json!({"differences": [
            {"kind": "upfront", "provider": "example.tools", "name": "Read"},
            {"kind": "upfront", "provider": "example.tools", "name": " read "},
            {"kind": "upfront", "provider": "example.tools", "name": "読み取り"}
        ]})
    );
    assert!(!errors::is_retryable(stale.code()));
}

#[test]
fn tool_upfront_cannot_use_another_items_offered_name() {
    let list = vec!["search".into()];
    let first = upfront::resolve("example.first", &list, ["read"]);
    assert_eq!(
        first.unwrap_err().detail(),
        json!({"differences": [
            {"kind": "upfront", "provider": "example.first", "name": "search"}
        ]})
    );
    assert_eq!(
        upfront::resolve("example.second", &list, ["search"]),
        Ok(list)
    );
}

#[test]
fn tool_upfront_absence_keeps_legacy_wire_bytes() {
    // Pin the corpus from before upfront existed, independently of the current
    // checksum manifest, so a wire change cannot silently rewrite old plans.
    let bytes = include_bytes!("../test-vectors/llm-runner-v1/plans.json");
    assert_eq!(
        sha256_hex(bytes),
        "803dcf6ae25740fd7d971f8054630666aca545c51d087bfe37cf9d9c3a0f2b79"
    );
    let corpus: Value = serde_json::from_slice(bytes).unwrap();
    for section in ["admission_checks", "frozen_cases"] {
        for sample in corpus[section].as_array().unwrap() {
            for field in ["request", "first_send", "later_send"] {
                if let Some(request) = sample.get(field) {
                    let wire: cortexkit_role_llm_runner::send::SendRequest =
                        serde_json::from_value(request.clone()).unwrap();
                    if let Some(plan) = &wire.plan {
                        let before = serde_json::to_vec(plan).unwrap();
                        assert!(upfront::check_plan(&json!(plan), false).is_ok());
                        let after = serde_json::to_value(&wire).unwrap();
                        assert_eq!(serde_json::to_vec(&after["plan"]).unwrap(), before);
                    }
                }
            }
        }
    }
}

#[test]
fn tool_upfront_is_checked_on_a_composer_shaped_plan() {
    // A plan composer sends tool items in `tool_items`, beside its other item
    // lists; the helper must check the hints there.
    let plan = json!({
        "system_text_items": [{"provider": "example.text"}],
        "tool_items": [
            {"provider": "example.first"},
            {"provider": "example.second", "upfront": ["read", "read"]}
        ],
        "compaction_item": {"provider": "example.compaction"}
    });
    assert_eq!(
        upfront::check_plan(&plan, true),
        Err("plan.tool_items[1].upfront".into())
    );
    let plan = json!({"tool_items": [{"provider": "example.first", "upfront": ["read"]}]});
    assert!(upfront::check_plan(&plan, true).is_ok());
    assert_eq!(
        upfront::check_plan(&plan, false),
        Err("plan.tool_items[0].upfront".into())
    );
}

#[test]
fn tool_upfront_check_item_matches_check_plan_for_each_item() {
    let items = [
        json!({"provider": "example.first"}),
        json!({"provider": "example.second", "upfront": ["read"]}),
        json!({"provider": "example.third", "upfront": []}),
    ];
    let plan = json!({"tool_items": items});
    let first_failure = items
        .iter()
        .enumerate()
        .find_map(|(index, item)| upfront::check_item(item, index, true).err());
    assert_eq!(first_failure, Some("plan.tool_items[2].upfront".into()));
    assert_eq!(upfront::check_plan(&plan, true).err(), first_failure);
    assert_eq!(
        upfront::check_item(&items[1], 1, false),
        Err("plan.tool_items[1].upfront".into())
    );
}
