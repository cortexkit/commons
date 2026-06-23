//! Emit the canonical cross-language golden vectors for the storage descriptor +
//! derivation helpers. Both the Rust lib (commons) and the TS lib
//! (@cortexkit/store) assert against these exact values, so a TS module can never
//! drift to a different database name or store path than the Rust resolver.
//!
//! Run: `cargo run -p cortexkit-store-types --example golden-vectors`

use cortexkit_store_types::{postgres_database_name, sqlite_store_path, StorageDescriptor};

fn main() {
    // Representative module ids: real ones, the slug-collision pair (a-b vs a_b
    // must NOT share a database name), and a too-long id (must fit pg's 63 bytes).
    let ids = [
        "alfonso-routing",
        "llm-runner",
        "magic-context",
        "ai-provider-quota",
        "a-b",
        "a_b",
        "a-very-long-module-id-that-exceeds-the-postgres-identifier-byte-limit-by-a-lot",
    ];
    let data_home = "/data";

    let vectors: Vec<_> = ids
        .iter()
        .map(|id| {
            let descriptor = StorageDescriptor {
                module_id: (*id).to_string(),
                storage_namespace: "default".to_string(),
                isolation: cortexkit_store_types::Isolation::Module,
                backend: cortexkit_store_types::StorageBackend::Sqlite {
                    path: sqlite_store_path(data_home, id),
                },
            };
            serde_json::json!({
                "module_id": id,
                "postgres_database_name": postgres_database_name(id),
                "sqlite_store_path": sqlite_store_path(data_home, id),
                "sqlite_descriptor": descriptor,
            })
        })
        .collect();

    let doc = serde_json::json!({
        "data_home": data_home,
        "vectors": vectors,
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
