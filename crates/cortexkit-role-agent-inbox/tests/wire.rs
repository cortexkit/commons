use cortexkit_role_agent_inbox::{
    errors::{self, Code, Refusal},
    neutralize,
    post::*,
    read::*,
    withdraw::*,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/agent-inbox-v1")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) -> T {
    let decoded: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), *value);
    decoded
}

#[test]
fn post_shapes() {
    let file = fixture("posts.json");
    for value in file["requests"].as_array().unwrap() {
        round_trip::<PostRequest>(value).validate().unwrap();
    }
    for value in file["replies"].as_array().unwrap() {
        round_trip::<PostReply>(value);
    }
    for value in file["invalid"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<PostRequest>(value.clone()).is_err(),
            "{value}"
        );
    }
    for value in file["invalid_semantics"].as_array().unwrap() {
        assert!(serde_json::from_value::<PostRequest>(value.clone())
            .unwrap()
            .validate()
            .is_err());
    }
}

#[test]
fn withdraw_shapes() {
    let file = fixture("withdraw.json");
    for value in file["requests"].as_array().unwrap() {
        round_trip::<WithdrawRequest>(value);
    }
    for value in file["replies"].as_array().unwrap() {
        round_trip::<WithdrawReply>(value);
    }
    for value in file["invalid"].as_array().unwrap() {
        assert!(serde_json::from_value::<WithdrawRequest>(value.clone()).is_err());
    }
}

#[test]
fn pull_shapes() {
    let file = fixture("pull.json");
    for value in file["list_requests"].as_array().unwrap() {
        round_trip::<ListRequest>(value);
    }
    for value in file["read_requests"].as_array().unwrap() {
        round_trip::<ReadRequest>(value);
    }
    for value in file["list_replies"].as_array().unwrap() {
        round_trip::<ListReply>(value);
    }
    for value in file["read_replies"].as_array().unwrap() {
        round_trip::<ReadReply>(value);
    }
    for value in file["invalid_list"].as_array().unwrap() {
        assert!(serde_json::from_value::<ListRequest>(value.clone()).is_err());
    }
    for value in file["invalid_read"].as_array().unwrap() {
        assert!(serde_json::from_value::<ReadRequest>(value.clone()).is_err());
    }
}

#[test]
fn refusal_finality() {
    let file = fixture("refusals.json");
    let mut known = std::collections::BTreeSet::new();
    for value in file.as_array().unwrap() {
        let reply = round_trip::<Refusal>(&value["reply"]);
        let terminal = value["terminal"].as_bool().unwrap();
        assert_eq!(reply.is_terminal(), terminal);
        assert_eq!(errors::is_retryable(reply.code.as_str()), !terminal);
        if !matches!(reply.code, Code::Other(_)) {
            known.insert(reply.code.as_str().to_owned());
        }
    }
    assert_eq!(known, errors::CODES.iter().map(|s| s.to_string()).collect());
    assert!(!errors::CODES.contains(&"no_implementer"));
}

#[test]
fn neutralize_pairs() {
    for pair in fixture("neutralize.json").as_array().unwrap() {
        let input = pair["input"].as_str().unwrap();
        let expected = pair["output"].as_str().unwrap();
        assert_eq!(neutralize(input), expected, "{input:?}");
        assert_eq!(neutralize(expected), expected, "not idempotent: {input:?}");
    }
}

#[test]
fn serialized_notice_cap_counts_escaping_unicode_and_data() {
    let mut notice = Notice::new("key", Content::text(""), Attention::Wake);
    let overhead = serde_json::to_vec(&notice).unwrap().len();
    notice.content = Content::text("x".repeat(MAX_NOTICE_BYTES - overhead));
    assert_eq!(serde_json::to_vec(&notice).unwrap().len(), MAX_NOTICE_BYTES);
    check_notice_len(&notice).unwrap();
    notice.content = Content::text("x".repeat(MAX_NOTICE_BYTES - overhead + 1));
    assert_eq!(
        check_notice_len(&notice).unwrap_err().code,
        Code::ContentTooLarge
    );
    for text in ["é".repeat(8192), "\n".repeat(8192), "\"".repeat(8192)] {
        notice.content = Content::text(text);
        assert_eq!(
            check_notice_len(&notice).unwrap_err().code,
            Code::ContentTooLarge
        );
    }
    notice.content = Content::text("short");
    notice.data =
        Some(serde_json::from_value(json!({"large":"x".repeat(MAX_NOTICE_BYTES)})).unwrap());
    assert_eq!(
        check_notice_len(&notice).unwrap_err().code,
        Code::ContentTooLarge
    );
}

#[test]
fn link_limit_and_builders() {
    let link = Link {
        label: "result".into(),
        url: "https://example.test".into(),
    };
    let notice = Notice::new(
        "key",
        Content::template("echo", Default::default()),
        Attention::Ride,
    )
    .with_target_agent("agent")
    .with_replace_key("replace")
    .with_deliver_after_ms(2000)
    .with_expires_at_ms(3000)
    .with_data(Default::default())
    .with_links(vec![link.clone(); MAX_LINKS]);
    let mut request = PostRequest::new(Scope::new("flow", 1), notice);
    request.validate().unwrap();
    request.notice.links.as_mut().unwrap().push(link);
    assert_eq!(request.validate().unwrap_err().code, Code::InvalidParams);
    PostRequest::owner_notice(
        "flow",
        Notice::new("owner", Content::text("recovered"), Attention::Silent),
    )
    .validate()
    .unwrap();
    assert_eq!(
        serde_json::to_value(WithdrawRequest::replace("replace")).unwrap(),
        json!({"replace_key":"replace"})
    );
    assert_eq!(
        serde_json::to_value(WithdrawRequest::idempotency("key")).unwrap(),
        json!({"idempotency_key":"key"})
    );
}

#[test]
fn replies_are_open_and_buildable() {
    let reply: PostReply = serde_json::from_value(json!({"state":"future","new_field":7})).unwrap();
    assert_eq!(reply.state, PostState::Other("future".into()));
    assert_eq!(PostState::Accepted.as_str(), "accepted");
    assert_eq!(
        PostReply::new(PostState::Accepted)
            .with_item_id("one")
            .with_replayed(true),
        PostReply::accepted("one").with_replayed(true)
    );
    let refusal = Refusal::new(Code::from("future"), "future").with_detail(json!({"field":"new"}));
    assert!(refusal.is_terminal());
    round_trip::<Refusal>(&serde_json::to_value(refusal).unwrap());
    let item =
        Item::new("one", ItemState::Other("future".into()), "body").with_data(json!({"x":1}));
    round_trip::<ReadReply>(&serde_json::to_value(ReadReply::new(Some(item.clone()))).unwrap());
    round_trip::<ListReply>(&serde_json::to_value(ListReply::new(vec![item])).unwrap());
    let reply = WithdrawReply::new(WithdrawState::Other("future".into()));
    round_trip::<WithdrawReply>(&serde_json::to_value(reply).unwrap());
    let _: ReadRequest = round_trip(&serde_json::to_value(ReadRequest::new("one")).unwrap());
    for value in fixture("pull.json")["list_replies"].as_array().unwrap() {
        let mut value = value.clone();
        value["extra"] = json!(true);
        let _: ListReply = serde_json::from_value(value).unwrap();
    }
}
