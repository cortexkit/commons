use cortexkit_role_tool_provider::{
    describe::{check_describe, Major, RoleDescribe},
    ops,
    views::{ViewChange, ViewEntry, ViewManifest, ViewRead, ViewsReply, ViewsRequest},
    PROVIDES, TOOL_VIEWS_CAPABILITY,
};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../test-vectors/tool-provider-v1/views.json")).unwrap()
}

#[test]
fn display_and_provider_view_vectors_are_identical() {
    assert_eq!(
        include_str!("../test-vectors/tool-provider-v1/views.json"),
        cortexkit_role_tool_display::vectors::VIEWS_JSON
    );
}

#[test]
fn shared_views_vectors_validate_and_round_trip() {
    let fixture = fixture();
    let request: ViewsRequest = serde_json::from_value(fixture["request"].clone()).unwrap();
    request.validate().unwrap();
    let reply: ViewsReply = serde_json::from_value(fixture["reply"].clone()).unwrap();
    reply.validate_for(&request).unwrap();
    assert_eq!(serde_json::to_value(&reply).unwrap(), fixture["reply"]);
    assert!(matches!(reply.views["pin:missing"], ViewEntry::Unknown));
    for case in fixture["invalid_requests"].as_array().unwrap() {
        let request: ViewsRequest = serde_json::from_value(case.clone()).unwrap();
        let error = request.validate().unwrap_err();
        assert_eq!(error.code, "invalid_request");
        assert_eq!(error.detail.unwrap()["field"], case["field"]);
    }
    for case in fixture["invalid_replies"].as_array().unwrap() {
        if let Ok(reply) = serde_json::from_value::<ViewsReply>(case.clone()) {
            assert!(reply.validate_for(&request).is_err(), "{case}");
        }
    }
}

#[test]
fn pin_count_and_opaque_bounds_are_enforced() {
    ViewsRequest::new(vec![]).validate().unwrap();
    ViewsRequest::new(vec!["pin:1".into(); 64])
        .validate()
        .unwrap();
    let error = ViewsRequest::new(vec!["pin:1".into(); 65])
        .validate()
        .unwrap_err();
    assert_eq!(error.detail.unwrap()["field"], "pins");
    assert!(ViewsRequest::new(vec!["x".repeat(257)]).validate().is_err());
}

#[test]
fn views_are_opt_in_without_changing_required_ops_or_existing_shapes() {
    let major = Major {
        version: PROVIDES.into(),
        ops: vec![ops::ROLE_DESCRIBE.into(), ops::TOOL_CATALOG.into()],
        stability: "alpha".into(),
    };
    let old = RoleDescribe::new(vec![major.clone()], "1");
    check_describe(&serde_json::to_value(&old).unwrap()).unwrap();
    let mut new = old
        .clone()
        .with_capabilities(vec![TOOL_VIEWS_CAPABILITY.into()]);
    new.majors[0].ops.push(ops::TOOL_VIEWS.into());
    check_describe(&serde_json::to_value(&new).unwrap()).unwrap();
    assert_eq!(
        cortexkit_role_tool_provider::REQUIRED_OPS,
        &["role.describe", "tool.catalog"]
    );
    assert_eq!(
        serde_json::to_value(old).unwrap(),
        json!({
            "majors":[{"version":"tool-provider/v1","ops":["role.describe","tool.catalog"],"stability":"alpha"}],
            "implementation_version":"1", "capabilities":[]
        })
    );
}

#[test]
fn constructors_preserve_read_extensions_and_opaque_templates() {
    let fixture = fixture();
    let schema =
        serde_json::from_value(fixture["reply"]["views"]["pin:command:1"]["record_schema"].clone())
            .unwrap();
    let mut read = ViewRead::new("command.output");
    read.extensions
        .insert("future".into(), json!({"opaque":true}));
    let manifest = ViewManifest::new(schema, vec![read], json!([null, "opaque", {"unknown":2}]));
    let value = serde_json::to_value(&manifest).unwrap();
    assert_eq!(value["reads"][0]["future"], json!({"opaque":true}));
    let reply = ViewsReply::new([("pin:1".into(), ViewEntry::Manifest(manifest))].into());
    reply
        .validate_for(&ViewsRequest::new(vec!["pin:1".into()]))
        .unwrap();
    assert_eq!(
        serde_json::from_value::<ViewsReply>(serde_json::to_value(&reply).unwrap()).unwrap(),
        reply
    );
}

#[test]
fn template_revisions_replace_only_the_template() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../test-vectors/tool-provider-v1/view-revisions.json"
    ))
    .unwrap();
    let base: ViewManifest = serde_json::from_value(fixture["kept"].clone()).unwrap();
    assert_eq!(base.template_revision, 0);
    // Revision 0 is omitted, so a view without revisions has the same members.
    assert_eq!(serde_json::to_value(&base).unwrap(), fixture["kept"]);
    for case in fixture["cases"].as_array().unwrap() {
        let fetched: ViewManifest = serde_json::from_value(case["fetched"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&fetched).unwrap(), case["fetched"]);
        let kept = base
            .clone()
            .with_template_revision(case["kept_revision"].as_u64().unwrap_or(0));
        let expected = match case["change"].as_str().unwrap() {
            "same" => ViewChange::Same,
            "newer_template" => ViewChange::NewerTemplate,
            "stale" => ViewChange::Stale,
            "conflict" => ViewChange::Conflict,
            other => panic!("unknown change {other}"),
        };
        assert_eq!(
            fetched.compare_with_kept(&kept),
            expected,
            "{}",
            case["name"]
        );
    }
}
