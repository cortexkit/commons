//! Dependency-light storage descriptor types: the wire/config contract for
//! CortexKit module storage.
//!
//! There is one central storage config. subc resolves it into a
//! [`StorageDescriptor`] per module and delivers that descriptor to the module
//! (today via the registration handshake). The module hands the descriptor to the
//! `cortexkit-store` crate, which opens the actual database.
//!
//! This crate is kept dependency-light (serde only, no database driver) so the
//! wire crate that carries the descriptor can depend on it without pulling sqlite
//! or a postgres driver into the thin daemon. The heavy `cortexkit-store` crate
//! re-exports these types and provides the open/migrate mechanics.
//!
//! ## Design invariants
//!
//! - The backend set is **extensible** (sqlite now, postgres soon, cloud later). A
//!   new variant is additive; module code does not branch on the backend, it just
//!   hands the descriptor to `cortexkit-store`.
//! - Database **isolation** is explicit, never derived from a naming convention,
//!   so a future per-(module, project) isolation is an additive variant rather
//!   than a breaking change to how names are built.
//! - The descriptor a module receives is fully **resolved and least-privilege**:
//!   it never carries central config or an admin credential. For postgres the DSN
//!   reaches only the module's own database.

use serde::{Deserialize, Serialize};

/// How many physical databases a module's storage spans.
///
/// Explicit, never inferred from a name, so finer isolation can be added without
/// changing existing descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Isolation {
    /// One database for the whole module. A project-scoped module partitions its
    /// own rows internally (e.g. by a project key); it does not get a separate
    /// database per project.
    Module,
    // A future `PerProject { .. }` variant is additive: a module that needs a
    // separate physical database per project would receive that isolation, and
    // the per-project descriptor arrives once the project is known.
}

/// The backend a module's storage runs on.
///
/// Extensible by design: adding a variant (e.g. a cloud backend) does not change
/// the descriptor's meaning for existing backends, and `cortexkit-store` opens
/// whichever variant it is handed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "backend")]
pub enum StorageBackend {
    /// A local sqlite file at `path` (absolute).
    Sqlite { path: String },
    /// A postgres database. `dsn` is a scoped, least-privilege runtime DSN that
    /// reaches only `database` (never an admin or `CREATEDB` DSN). The per-module
    /// database is provisioned out of band; the module connects with this DSN.
    Postgres { dsn: String, database: String },
    // A future `Cloud { endpoint, auth_ref, .. }` variant is additive.
}

impl StorageBackend {
    /// A short, stable backend label used in lease-key namespacing and diagnostics
    /// (so the same logical scope under two backends maps to distinct locks).
    pub fn label(&self) -> &'static str {
        match self {
            StorageBackend::Sqlite { .. } => "sqlite",
            StorageBackend::Postgres { .. } => "postgres",
        }
    }
}

/// The resolved storage handle subc delivers to a module. The module passes this
/// to `cortexkit-store` to open its database; it never sees central config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDescriptor {
    /// The module this storage belongs to. Part of lease-key namespacing so two
    /// modules sharing a lease root cannot collide.
    pub module_id: String,
    /// A stable namespace for this module's storage, independent of backend
    /// naming. Used (with `module_id` and the backend label) to derive the
    /// single-writer lease key.
    pub storage_namespace: String,
    /// How many physical databases this storage spans.
    pub isolation: Isolation,
    /// Where and how the storage lives.
    pub backend: StorageBackend,
}

/// Build the per-module postgres database name: `cortexkit_<slug>_<16hex>`.
///
/// The 16-hex suffix is a hash of the FULL `module_id`, so two ids that slug to
/// the same string (for example `a-b` and `a_b` both slug to `a_b`) still produce
/// distinct database names. This is why a bare "hyphen to underscore" rule is
/// unsafe on its own. The slug is bounded so the whole name fits postgres' 63-byte
/// identifier limit.
pub fn postgres_database_name(module_id: &str) -> String {
    const MAX_SLUG: usize = 36; // 63 - len("cortexkit_") - len("_") - 16
    let slug: String = module_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(MAX_SLUG)
        .collect();
    format!("cortexkit_{slug}_{}", fnv1a_hex(module_id))
}

/// The conventional sqlite store path for a module under a data-home root
/// (`<data_home>/cortexkit/<module_id>/store.db`). subc uses this to resolve a
/// sqlite descriptor; the resolved absolute path then travels in the descriptor.
pub fn sqlite_store_path(data_home: &str, module_id: &str) -> String {
    format!(
        "{}/cortexkit/{}/store.db",
        data_home.trim_end_matches('/'),
        module_id
    )
}

/// FNV-1a 64-bit, hex: a dependency-free deterministic hash for name disambiguation.
fn fnv1a_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_collision_is_broken_by_the_hash() {
        // The Oracle's flagged hazard: a bare hyphen->underscore rule collides
        // `a-b` with `a_b`. The hash of the full id keeps them distinct.
        let a = postgres_database_name("a-b");
        let b = postgres_database_name("a_b");
        assert_ne!(a, b, "distinct module ids must not share a database name");
        assert!(a.starts_with("cortexkit_a_b_"));
        assert!(b.starts_with("cortexkit_a_b_"));
    }

    #[test]
    fn database_name_fits_postgres_identifier_limit() {
        let long = "a-very-long-module-id-that-exceeds-the-postgres-identifier-byte-limit-by-a-lot";
        let name = postgres_database_name(long);
        assert!(name.len() <= 63, "db name {} is {} bytes", name, name.len());
    }

    #[test]
    fn sqlite_path_follows_convention() {
        assert_eq!(
            sqlite_store_path("/home/u/.local/share", "alfonso-routing"),
            "/home/u/.local/share/cortexkit/alfonso-routing/store.db"
        );
        // A trailing slash on the data home does not double up.
        assert_eq!(
            sqlite_store_path("/data/", "m"),
            "/data/cortexkit/m/store.db"
        );
    }

    // Golden round-trip: the descriptor wire shape is a contract. If a field name
    // or tag changes, this fails loudly (the change is then intentional, not
    // accidental drift).
    #[test]
    fn sqlite_descriptor_golden_json() {
        let d = StorageDescriptor {
            module_id: "alfonso-routing".into(),
            storage_namespace: "route-state".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: "/data/cortexkit/alfonso-routing/store.db".into(),
            },
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(
            json,
            r#"{"module_id":"alfonso-routing","storage_namespace":"route-state","isolation":{"kind":"module"},"backend":{"backend":"sqlite","path":"/data/cortexkit/alfonso-routing/store.db"}}"#
        );
        let back: StorageDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn postgres_descriptor_golden_json() {
        let d = StorageDescriptor {
            module_id: "alfonso-routing".into(),
            storage_namespace: "route-state".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Postgres {
                dsn: "postgres://routing:scoped@localhost/cortexkit_alfonso_routing_0badc0de"
                    .into(),
                database: "cortexkit_alfonso_routing_0badc0de".into(),
            },
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: StorageDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn backend_label_is_stable() {
        assert_eq!(
            StorageBackend::Sqlite { path: "x".into() }.label(),
            "sqlite"
        );
        assert_eq!(
            StorageBackend::Postgres {
                dsn: "x".into(),
                database: "y".into()
            }
            .label(),
            "postgres"
        );
    }
}
