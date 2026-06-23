//! Lock the derivation helpers to the committed cross-language golden vectors.
//!
//! `tests/golden/storage_vectors.json` is the canonical contract the TS lib
//! (@cortexkit/store) also asserts against. If a change here alters a database
//! name, store path, or descriptor shape, this fails — and that change is then a
//! deliberate cross-language wire break (regenerate the fixture via
//! `cargo run -p cortexkit-store-types --example golden-vectors` and update both
//! sides), never an accidental drift that silently lands a TS module on a
//! different database than the Rust resolver.

use cortexkit_store_types::{postgres_database_name, sqlite_store_path, StorageDescriptor};
use serde_json::Value;

const VECTORS: &str = include_str!("golden/storage_vectors.json");

#[test]
fn helpers_reproduce_the_golden_vectors() {
    let doc: Value = serde_json::from_str(VECTORS).expect("parse golden vectors");
    let data_home = doc["data_home"].as_str().expect("data_home");
    let vectors = doc["vectors"].as_array().expect("vectors array");
    assert!(!vectors.is_empty(), "golden vectors must not be empty");

    for v in vectors {
        let id = v["module_id"].as_str().expect("module_id");

        assert_eq!(
            postgres_database_name(id),
            v["postgres_database_name"].as_str().unwrap(),
            "postgres_database_name drift for module_id {id}"
        );
        assert_eq!(
            sqlite_store_path(data_home, id),
            v["sqlite_store_path"].as_str().unwrap(),
            "sqlite_store_path drift for module_id {id}"
        );

        // The descriptor shape (field names + tags) is part of the wire contract.
        let descriptor: StorageDescriptor =
            serde_json::from_value(v["sqlite_descriptor"].clone()).expect("descriptor parses");
        let reserialized = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(
            reserialized, v["sqlite_descriptor"],
            "descriptor shape drift for module_id {id}"
        );
    }
}

#[test]
fn golden_vectors_break_slug_collisions() {
    // The fixture must contain the a-b / a_b pair and they must have DISTINCT
    // database names (the whole point of hashing the full id, not slug-folding).
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let by_id: std::collections::HashMap<&str, &str> = doc["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["module_id"].as_str().unwrap(),
                v["postgres_database_name"].as_str().unwrap(),
            )
        })
        .collect();
    let a = by_id.get("a-b").expect("fixture has a-b");
    let b = by_id.get("a_b").expect("fixture has a_b");
    assert_ne!(a, b, "a-b and a_b must map to distinct database names");
}
