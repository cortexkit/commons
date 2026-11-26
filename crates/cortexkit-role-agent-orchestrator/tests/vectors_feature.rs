use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cortexkit_role_agent_orchestrator::{
    errors::{self, Code, Refusal},
    ops,
    replies::{Item, ItemReply, ItemState, ListReply, SyncReply},
    request::Request,
    vectors, AWAITED_WORK, REQUIRED_OPS, ROLE, VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn fixture(name: &str) -> Value {
    serde_json::from_str(vectors::file(name).unwrap()).unwrap()
}

#[test]
fn packaged_vector_inventory_and_checksums() {
    assert_eq!(vectors::DIR, "awaited-work-v1");
    assert_eq!(vectors::file("README.md"), Some(vectors::README));
    assert_eq!(vectors::file("missing"), None);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join(vectors::DIR);
    let on_disk: BTreeSet<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "SHA256SUMS")
        .collect();
    let packaged: BTreeSet<_> = vectors::FILES
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();
    assert_eq!(on_disk, packaged);
    assert_eq!(vectors::FILES.len(), packaged.len());
    assert_eq!(
        std::fs::read(dir.join("SHA256SUMS")).unwrap(),
        vectors::SHA256SUMS.as_bytes()
    );

    let checksums: Vec<_> = vectors::SHA256SUMS.lines().collect();
    assert_eq!(checksums.len(), vectors::FILES.len());
    for (line, (name, contents)) in checksums.into_iter().zip(vectors::FILES) {
        let (hash, listed_name) = line.split_once("  ").unwrap();
        assert_eq!(listed_name, *name);
        let bytes = std::fs::read(dir.join(name)).unwrap();
        assert_eq!(bytes, contents.as_bytes(), "{name}");
        assert_eq!(hash, format!("{:x}", Sha256::digest(bytes)), "{name}");
    }
}

#[test]
fn all_methods_have_valid_closed_envelopes() {
    assert_eq!(
        (ROLE, VERSION, AWAITED_WORK),
        ("agent-orchestrator", "v1", "awaited-work/v1")
    );
    let requests = fixture("requests.json");
    let methods: Vec<_> = requests["valid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| {
            let request: Request = serde_json::from_value(value.clone()).unwrap();
            request.validate().unwrap();
            assert_eq!(serde_json::to_value(&request).unwrap(), *value);
            assert_eq!(request.method(), value["method"].as_str().unwrap());
            request.method()
        })
        .collect();
    assert_eq!(methods, REQUIRED_OPS);

    for case in requests["malformed"].as_array().unwrap() {
        let wire = case["request"].to_string();
        assert!(
            serde_json::from_str::<Request>(&wire).is_err(),
            "accepted {}",
            case["name"]
        );
    }

    let reordered: Request =
        serde_json::from_str(r#"{"params":{},"method":"awaited_work.list"}"#).unwrap();
    assert_eq!(reordered.method(), ops::AWAITED_WORK_LIST);
    let duplicate =
        r#"{"method":"awaited_work.cancel","params":{"instance":"d","key":"a","key":"b"}}"#;
    assert!(serde_json::from_str::<Request>(duplicate).is_err());
}

#[test]
fn finish_null_more_decodes_as_absent() {
    let mut value = fixture("requests.json")["null_more"].clone();
    let request: Request = serde_json::from_value(value.clone()).unwrap();
    request.validate().unwrap();
    let Request::Finish(params) = &request else {
        panic!("expected finish request");
    };
    assert_eq!(params.more, None);
    value["params"].as_object_mut().unwrap().remove("more");
    assert_eq!(serde_json::to_value(request).unwrap(), value);
}

#[test]
fn notice_record_vectors_round_trip() {
    let requests = fixture("requests.json");
    for name in ["finish_notice_record", "progress_notice_record"] {
        let value = &requests[name];
        let request: Request = serde_json::from_value(value.clone()).unwrap();
        request.validate().unwrap();
        assert_eq!(&serde_json::to_value(&request).unwrap(), value);
    }

    for method in [ops::AWAITED_WORK_FINISH, ops::AWAITED_WORK_PROGRESS] {
        let value = requests["valid"]
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["method"] == method)
            .unwrap();
        assert!(value["params"].get("notice_record").is_none());
        let request: Request = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(&serde_json::to_value(&request).unwrap(), value);
    }
}

fn assert_invalid(result: Result<(), Refusal>, field: &str, case: &str) {
    let refusal = result.expect_err(case);
    assert_eq!(refusal.code, Code::InvalidParams, "{case}");
    assert!(
        refusal.message.starts_with(&format!("{field}:")),
        "{case}: {}",
        refusal.message
    );
}

