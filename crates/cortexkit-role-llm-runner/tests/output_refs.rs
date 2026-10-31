use cortexkit_role_llm_runner::{content_digest::sha256_hex, output_refs::*};
use serde_json::json;
use std::collections::BTreeMap;

fn request() -> BTreeMap<u64, Target> {
    BTreeMap::from([
        (1, Target::text("user", b"user text".to_vec())),
        (2, Target::text("assistant", b"model text".to_vec())),
        (
            3,
            Target::text("tool", b"stored output".to_vec()).with_tool("call", "read", "provider"),
        ),
        (
            4,
            Target::text("held", b"call again".to_vec())
                .with_tool("held-call", "read", "provider")
                .held(),
        ),
        (
            5,
            Target::text("large", vec![b'x'; MAX_SNAPSHOT_BYTES + 1])
                .with_tool("big-call", "read", "provider"),
        ),
    ])
}
fn mint() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("r{n}")
    }
}

#[test]
fn output_refs_scanner_contexts_and_unknown_targets() {
    let mut r = Resolution::new();
    r.reply("```\n$$tc(3)\n```\n`$$tc(3)` \\$$tc(3) <Chart title=\"$$tc(3)\" data={Parse($$tc(3), \"$$tc(3)\")} /> $$tc(1) $$tc(2) $$tc(99)", &request(), &mut mint());
    assert_eq!(r.records.len(), 4);
    assert_eq!(r.snapshots.len(), 1);
    assert_eq!(r.records[0].state, "resolved");
    assert_eq!(r.records[0].bytes, Some(13));
    assert_eq!(
        r.snapshots[&r.records[0].sha256.clone().unwrap()],
        b"stored output"
    );
    assert_eq!(r.records[1].reason.as_deref(), Some("not_a_tool_output"));
    assert_eq!(r.records[2].reason.as_deref(), Some("not_a_tool_output"));
    assert_eq!(r.records[3].reason.as_deref(), Some("unknown_tag"));
}

#[test]
fn output_refs_are_fixed_at_writing_request() {
    let mut request = request();
    let mut r = Resolution::new();
    r.reply("$$tc(3) $$tc(4)", &request, &mut mint());
    let before = canonical_records(&r.records);
    request.insert(3, Target::text("new", b"later output".to_vec()));
    request.get_mut(&4).unwrap().content = b"late answer".to_vec();
    assert_eq!(canonical_records(&r.records), before);
    assert_eq!(r.records[1].state, "held");
    assert_eq!(r.records[1].call_key.as_deref(), Some("held-call"));
    assert_eq!(
        r.snapshots[&r.records[1].sha256.clone().unwrap()],
        b"call again"
    );
}

#[test]
fn output_refs_caps_apply_across_parts() {
    let mut r = Resolution::new();
    let request = request();
    r.reply("$$tc(5)", &request, &mut mint());
    assert_eq!(r.records[0].state, "too_large");
    assert_eq!(r.records[0].bytes, Some((MAX_SNAPSHOT_BYTES + 1) as u64));
    assert_eq!(
        r.records[0].sha256.as_deref(),
        Some(sha256_hex(&vec![b'x'; MAX_SNAPSHOT_BYTES + 1]).as_str())
    );
    let mut mint = mint();
    r.reply(&"$$tc(3) ".repeat(63), &request, &mut mint);
    r.reply("$$tc(3)", &request, &mut mint);
    assert_eq!(r.records[64].reason.as_deref(), Some("too_many_refs"));
    let request = BTreeMap::from([(
        1,
        Target::text("part", vec![b'y'; MAX_SNAPSHOT_BYTES]).with_tool("c", "t", "p"),
    )]);
    let mut r = Resolution::new();
    r.reply(&"$$tc(1) ".repeat(5), &request, &mut mint);
    assert_eq!(
        r.records
            .iter()
            .map(|r| r.state.as_str())
            .collect::<Vec<_>>(),
        ["resolved", "resolved", "resolved", "resolved", "too_large"]
    );
    assert_eq!(r.snapshots.len(), 1);
}

