use cortexkit_role_tool_display::{
    notice_idempotency_key, notice_view_key, parse_notice_view_key, write_outcome, NoticeRecord,
    NoticeWrite, Principal, WriteReply, DISPLAY_WRITER_UNVERIFIED, DISPLAY_WRITE_CONFLICT,
    NOTICE_RECORD_DISPLAY_CAPABILITY, NOTICE_RECORD_WRITER_CAPABILITY,
};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../test-vectors/tool-display-v1/notices.json")).unwrap()
}

fn modified_write(base: &Value, case: &Value) -> Value {
    let Some(pointer) = case["pointer"].as_str() else {
        return base.clone();
    };
    let mut write = base.clone();
    if let Some(append) = case["append"].as_str() {
        let target = write.pointer_mut(pointer).unwrap();
        *target = json!(format!("{}{append}", target.as_str().unwrap()));
    } else {
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        write.pointer_mut(parent).unwrap()[field] = case["value"].clone();
    }
    write
}

#[test]
fn notice_records_round_trip_writes_served_shapes_and_fold_pair() {
    let fixture = fixture();
    assert_eq!(NOTICE_RECORD_WRITER_CAPABILITY, fixture["tokens"]["writer"]);
    assert_eq!(
        NOTICE_RECORD_DISPLAY_CAPABILITY,
        fixture["tokens"]["display"]
    );
    let writer = serde_json::from_value(fixture["writer"].clone()).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    for case in cases {
        let write: NoticeWrite = serde_json::from_value(case["write"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&write).unwrap(), case["write"]);
        write.check(&writer).unwrap();
        let record = NoticeRecord::from_body(&writer, write.body).unwrap();
        record.check().unwrap();
        let expected: NoticeRecord = serde_json::from_value(case["served"].clone()).unwrap();
        assert_eq!(record, expected);
        assert_eq!(serde_json::to_value(record).unwrap(), case["served"]);
    }
    let find = |which: &str| {
        cases
            .iter()
            .find(|case| case["write"]["send_id"] == fixture["fold"][which])
            .unwrap()
    };
    assert_eq!(
        find("earlier")["served"]["sender"],
        find("later")["served"]["sender"]
    );
    assert_eq!(
        find("earlier")["served"]["key"],
        find("later")["served"]["key"]
    );
    assert_ne!(
        find("earlier")["write"]["send_id"],
        find("later")["write"]["send_id"]
    );
    assert_eq!(find("visible")["tag"], "fold_later");

    let record_key_only = cases
        .iter()
        .find(|case| case["tag"] == "record_key_without_send")
        .unwrap();
    assert_eq!(record_key_only["record_key_only"], json!(true));
    assert_eq!(record_key_only["write"]["send_id"], "record-background-01");

    let cap: NoticeWrite = serde_json::from_value(fixture["cap_write"].clone()).unwrap();
    assert_eq!(cap.check(&writer).unwrap().len(), 16_384);
    let record = NoticeRecord::from_body(&writer, cap.body).unwrap();
    assert_eq!(
        parse_notice_view_key(&record.view).unwrap().2,
        9_007_199_254_740_991
    );
}

#[test]
fn notice_write_refusals_match_vectors() {
    let fixture = fixture();
    let base = &fixture["cases"][0]["write"];
    for case in fixture["refusals"].as_array().unwrap() {
        let writer =
            serde_json::from_value(case.get("writer").unwrap_or(&fixture["writer"]).clone())
                .unwrap();
        let base = case["base"].as_str().map_or(base, |key| &fixture[key]);
        let write: NoticeWrite = serde_json::from_value(modified_write(base, case)).unwrap();
        let refusal = write.check(&writer).unwrap_err();
        assert_eq!(refusal.code, case["code"], "{}", case["tag"]);
        assert_eq!(
            refusal.detail.as_ref(),
            case.get("detail"),
            "{}",
            case["tag"]
        );
    }
    for case in fixture["malformed"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<NoticeWrite>(modified_write(base, case)).is_err(),
            "{}",
            case["tag"]
        );
    }
}

#[test]
fn notice_retries_use_canonical_bytes_and_attested_storage_key() {
    let fixture = fixture();
    let writer = serde_json::from_value(fixture["writer"].clone()).unwrap();
    let mut write: NoticeWrite =
        serde_json::from_value(fixture["cases"][0]["write"].clone()).unwrap();
    let first = write.check(&writer).unwrap();
    assert_eq!(first, br#"{"data":{"exit_code":0,"status":"finished","title":"Build example"},"key":"task-01","view":"finished:1"}"#);
    let reordered: NoticeWrite = serde_json::from_str(r#"{
        "send_id":"send-finished","session":"session-example","kind":"notice",
        "body":{"view":"finished:1","data":{"title":"Build example","status":"finished","exit_code":0},"key":"task-01"},
        "runner":{"kind":"reserved","module_id":"example-runner"}
    }"#).unwrap();
    let retry = reordered.check(&writer).unwrap();
    assert_eq!(write_outcome(None, &first).unwrap(), WriteReply::Stored);
    assert_eq!(
        write_outcome(Some(&first), &retry).unwrap(),
        WriteReply::AlreadyStored
    );

    let key = notice_idempotency_key(&writer, &write).unwrap();
    assert_eq!(
        key,
        br#"["session-example",{"kind":"reserved","module_id":"example-writer"},"send-finished"]"#
    );
    write.runner = Principal::Direct;
    write.body.data.insert("exit_code".into(), json!(1));
    let changed = write.check(&writer).unwrap();
    assert_eq!(
        write_outcome(Some(&first), &changed).unwrap_err().code,
        DISPLAY_WRITE_CONFLICT
    );
    assert_eq!(notice_idempotency_key(&writer, &write).unwrap(), key);
    let mut moved = write.clone();
    moved.session = "session-moved".into();
    assert_ne!(notice_idempotency_key(&writer, &moved).unwrap(), key);
    let other = Principal::Reserved {
        module_id: "other-writer".into(),
    };
    assert_ne!(notice_idempotency_key(&other, &write).unwrap(), key);
    write.send_id = "different-send".into();
    assert_ne!(notice_idempotency_key(&writer, &write).unwrap(), key);
}

#[test]
fn notice_served_record_cannot_claim_another_writers_view() {
    let fixture = fixture();
    let mut record: NoticeRecord =
        serde_json::from_value(fixture["cases"][0]["served"].clone()).unwrap();
    let write: NoticeWrite = serde_json::from_value(fixture["cases"][0]["write"].clone()).unwrap();
    assert_eq!(
        NoticeRecord::from_body(&Principal::Direct, write.body)
            .unwrap_err()
            .code,
        DISPLAY_WRITER_UNVERIFIED
    );
    record.view = "nv1:other-writer:finished:1".into();
    assert_eq!(record.check().unwrap_err().field, "view");
    assert!(notice_view_key("bad:writer", "finished", 1).is_none());
    assert!(notice_view_key("bad writer", "finished", 1).is_none());
    assert!(notice_view_key("", "finished", 1).is_none());
}
