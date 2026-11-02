use std::fmt::Debug;

use cortexkit_role_compaction::{
    notice::{Conditions, Event, NoticeRequest},
    part::{Delivered, FileMedia, Opaque, Part, PartBody, ToolPart},
    read::{Cursor, Entry, ReadPage, ReadRequest, StepUsage},
    ride::RideRequest,
    setup::SetupRequest,
    view::{AppliedView, Edit, View, ViewRefusal},
    wire,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

fn boundaries<T: DeserializeOwned + Debug>(mut value: Value, fields: &[&str]) {
    for field in fields {
        let original = value[*field].clone();
        value[*field] = json!(9_007_199_254_740_991_u64);
        wire::params::<T>(value.clone()).unwrap();
        value[*field] = json!(9_007_199_254_740_992_u64);
        let refusal = wire::params::<T>(value.clone()).unwrap_err();
        assert_eq!(refusal.code, "invalid_params");
        assert_eq!(refusal.detail, Some(json!({"field": field})));
        value[*field] = original;
    }
}

#[test]
fn protocol_integer_boundaries() {
    let model = json!({"provider": "example-provider", "model": "example-model"});
    let rules = json!({"signed_parts": "none", "history_edit_cost": "suffix"});
    let conditions =
        json!({"model": model, "rules": rules, "context_window": 100, "output_reserve": 10});
    let part = json!({"id": "part-a", "kind": "text", "text": "Example.", "digest": "example"});
    boundaries::<Conditions>(
        conditions.clone(),
        &["context_window", "output_reserve", "last_prompt_tokens"],
    );
    boundaries::<SetupRequest>(
        json!({"preset": "example", "model": model, "rules": rules, "context_window": 100, "output_reserve": 10, "detects": [], "now": 0}),
        &["context_window", "output_reserve", "now"],
    );
    boundaries::<NoticeRequest>(json!({"seq": 1, "conditions": conditions}), &["seq"]);
    boundaries::<RideRequest>(
        json!({"causes": ["over_limit"], "deadline_ms": 10, "conditions": conditions}),
        &["deadline_ms", "request_tokens"],
    );
    boundaries::<Cursor>(
        json!({"ordinal": 1, "mid": "message-a", "digest": "example"}),
        &["ordinal"],
    );
    boundaries::<Entry>(
        json!({"ordinal": 1, "mid": "message-a", "role": "user", "written_at": 0, "digest": "example", "parts": [part]}),
        &["ordinal", "written_at"],
    );
    boundaries::<Part>(part, &["tag"]);
    boundaries::<StepUsage>(
        json!({}),
        &[
            "prompt_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
            "output_tokens",
            "reasoning_tokens",
        ],
    );
    boundaries::<ToolPart>(
        json!({"call_id": "call-a", "name": "example-tool", "input": {}, "status": "running"}),
        &["output_bytes"],
    );
    boundaries::<Delivered>(
        json!({"sender": "example-sender", "text_bytes": 0}),
        &["text_bytes"],
    );
    boundaries::<FileMedia>(
        json!({"mime": "image/png", "transport": "inline"}),
        &["size", "width", "height", "duration_ms"],
    );
    boundaries::<Opaque>(json!({"provider_kind": "example-provider.item"}), &["size"]);
    boundaries::<View>(
        json!({"version": 1, "checksum": "example", "edits": []}),
        &["version"],
    );
    boundaries::<AppliedView>(json!({"version": 1, "checksum": "example"}), &["version"]);
    boundaries::<Edit>(
        json!({"since": 1, "op": "drop", "mid": "message-a"}),
        &["since"],
    );
    boundaries::<PartBody>(json!({"kind": "reasoning"}), &["text_bytes"]);
    boundaries::<Event>(
        json!({"event": "transcript_changed", "from_ordinal": 1}),
        &["from_ordinal"],
    );
    boundaries::<Event>(
        json!({"event": "marker_written", "version": 1}),
        &["version"],
    );
    boundaries::<Event>(
        json!({"event": "view_refused", "version": 1, "refusal": {"code": "bad_checksum"}}),
        &["version"],
    );
    boundaries::<ViewRefusal>(
        json!({"code": "stale_version", "version": 1, "latest": 2}),
        &["version", "latest"],
    );
    boundaries::<ViewRefusal>(
        json!({"code": "foreign_tag", "mid": "message-a", "part": "part-a", "field": "text", "tag": 1}),
        &["tag"],
    );
    boundaries::<ViewRefusal>(json!({"code": "unsupported_edit", "index": 0}), &["index"]);
    boundaries::<ReadPage>(json!({"entries": [], "head": {}}), &["next_after"]);
    boundaries::<ReadRequest>(
        json!({"select": {"by": "after"}, "detail": "compact"}),
        &["max_bytes"],
    );

    for (select, field) in [
        (
            json!({"by": "after", "after_ordinal": 9_007_199_254_740_992_u64}),
            "select.after_ordinal",
        ),
        (
            json!({"by": "after", "limit": 9_007_199_254_740_992_u64}),
            "select.limit",
        ),
        (
            json!({"by": "tags", "tags": [9_007_199_254_740_992_u64]}),
            "select.tags[0]",
        ),
    ] {
        let refusal = wire::params::<ReadRequest>(json!({"select": select, "detail": "compact"}))
            .unwrap_err();
        assert_eq!(refusal.detail, Some(json!({"field": field})));
    }
}

#[test]
fn read_limit_must_be_positive() {
    for select in [json!({"by": "after"}), json!({"by": "after", "limit": 1})] {
        wire::params::<ReadRequest>(json!({"select": select, "detail": "compact"})).unwrap();
    }
    let refusal = wire::params::<ReadRequest>(
        json!({"select": {"by": "after", "limit": 0}, "detail": "compact"}),
    )
    .unwrap_err();
    assert_eq!(refusal.code, "invalid_params");
    assert_eq!(refusal.detail, Some(json!({"field": "select.limit"})));
}

#[test]
fn opaque_json_numbers_are_not_protocol_integers() {
    let payload = json!({"large": u64::MAX, "negative": -2, "fraction": 1.25});
    let tool: ToolPart = wire::params(json!({"call_id": "call-a", "name": "example-tool", "input": payload, "status": "running", "future_count": u64::MAX})).unwrap();
    assert_eq!(tool.input, payload);
    let part: PartBody = wire::params(json!({"kind": "future_kind", "count": u64::MAX})).unwrap();
    assert_eq!(
        serde_json::to_value(part).unwrap(),
        json!({"kind": "future_kind", "count": u64::MAX})
    );
    let edit: Edit = wire::params(json!({"since": 1, "op": "replace", "mid": "message-a", "part": "part-a", "field": "delivered", "value": payload, "expect": "example"})).unwrap();
    assert_eq!(serde_json::to_value(edit).unwrap()["value"], payload);
}