#[test]
fn output_refs_only_declared_slots_and_opted_in_plan_tools() {
    let slots = [
        ReferenceSlot::new("data", "token"),
        ReferenceSlot::new("program", "source:rich-ui/1"),
    ];
    let arguments = json!({"data":"extra $$tc(3)", "program":"export const x = Parse($$tc(3), \"$$tc(1)\")", "other":"$$tc(3)"});
    let mut r = Resolution::new();
    r.call(&arguments, &slots, false, &request(), &mut mint());
    assert!(r.records.is_empty());
    r.call(&arguments, &slots, true, &request(), &mut mint());
    assert_eq!(r.records.len(), 2);
    assert_eq!(r.records[0].reason.as_deref(), Some("not_a_reference"));
    assert_eq!(r.records[1].tag.as_deref(), Some("3"));
    assert_eq!(r.records[1].state, "resolved");
    let mut r = Resolution::new();
    r.reply(
        "$$tc(99999999999999999999999999999999)",
        &request(),
        &mut mint(),
    );
    assert_eq!(r.records[0].reason.as_deref(), Some("unknown_tag"));
}

#[test]
fn output_refs_descriptor_paging_and_read_limits() {
    let records: Vec<_> = (0..70)
        .map(|i| Record::unresolved(format!("r{i}"), Some("7".into()), "unknown_tag"))
        .collect();
    let descriptor = Descriptor::new("set", "mid", "rich-ui/1", &records);
    assert!(descriptor.check(&records));
    let oversized = Descriptor::new("s".repeat(4096), "m", "rich-ui/1", &records);
    assert!(!oversized.check(&records));
    let mut changed = records.clone();
    changed[0].tag = Some("8".into());
    assert!(!descriptor.check(&changed));
    let mut cursor = None;
    let mut got = Vec::new();
    loop {
        let page = Page::from_records(&records, cursor.as_deref()).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= MAX_PAGE_BYTES);
        assert!(page.records.len() <= MAX_PAGE_RECORDS);
        got.extend(page.records);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(got, records);
    assert_eq!(
        Page::from_records(&records, Some("foreign")).unwrap_err(),
        "unknown_ref"
    );
    assert!(ReadRequest::new("r", 10, 0).check(10).is_ok());
    assert_eq!(ReadRequest::new("r", 11, 0).check(10), Err("offset"));
    assert_eq!(
        ReadRequest::new("r", 0, MAX_READ_BYTES + 1).check(10),
        Err("length")
    );
}

#[test]
fn output_refs_plan_refusal_and_group_dependency() {
    for flag in [json!(true), json!(false), json!(null), json!("yes")] {
        let plan = json!({"tool_items":[{"output_refs":flag}]});
        assert_eq!(check_plan(&plan, false), Err(PLAN_FIELD));
        assert_eq!(check_plan(&plan, true).is_ok(), flag.is_boolean());
        let plan = plan.as_object().unwrap().clone();
        assert_eq!(
            cortexkit_role_llm_runner::send::SendRequest::new("p", "s")
                .with_plan(plan.clone())
                .check_output_refs(false),
            Err(PLAN_FIELD)
        );
        assert_eq!(
            cortexkit_role_llm_runner::adopt::AdoptRequest::new("a", plan.clone())
                .check_output_refs(false),
            Err(PLAN_FIELD)
        );
        assert_eq!(
            cortexkit_role_llm_runner::session_change::RefreshRequest::new(plan, "immediately", 1)
                .check_output_refs(false),
            Err(PLAN_FIELD)
        );
    }
    assert!(check_plan(&json!({"tool_items":[{}]}), false).is_ok());
    use cortexkit_role_llm_runner::{
        capabilities,
        describe::{check_describe, Major, RoleDescribe},
        ops, PROVIDES,
    };
    let ops = vec![
        ops::ROLE_DESCRIBE,
        ops::SESSION_BASELINE,
        ops::SESSION_OUTPUT_REFS,
        ops::SESSION_OUTPUT_REF_READ,
    ];
    let describe = RoleDescribe::new(
        vec![Major::new(
            PROVIDES,
            ops.into_iter().map(str::to_owned).collect(),
            "alpha",
        )],
        "test",
    )
    .with_capabilities(vec![capabilities::OUTPUT_REFS.into()]);
    assert!(check_describe(&serde_json::to_value(describe).unwrap())
        .unwrap_err()
        .iter()
        .any(|p| format!("{p:?}").contains("output_refs requires tags")));
}