#[test]
fn request_bound_examples_validate_at_the_edges() {
    #[derive(Deserialize)]
    struct Case {
        name: String,
        request: Request,
        invalid_field: Option<String>,
    }

    let cases: Vec<Case> = serde_json::from_value(fixture("bounds.json")).unwrap();
    for case in cases {
        let result = case.request.validate();
        if let Some(field) = case.invalid_field {
            assert_invalid(result, &field, &case.name);
        } else {
            assert_eq!(result, Ok(()), "{}", case.name);
        }
    }
}

#[test]
fn shared_fields_are_validated_in_every_operation_that_carries_them() {
    let requests = fixture("requests.json");
    for value in requests["valid"].as_array().unwrap() {
        let method = value["method"].as_str().unwrap();
        if method == ops::AWAITED_WORK_LIST {
            continue;
        }
        let mut invalid = value.clone();
        invalid["params"]["instance"] = json!("not an instance");
        let request: Request = serde_json::from_value(invalid).unwrap();
        assert_invalid(request.validate(), "instance", method);
        if method != ops::AWAITED_WORK_SYNC {
            let mut invalid = value.clone();
            invalid["params"]["key"] = json!("not a key");
            let request: Request = serde_json::from_value(invalid).unwrap();
            assert_invalid(request.validate(), "key", method);
        }
    }
}

#[test]
fn refusal_codes_are_complete_and_terminal() {
    let mut known = BTreeSet::new();
    let mut known_counts = BTreeMap::new();
    let mut future = 0;
    for value in fixture("refusals.json").as_array().unwrap() {
        let refusal: Refusal = serde_json::from_value(value.clone()).unwrap();
        let code = value["code"].as_str().unwrap();
        assert_eq!(refusal.code.as_str(), code);
        assert!(errors::is_terminal(code));
        assert!(refusal.is_terminal());
        if let Code::Other(actual) = &refusal.code {
            assert_eq!(actual, "future_refusal");
            future += 1;
        } else {
            known.insert(code.to_string());
            *known_counts.entry(code.to_string()).or_insert(0usize) += 1;
        }
        let mut expected = value.clone();
        expected.as_object_mut().unwrap().remove("future_member");
        assert_eq!(serde_json::to_value(refusal).unwrap(), expected);
    }
    assert_eq!(
        known,
        errors::CODES.iter().map(|code| code.to_string()).collect()
    );
    for code in errors::CODES {
        let expected = if *code == "invalid_params" { 2 } else { 1 };
        assert_eq!(known_counts.get(*code), Some(&expected), "{code}");
    }
    assert_eq!(future, 1);
}

#[test]
fn replies_ignore_future_members_and_keep_future_states() {
    let file = fixture("replies.json");
    let states = [
        ItemState::Open,
        ItemState::Done,
        ItemState::Failed,
        ItemState::Cancelled,
        ItemState::Lost,
        ItemState::SessionEnded,
        ItemState::Other("future_state".into()),
    ];
    let items = file["item"].as_array().unwrap();
    assert_eq!(items.len(), states.len());
    for (value, state) in items.iter().zip(states) {
        let reply: ItemReply = serde_json::from_value(value.clone()).unwrap();
        let mut item = Item::new("work!42~", "Check the demo files", state, 1795700000000);
        if value["item"]["state"] != "open" {
            item = item.with_ended_at_ms(1795700000010);
        }
        assert_eq!(reply, ItemReply::new(item));
        assert_eq!(
            reply.item.state.is_ended(),
            value["item"]["state"] != "open"
        );
        let mut expected = value.clone();
        expected
            .as_object_mut()
            .unwrap()
            .remove("future_reply_member");
        expected["item"]
            .as_object_mut()
            .unwrap()
            .remove("future_item_member");
        assert_eq!(serde_json::to_value(reply).unwrap(), expected);
    }

    let syncs = file["sync"].as_array().unwrap();
    assert_eq!(syncs.len(), 1);
    let sync: SyncReply = serde_json::from_value(syncs[0].clone()).unwrap();
    assert_eq!(
        sync,
        SyncReply::new(vec!["work!42~".into()], vec!["work!43~".into()])
    );
    assert_eq!(
        serde_json::to_value(sync).unwrap(),
        json!({"lost": ["work!42~"], "unknown": ["work!43~"]})
    );

    let lists = file["list"].as_array().unwrap();
    assert_eq!(lists.len(), 2);
    let open: ItemReply = serde_json::from_value(items[0].clone()).unwrap();
    for (value, list) in lists
        .iter()
        .zip([ListReply::new(vec![open.item]), ListReply::new(vec![])])
    {
        let reply: ListReply = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(reply, list);
        assert_eq!(
            serde_json::to_value(reply).unwrap(),
            json!({"items": value["items"]})
        );
    }
}
