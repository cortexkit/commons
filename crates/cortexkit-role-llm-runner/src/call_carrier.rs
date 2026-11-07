//! Shared decoding for the module id attached to an imported call.

use serde::{Deserialize, Deserializer};

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let module = String::deserialize(deserializer)?;
    if !crate::attribution::module_valid(&module) {
        return Err(serde::de::Error::custom("call_carrier: invalid module id"));
    }
    Ok(Some(module))
}

#[cfg(test)]
mod tests {
    use crate::{delivery_order, display, live, output_refs, read, rides, send, vectors};
    use serde::{de::DeserializeOwned, Serialize};
    use serde_json::{json, Value};

    fn check<T: DeserializeOwned + Serialize>(own: T, imported: T) {
        let own = serde_json::to_value(own).unwrap();
        assert!(own.get("call_carrier").is_none());
        let decoded: T = serde_json::from_value(own.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), own);
        let imported = serde_json::to_value(imported).unwrap();
        assert_eq!(imported["call_carrier"], "example-source-runner");
        let decoded: T = serde_json::from_value(imported.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), imported);
        for invalid in [
            json!(""),
            json!("Upper"),
            json!("../runner"),
            json!("a".repeat(65)),
            json!(null),
            json!(5),
            json!({"kind":"reserved","module_id":"runner"}),
        ] {
            let mut raw = own.clone();
            raw["call_carrier"] = invalid.clone();
            assert!(
                serde_json::from_value::<T>(raw).is_err(),
                "accepted {invalid}"
            );
        }
        for valid in ["0", "a_b-c", &"a".repeat(64)] {
            let mut raw = own.clone();
            raw["call_carrier"] = json!(valid);
            assert!(serde_json::from_value::<T>(raw).is_ok(), "rejected {valid}");
        }
    }

    #[test]
    fn call_carrier_reader_types_validate_and_build() {
        let carrier = "example-source-runner";
        let call =
            display::ToolCallPart::new("0", "call", "echo", &json!({}), None).with_call_key("key");
        check(call.clone(), call.with_call_carrier(carrier));
        let time = read::ToolTime::new("call").with_call_key("key");
        check(time.clone(), time.with_call_carrier(carrier));
        let attribution = read::ToolCallAttribution::new("call").with_call_key("key");
        check(attribution.clone(), attribution.with_call_carrier(carrier));
        let notice = read::NoticeFact::new(0, "send", "sender", "note").with_call_key("key");
        check(notice.clone(), notice.with_call_carrier(carrier));
        let started = live::ToolStarted::new("mid", "call", "echo", &json!({}), None, None)
            .with_call_key("key");
        check(started.clone(), started.with_call_carrier(carrier));
        let finished = live::ToolFinished::new("mid", "call", "ok", 0, None).with_call_key("key");
        check(finished.clone(), finished.with_call_carrier(carrier));
        let delivered =
            live::NoticeDelivered::new("mid", 0, "key", "send", "sender").with_text("note");
        check(delivered.clone(), delivered.with_call_carrier(carrier));
        let descriptor =
            output_refs::Descriptor::new("set", "mid", "dialect", &[]).with_call_key("key");
        check(descriptor.clone(), descriptor.with_call_carrier(carrier));
        let delivery = delivery_order::EntryDelivery::new(delivery_order::Mode::ToolResult)
            .with_call_key("key");
        check(delivery.clone(), delivery.with_call_carrier(carrier));
        let receipt = send::Delivered::new("step").with_call_key("key");
        check(receipt.clone(), receipt.with_call_carrier(carrier));
        let ride = rides::Ride::new("send", "sender", "key", "note");
        assert!(ride.call_carrier.is_none());
        assert_eq!(
            ride.with_call_carrier(carrier).call_carrier.as_deref(),
            Some(carrier)
        );
    }

    #[test]
    fn call_carrier_absent_preserves_existing_sample_bytes() {
        let bytes =
            br#"{"tool_call_id":"call","started_at":7,"call_key":"key","schema_pin":"pin"}"#;
        let time: read::ToolTime = serde_json::from_slice(bytes).unwrap();
        assert!(time.call_carrier.is_none());
        assert_eq!(serde_json::to_vec(&time).unwrap(), bytes);
        // Existing vectors are still read without adding an absent wire member.
        let file = vectors::load("call-identity.json");
        for case in vectors::cases(&file, "rows") {
            let _: display::DisplayRow = vectors::round_trip("existing row", &case["row"]);
        }
    }

    #[test]
    fn call_carrier_imported_vector_retains_key_on_call_and_notice() {
        let file = vectors::load("call-identity.json");
        let row: display::DisplayRow =
            vectors::round_trip("imported row", &file["imported_call"]["row"]);
        let notice: read::NoticeFact =
            vectors::round_trip("imported notice", &file["imported_call"]["notice"]);
        let display::DisplayPart::ToolCall(call) = &row.parts[0] else {
            panic!("missing tool call")
        };
        assert_eq!(call.call_key.as_deref(), Some("source-run.c0"));
        assert_eq!(notice.call_key, "source-run.c0");
        assert_eq!(call.call_carrier.as_deref(), Some("example-source-runner"));
        assert_eq!(
            notice.call_carrier.as_deref(),
            Some("example-source-runner")
        );
        let mut event = serde_json::to_value(live::LiveEvent::NoticeDelivered(
            live::NoticeDelivered::new("mid", 0, "key", "send", "sender")
                .with_call_carrier("example-source-runner"),
        ))
        .unwrap();
        event["call_carrier"] = Value::String("Bad module".into());
        assert!(live::LiveEvent::decode_strict(&event)
            .unwrap_err()
            .contains("call_carrier"));
        assert_eq!(
            serde_json::from_value::<live::LiveEvent>(event).unwrap(),
            live::LiveEvent::DisplayReset
        );
    }
}