#[test]
fn output_refs_shared_scanner_samples() {
    for samples in [
        cortexkit_rich_ui_syntax::vectors::ORIGINAL_SAMPLES,
        cortexkit_rich_ui_syntax::vectors::EXTENDED_SAMPLES,
    ] {
        let samples: serde_json::Value = serde_json::from_str(samples).unwrap();
        for case in samples["cases"].as_array().unwrap() {
            let mut r = Resolution::new();
            let source = case["source"].as_str().unwrap();
            let mode = case["mode"].as_str().unwrap();
            if mode == "reply" {
                r.reply(source, &BTreeMap::new(), &mut mint());
            } else {
                r.call(
                    &json!({"data":source}),
                    &[ReferenceSlot::new(
                        "data",
                        if mode == "token_slot" {
                            "token"
                        } else {
                            "source:rich-ui/1"
                        },
                    )],
                    true,
                    &BTreeMap::new(),
                    &mut mint(),
                );
            }
            let got: Vec<_> = r.records.iter().filter_map(|r| r.tag.clone()).collect();
            let expected: Vec<_> = case["references"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| match &r["n"] {
                    serde_json::Value::String(s) => s.clone(),
                    n => n.to_string(),
                })
                .collect();
            assert_eq!(got, expected, "{}", case["name"]);
        }
    }
}

#[cfg(feature = "test-vectors")]
#[test]
fn output_refs_canonical_vector() {
    let v: serde_json::Value =
        serde_json::from_str(cortexkit_role_llm_runner::vectors::file("output-refs.json").unwrap())
            .unwrap();
    let records: Vec<Record> = serde_json::from_value(v["records"].clone()).unwrap();
    let descriptor: Descriptor = serde_json::from_value(v["descriptor"].clone()).unwrap();
    assert_eq!(
        String::from_utf8(canonical_records(&records)).unwrap(),
        v["canonical_records"]
    );
    assert!(descriptor.check(&records));
    assert_eq!(serde_json::to_value(&records).unwrap(), v["records"]);
}

#[test]
fn output_refs_pinned_limits() {
    assert_eq!(MAX_REFERENCES, 64);
    assert_eq!(MAX_SNAPSHOT_BYTES, 1_048_576);
    assert_eq!(MAX_SET_BYTES, 4_194_304);
    assert_eq!(MAX_DESCRIPTOR_BYTES, 4_096);
    assert_eq!(MAX_PAGE_RECORDS, 32);
    assert_eq!(MAX_PAGE_BYTES, 16_384);
    assert_eq!(MAX_READ_BYTES, 131_072);
}

#[test]
fn output_refs_page_byte_limit() {
    let records: Vec<_> = (0..32)
        .map(|i| {
            let mut record = Record::unresolved(format!("r{i}"), Some("7".into()), "unknown_tag");
            record.part = Some("p".repeat(2_000));
            record
        })
        .collect();
    let page = Page::from_records(&records, None).unwrap();
    assert!(!page.records.is_empty());
    assert!(page.records.len() < 32);
    assert!(page.next.is_some());
    assert!(serde_json::to_vec(&page).unwrap().len() <= 16_384);
}

#[test]
fn output_refs_image_snapshot_is_stored_bytes() {
    let bytes = vec![0, 255, 137, 80, 78, 71];
    let request = BTreeMap::from([(
        7,
        Target::text("image", bytes.clone())
            .with_tool("c", "t", "p")
            .with_image("image/png"),
    )]);
    let mut r = Resolution::new();
    r.reply("$$tc(7)", &request, &mut mint());
    let record = &r.records[0];
    assert_eq!(record.state, "resolved");
    assert_eq!(record.format.as_deref(), Some("image"));
    assert_eq!(record.mime.as_deref(), Some("image/png"));
    assert_eq!(record.bytes, Some(6));
    assert_eq!(r.snapshots[record.sha256.as_ref().unwrap()], bytes);
}

#[test]
fn output_refs_is_checked_on_every_item_list_of_a_composer_shaped_plan() {
    for (plan, field) in [
        (json!({"tool_items":[{}, {"output_refs":true}]}), PLAN_FIELD),
        (
            json!({"tool_items":[{}], "system_text_items":[{"output_refs":false}]}),
            SYSTEM_TEXT_PLAN_FIELD,
        ),
        (
            json!({"step_transform_items":[{"output_refs":true}]}),
            STEP_TRANSFORM_PLAN_FIELD,
        ),
        (
            json!({"compaction_item":{"provider":"example.compaction","output_refs":false}}),
            COMPACTION_PLAN_FIELD,
        ),
    ] {
        assert_eq!(check_plan(&plan, false), Err(field), "{plan}");
        assert!(check_plan(&plan, true).is_ok(), "{plan}");
    }
    let malformed = json!({"system_text_items":[{"output_refs":"yes"}]});
    assert_eq!(check_plan(&malformed, true), Err(SYSTEM_TEXT_PLAN_FIELD));
}
