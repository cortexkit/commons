use cortexkit_role_compaction::{
    notice::Conditions,
    read::Role,
    rules::{EditCost, ProviderRules, SignedParts},
    setup::{ModelRef, SetupRequest},
    view::{Edit, EditOp, InsertAt, InsertedImage, InsertedMessage, View},
    wire,
};
use serde_json::json;

fn image_message() -> InsertedMessage {
    InsertedMessage::new("visual-summary", Role::User, vec!["A visual summary.".into()])
        .with_images(vec![InsertedImage::new(
            "image/png",
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGNgAAIAAAUAAXpeqz8AAAAASUVORK5CYII=",
        )])
}

fn inserted_view(message: InsertedMessage) -> View {
    View::seal(
        1,
        vec![Edit::new(
            1,
            EditOp::Insert {
                at: InsertAt::Start,
                message,
            },
        )],
    )
}

#[test]
fn inserted_images_round_trip_with_required_strings() {
    let message = image_message();
    let raw = serde_json::to_value(&message).unwrap();
    assert_eq!(
        raw["images"],
        json!([{
            "media_type": "image/png",
            "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGNgAAIAAAUAAXpeqz8AAAAASUVORK5CYII=",
        }])
    );
    assert_eq!(
        wire::params::<InsertedMessage>(raw.clone()).unwrap(),
        message
    );

    let mut future = raw.clone();
    future["images"][0]["future_field"] = json!(true);
    assert_eq!(wire::params::<InsertedMessage>(future).unwrap(), message);

    for field in ["media_type", "data"] {
        let mut malformed = raw.clone();
        malformed["images"][0][field] = json!(42);
        assert_eq!(
            wire::params::<InsertedMessage>(malformed).unwrap_err().code,
            "invalid_params"
        );
        let mut missing = raw.clone();
        missing["images"][0].as_object_mut().unwrap().remove(field);
        assert!(wire::params::<InsertedMessage>(missing).is_err());
    }
}

#[test]
fn inserted_images_participate_in_view_checksum() {
    let view = inserted_view(image_message());
    assert!(view.verify());
    for (field, replacement) in [("data", "AA=="), ("media_type", "image/jpeg")] {
        let mut tampered = serde_json::to_value(&view).unwrap();
        tampered["edits"][0]["message"]["images"][0][field] = json!(replacement);
        let tampered: View = wire::params(tampered).unwrap();
        assert!(!tampered.verify(), "image {field} must be checksummed");
        let resealed = View::seal(tampered.version, tampered.edits);
        assert_ne!(view.checksum, resealed.checksum);
    }
}

#[test]
fn empty_inserted_images_hash_as_absent() {
    let message = InsertedMessage::new("summary", Role::User, vec!["A summary.".into()]);
    let view = inserted_view(message.clone());
    let absent = serde_json::to_value(&view).unwrap();
    assert!(absent["edits"][0]["message"].get("images").is_none());
    assert_eq!(
        inserted_view(message.with_images(Vec::new())).checksum,
        view.checksum
    );

    let mut explicit_empty = absent.clone();
    explicit_empty["edits"][0]["message"]["images"] = json!([]);
    let decoded: View = wire::params(explicit_empty).unwrap();
    assert!(decoded.verify());
    assert_eq!(decoded, view);
    assert_eq!(serde_json::to_value(decoded).unwrap(), absent);
}

#[test]
fn image_capability_preserves_unknown_true_and_false() {
    let model = ModelRef::new("example-provider", "example-model");
    let rules = ProviderRules::new(SignedParts::None, EditCost::Suffix);
    let setup = SetupRequest::new(
        "example",
        model.clone(),
        32_768,
        4_096,
        rules.clone(),
        vec![],
        1_700_000_000_000,
    );
    let conditions = Conditions::new(model, 32_768, 4_096, rules);
    for known in [None, Some(true), Some(false)] {
        let setup = match known {
            Some(value) => setup.clone().with_accepts_images(value),
            None => setup.clone(),
        };
        let conditions = match known {
            Some(value) => conditions.clone().with_accepts_images(value),
            None => conditions.clone(),
        };
        let setup_json = serde_json::to_value(&setup).unwrap();
        let conditions_json = serde_json::to_value(&conditions).unwrap();
        for raw in [&setup_json, &conditions_json] {
            assert_eq!(
                raw.get("accepts_images"),
                known.map(|value| json!(value)).as_ref()
            );
            assert!(raw["model"].get("accepts_images").is_none());
        }
        let decoded_setup: SetupRequest = wire::params(setup_json).unwrap();
        let decoded_conditions: Conditions = wire::params(conditions_json).unwrap();
        assert_eq!(decoded_setup.accepts_images, known);
        assert_eq!(decoded_conditions.accepts_images, known);
    }
}
