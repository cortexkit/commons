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

/// The platform data home, resolved by THE SUPERVISOR'S RULES -- this function
/// is a byte-for-byte mirror of `default_data_home` in subconscious
/// `crates/subc-core/src/daemon_config.rs`, which is the authority: the daemon
/// resolves each module's storage descriptor with that function, so a module
/// resolving its own path with different rules silently splits from the
/// directory its supervisor serves. Any change lands THERE first and is
/// mirrored here against the shared golden fixture
/// (`tests/golden/data_home_resolution.json`, vendored from subconscious).
///
/// The rules, in order: non-empty `XDG_DATA_HOME` is honored AS-IS (relative
/// values included -- the daemon does not require absoluteness, so neither may
/// we); on Windows, non-empty `APPDATA`, else `USERPROFILE\AppData\Roaming`;
/// then `$HOME/.local/share`; finally the relative `.local/share`. Empty env
/// values count as unset. No trimming here -- compose sites trim.
///
/// Modules must not re-derive this by hand. Hand-rolled `env-or-XDG-or-HOME`
/// assembly is how a module directory got fed back in as a data home and
/// produced a doubled `<module>/cortexkit/<module>` store path in production
/// (astrocyte, 2026-08); the resolver exists so that assembly has exactly one
/// spelling. The ONLY supported relocation mechanism is `XDG_DATA_HOME` itself
/// (rigs set it in the module's env block). Private per-module `*_DATA_DIR`
/// conventions are unsupported: they create a second boundary that this crate
/// cannot see.
pub fn resolve_data_home() -> String {
    resolve_data_home_path().to_string_lossy().into_owned()
}

/// `PathBuf` form of [`resolve_data_home`]; the join operations reproduce the
/// daemon's separator behavior exactly (backslash joins on Windows).
fn resolve_data_home_path() -> std::path::PathBuf {
    use std::path::PathBuf;

    if let Some(v) = non_empty_env("XDG_DATA_HOME") {
        return PathBuf::from(v);
    }

    #[cfg(windows)]
    {
        if let Some(app_data) = non_empty_env("APPDATA") {
            return PathBuf::from(app_data);
        }
        if let Some(user_profile) = non_empty_env("USERPROFILE") {
            return PathBuf::from(user_profile).join("AppData").join("Roaming");
        }
    }

    if let Some(home) = non_empty_env("HOME") {
        return PathBuf::from(home).join(".local").join("share");
    }

    PathBuf::from(".local").join("share")
}

/// Empty env values count as unset, mirroring the daemon's `non_empty_os_var`.
fn non_empty_env(key: &str) -> Option<std::ffi::OsString> {
    let value = std::env::var_os(key)?;
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// The conventional module data directory (`<data_home>/cortexkit/<module_id>`),
/// with the data home resolved by [`resolve_data_home`]. Modules that keep
/// non-sqlite state (journals, caches, rings) root it here.
pub fn module_data_dir(module_id: &str) -> String {
    format!(
        "{}/cortexkit/{}",
        resolve_data_home().trim_end_matches('/'),
        module_id
    )
}

/// THE entry point for a module resolving its own sqlite store path:
/// `<resolved data home>/cortexkit/<module_id>/store.db`. Wraps
/// [`resolve_data_home`] + [`sqlite_store_path`] so no module hand-assembles
/// either half. Prefer this over calling [`sqlite_store_path`] directly;
/// the two-argument form exists for callers that genuinely hold a foreign
/// data home (the daemon resolving descriptors, tests, rig tooling).
pub fn module_store_path(module_id: &str) -> String {
    sqlite_store_path(&resolve_data_home(), module_id)
}

/// The conventional sqlite store path for a module under a data-home root
/// (`<data_home>/cortexkit/<module_id>/store.db`). subc uses this to resolve a
/// sqlite descriptor; the resolved absolute path then travels in the descriptor.
///
/// The first argument is an XDG-STYLE DATA HOME (`~/.local/share`), never an
/// already-qualified module directory -- passing `<data_home>/cortexkit/<id>`
/// here doubles the nesting. Modules resolving their OWN path should call
/// [`module_store_path`] and never assemble the data home by hand.
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

#[cfg(test)]
mod resolver_tests {
    use super::*;

    // Env-mutating tests share one lock: cargo runs tests concurrently and
    // XDG_DATA_HOME is process-global.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn module_store_path_honours_xdg_data_home() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-test");
        let got = module_store_path("astrocyte");
        std::env::remove_var("XDG_DATA_HOME");
        assert_eq!(got, "/tmp/xdg-test/cortexkit/astrocyte/store.db");
    }

    #[test]
    fn module_store_path_defaults_to_home_local_share() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("XDG_DATA_HOME");
        std::env::set_var("HOME", "/tmp/home-test");
        let got = module_store_path("astrocyte");
        assert_eq!(
            got,
            "/tmp/home-test/.local/share/cortexkit/astrocyte/store.db"
        );
    }

    #[test]
    fn relative_xdg_data_home_is_honored_matching_the_daemon() {
        // The XDG basedir spec calls relative paths invalid, but the AUTHORITY
        // here is the supervisor, not the spec: subc's default_data_home
        // honors a non-empty XDG_DATA_HOME as-is, so this crate must too --
        // rejecting it would resolve a different directory than the descriptor
        // the daemon serves, which is the exact divergence class this crate
        // exists to eliminate (CKCRED's Windows finding, 2026-08).
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("XDG_DATA_HOME", "relative/path");
        std::env::set_var("HOME", "/tmp/home-test");
        let got = module_store_path("m");
        std::env::remove_var("XDG_DATA_HOME");
        assert_eq!(got, "relative/path/cortexkit/m/store.db");
    }

    #[test]
    fn empty_env_values_count_as_unset() {
        // Mirrors the daemon's non_empty_os_var: an empty XDG_DATA_HOME falls
        // through to the next rule rather than resolving an empty data home.
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("XDG_DATA_HOME", "");
        std::env::set_var("HOME", "/tmp/home-test");
        let got = module_store_path("m");
        std::env::remove_var("XDG_DATA_HOME");
        assert_eq!(got, "/tmp/home-test/.local/share/cortexkit/m/store.db");
    }

    #[test]
    fn feeding_a_module_dir_as_data_home_doubles_the_nesting() {
        // The astrocyte defect, pinned as a NEGATIVE example: this is what the
        // two-argument form does when handed an already-qualified module dir,
        // and why module_store_path exists. If this test ever fails, the
        // low-level contract changed and every caller comment referencing the
        // doubling hazard is stale.
        let doubled = sqlite_store_path("/x/.local/share/cortexkit/astrocyte", "astrocyte");
        assert_eq!(
            doubled,
            "/x/.local/share/cortexkit/astrocyte/cortexkit/astrocyte/store.db"
        );
    }
}
