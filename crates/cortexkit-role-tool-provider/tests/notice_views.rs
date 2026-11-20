use cortexkit_role_tool_provider::views::{NoticeView, ViewEntry, ViewsReply, ViewsRequest};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../test-vectors/tool-provider-v1/notice-views.json"
    ))
    .unwrap()
}

#[test]
fn notice_views_round_trip_with_tool_views_and_bucket_keys() {
    let fixture = fixture();
    let request: ViewsRequest = serde_json::from_value(fixture["request"].clone()).unwrap();
    let reply: ViewsReply = serde_json::from_value(fixture["reply"].clone()).unwrap();
    reply.validate_for(&request).unwrap();
    reply.validate_writer_for(&request).unwrap();
    assert_eq!(serde_json::to_value(&reply).unwrap(), fixture["reply"]);
    assert!(matches!(
        reply.views["pin:ordinary:1"],
        ViewEntry::Manifest(_)
    ));
    assert_eq!(
        reply.views["nv1:example-writer:absent:1"],
        ViewEntry::Unknown
    );
    let publisher = fixture["publisher"].as_str().unwrap();
    for (pin, key) in fixture["bucket_keys"].as_object().unwrap() {
        let ViewEntry::Notice(view) = &reply.views[pin] else {
            panic!("{pin} is not a notice view")
        };
        view.check_pin(publisher, pin).unwrap();
        assert_eq!(view.bucket_key(publisher).unwrap(), *key);
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            fixture["reply"]["views"][pin]
        );
    }

    let display: Value =
        serde_json::from_str(cortexkit_role_tool_display::vectors::NOTICES_JSON).unwrap();
    for case in display["cases"].as_array().unwrap() {
        let served = &case["served"];
        let pin = fixture["notice_cases"][case["tag"].as_str().unwrap()]
            .as_str()
            .unwrap();
        assert_eq!(served["view"], pin);
        let ViewEntry::Notice(view) = &reply.views[pin] else {
            panic!("{pin} is not a notice view")
        };
        assert!(jsonschema::is_valid(
            &view.notice.data_schema,
            &served["data"]
        ));
    }
}

#[test]
fn notice_view_refusals_name_the_pin_or_offending_member() {
    let fixture = fixture();
    let sample = fixture["sample_pin"].as_str().unwrap();
    for case in fixture["refusals"].as_array().unwrap() {
        let pin = case["pin"].as_str().unwrap_or(sample);
        let mut entry = fixture["reply"]["views"][sample].clone();
        if let Some(member) = case["member"].as_str() {
            *entry.pointer_mut(member).unwrap() = case["value"].clone();
        }
        let reply: ViewsReply = serde_json::from_value(json!({"views": {pin: entry}})).unwrap();
        let request = ViewsRequest::new(vec![pin.into()]);
        let refusal = if let Some(publisher) = case["publisher"].as_str() {
            reply.validate_for(&request).unwrap();
            let ViewEntry::Notice(view) = &reply.views[pin] else {
                panic!("not a notice view")
            };
            view.check_pin(publisher, pin).unwrap_err()
        } else {
            reply.validate_for(&request).unwrap_err()
        };
        assert_eq!(refusal.code, "invalid_request", "{}", case["tag"]);
        assert_eq!(
            refusal.detail.unwrap()["field"],
            case["field"],
            "{}",
            case["tag"]
        );
    }

    let tool = &fixture["reply"]["views"]["pin:ordinary:1"];
    let reply: ViewsReply = serde_json::from_value(json!({"views": {sample: tool}})).unwrap();
    let refusal = reply
        .validate_for(&ViewsRequest::new(vec![sample.into()]))
        .unwrap_err();
    assert_eq!(refusal.detail.unwrap()["field"], format!("views.{sample}"));
    let view: NoticeView =
        serde_json::from_value(fixture["reply"]["views"][sample].clone()).unwrap();
    assert_eq!(
        view.bucket_key("bad:writer").unwrap_err().code,
        "invalid_request"
    );
}

#[test]
fn notice_views_require_wire_members_but_read_extensions() {
    let fixture = fixture();
    let sample = fixture["sample_pin"].as_str().unwrap();
    let entry = &fixture["reply"]["views"][sample];
    for case in fixture["malformed"].as_array().unwrap() {
        let mut entry = entry.clone();
        if let Some(removed) = case["remove"].as_str() {
            let (parent, name) = removed.rsplit_once('/').unwrap();
            entry
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(name)
                .unwrap();
        } else {
            *entry.pointer_mut(case["member"].as_str().unwrap()).unwrap() = case["value"].clone();
        }
        assert!(
            serde_json::from_value::<ViewEntry>(entry).is_err(),
            "{}",
            case["tag"]
        );
    }

    let mut extended = entry.clone();
    extended["extra_view_member"] = json!(true);
    extended["notice"]["extra_notice_member"] = json!(true);
    extended["template"]["extra_template_member"] = json!(true);
    let read: ViewEntry = serde_json::from_value(extended).unwrap();
    assert_eq!(serde_json::to_value(read).unwrap(), *entry);

    let tool = &fixture["reply"]["views"]["pin:ordinary:1"];
    let mut extended = tool.clone();
    extended["notice"] = json!({"future": "member"});
    let read: ViewEntry = serde_json::from_value(extended).unwrap();
    assert!(matches!(read, ViewEntry::Manifest(_)));
    assert_eq!(serde_json::to_value(read).unwrap(), *tool);
}
