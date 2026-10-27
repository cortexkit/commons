//! `system_text.rich_ui`: the optional rich UI stamp on a system-text answer.

use cortexkit_role_tool_provider::{
    catalog::{system_text_digest, SystemTextAnswer},
    rich_ui::RichUiStamp,
    vectors,
};
use serde_json::Value;

fn cases() -> Value {
    serde_json::from_str(vectors::file("rich-ui-stamp.json").unwrap()).unwrap()
}

#[test]
fn answers_round_trip_and_an_unstamped_answer_carries_no_member() {
    for case in cases()["answers"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let wire = &case["system_text"];
        let answer: SystemTextAnswer = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(
            answer.item_digest,
            system_text_digest(answer.text.as_deref().unwrap()),
            "{name}"
        );
        assert_eq!(
            &serde_json::to_value(&answer).unwrap(),
            wire,
            "{name}: re-encoding changes the answer"
        );
        match &case["rich_ui"] {
            Value::Null => {
                assert_eq!(answer.rich_ui, None, "{name}");
                assert!(!wire.as_object().unwrap().contains_key("rich_ui"), "{name}");
            }
            stamp => {
                let expected: RichUiStamp = serde_json::from_value(stamp.clone()).unwrap();
                assert_eq!(answer.rich_ui.as_ref(), Some(&expected), "{name}");
            }
        }
    }
}

#[test]
fn the_builder_adds_only_the_stamp() {
    let answers = cases();
    let unstamped: SystemTextAnswer =
        serde_json::from_value(answers["answers"][0]["system_text"].clone()).unwrap();
    let stamp: RichUiStamp =
        serde_json::from_value(answers["answers"][1]["rich_ui"].clone()).unwrap();
    let stamped = unstamped.with_rich_ui(stamp);
    assert_eq!(
        serde_json::to_value(stamped).unwrap(),
        answers["answers"][1]["system_text"]
    );
}

#[test]
fn valid_stamps_decode_and_re_encode_their_two_members() {
    for case in cases()["valid"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let stamp: RichUiStamp = serde_json::from_value(case["rich_ui"].clone())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let expected = case.get("encoded").unwrap_or(&case["rich_ui"]);
        assert_eq!(&serde_json::to_value(&stamp).unwrap(), expected, "{name}");
        assert_eq!(
            RichUiStamp::new(stamp.dialect(), stamp.catalog_digest()).as_ref(),
            Ok(&stamp),
            "{name}"
        );
    }
}

#[test]
fn malformed_stamps_are_refused_naming_the_field() {
    for case in cases()["refused"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let field = case["field"].as_str().unwrap();
        let error = serde_json::from_value::<RichUiStamp>(case["rich_ui"].clone())
            .expect_err(name)
            .to_string();
        assert!(
            error.contains(&format!("rich_ui.{field}")),
            "{name}: {error}"
        );

        // The same refusal reaches a whole system-text answer.
        let mut answer = cases()["answers"][0]["system_text"].clone();
        answer["rich_ui"] = case["rich_ui"].clone();
        let error = serde_json::from_value::<SystemTextAnswer>(answer)
            .expect_err(name)
            .to_string();
        assert!(
            error.contains(&format!("rich_ui.{field}")),
            "{name}: {error}"
        );
    }
}
