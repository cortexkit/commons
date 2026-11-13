use cortexkit_role_compaction::{
    notice::{Event, NoticeRequest},
    provides_render_failed,
    view::ViewRefusal,
    COMPACTION_PROVIDES, COMPACTION_RENDER_FAILED_PROVIDES,
};
use serde_json::{json, Value};

fn round_trip_refusal(raw: Value) {
    let decoded: ViewRefusal = serde_json::from_value(raw.clone()).unwrap();
    assert!(
        !matches!(&decoded, ViewRefusal::Unknown { .. }),
        "render_failed must decode as a known refusal"
    );
    assert_eq!(serde_json::to_value(decoded).unwrap(), raw);

    let notice = json!({"event":"view_refused", "version":7, "refusal":raw});
    let decoded: Event = serde_json::from_value(notice.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), notice);
}

#[test]
fn render_failed_edit_round_trips_as_a_known_refusal() {
    round_trip_refusal(json!({"code":"render_failed", "mid":"message-a", "part":"part-a"}));
    for field in ["text", "output", "attachments", "delivered", "input.query"] {
        round_trip_refusal(json!({
            "code":"render_failed", "mid":"message-a", "part":"part-a", "field":field
        }));
    }
}

#[test]
fn render_failed_message_round_trips_as_a_known_refusal() {
    round_trip_refusal(json!({"code":"render_failed", "mid":"message-a"}));
}

#[test]
fn render_failed_unnamed_round_trips_as_a_known_refusal() {
    round_trip_refusal(json!({"code":"render_failed"}));
}

#[test]
fn render_failed_builders_emit_only_the_requested_locator() {
    for (refusal, raw) in [
        (
            ViewRefusal::render_failed(),
            json!({"code":"render_failed"}),
        ),
        (
            ViewRefusal::render_failed_message("message-a"),
            json!({"code":"render_failed", "mid":"message-a"}),
        ),
        (
            ViewRefusal::render_failed_edit("message-a", "part-a", None),
            json!({"code":"render_failed", "mid":"message-a", "part":"part-a"}),
        ),
        (
            ViewRefusal::render_failed_edit("message-a", "part-a", Some("output".into())),
            json!({"code":"render_failed", "mid":"message-a", "part":"part-a", "field":"output"}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&refusal).unwrap(), raw);
        assert_eq!(serde_json::from_value::<ViewRefusal>(raw).unwrap(), refusal);
    }
}

#[test]
fn render_failed_notice_is_projected_only_for_declaring_providers() {
    assert_eq!(
        COMPACTION_RENDER_FAILED_PROVIDES,
        "compaction-render-failed/v1"
    );
    let legacy: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/notice-notes-legacy-request.json"
    ))
    .unwrap();
    let notice: NoticeRequest = serde_json::from_value(legacy["params"].clone()).unwrap();
    for declared in [false, true] {
        let mut provides = vec![COMPACTION_PROVIDES.to_owned()];
        if declared {
            provides.push(COMPACTION_RENDER_FAILED_PROVIDES.to_owned());
        }
        assert_eq!(provides_render_failed(&provides), declared);
        for refusal in [
            ViewRefusal::render_failed(),
            ViewRefusal::render_failed_message("message-a"),
            ViewRefusal::render_failed_edit("message-a", "part-a", Some("attachments".into())),
        ] {
            let projected = notice.clone().with_view_refused(
                7,
                refusal.clone(),
                provides_render_failed(&provides),
            );
            assert_eq!(projected.events.len(), usize::from(declared));
            let encoded = serde_json::to_value(&projected).unwrap();
            if declared {
                assert_eq!(encoded["events"][0]["version"], 7);
                assert_eq!(
                    encoded["events"][0]["refusal"],
                    serde_json::to_value(&refusal).unwrap()
                );
            } else {
                assert_eq!(encoded, legacy["params"]);
            }
            let projected = projected.with_view_refused(8, ViewRefusal::BadChecksum, false);
            assert_eq!(projected.events.len(), usize::from(declared) + 1);
            assert!(matches!(
                projected.events.last(),
                Some(Event::ViewRefused {
                    version: 8,
                    refusal: ViewRefusal::BadChecksum
                })
            ));
        }
    }
    assert!(!provides_render_failed(&[]));
    assert!(!provides_render_failed(&[
        "compaction-render-failed/v2".into(),
        "compaction-render-failed/v1-extra".into()
    ]));
}

#[test]
fn render_failed_rejects_non_string_locators() {
    for field in ["mid", "part", "field"] {
        let mut raw =
            json!({"code":"render_failed", "mid":"message-a", "part":"part-a", "field":"output"});
        raw[field] = json!(42);
        assert!(serde_json::from_value::<ViewRefusal>(raw).is_err());
    }
}

#[test]
fn render_failed_notice_vectors_pin_served_state_shapes() {
    for (fixture, refusal) in [
        (
            include_str!("../test-vectors/compaction-v2/notice-render-failed-part-request.json"),
            json!({
                "code":"render_failed", "mid":"message-b", "part":"part-b",
                "field":"attachments"
            }),
        ),
        (
            include_str!("../test-vectors/compaction-v2/notice-render-failed-message-request.json"),
            json!({"code":"render_failed", "mid":"message-b"}),
        ),
        (
            include_str!(
                "../test-vectors/compaction-v2/notice-render-failed-unlocated-request.json"
            ),
            json!({"code":"render_failed"}),
        ),
    ] {
        let raw: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(
            raw["params"]["events"],
            json!([{"event":"view_refused", "version":7, "refusal":refusal}])
        );
        let notice: NoticeRequest = serde_json::from_value(raw["params"].clone()).unwrap();
        assert_eq!(serde_json::to_value(notice).unwrap(), raw["params"]);
    }
}
