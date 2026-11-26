use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use cortexkit_log::{Config, Crates};

fn main() {
    let logs = PathBuf::from(std::env::args_os().nth(1).expect("log directory"));
    let mut config = Config::in_dir("join-fixture", logs);
    config.spec = Some("info".to_owned());
    config.clock = Some(Arc::new(|| UNIX_EPOCH + Duration::from_secs(1_735_000_000)));
    let handle = cortexkit_log::init_with_crates(
        config,
        Crates::new().own(["join_fixture", "join_fixture_bin"]),
    )
    .expect("initialize fixture logger");
    assert!(!handle.fallback_active());

    join_fixture::emit();
    tracing::info!("bin event");
    tracing::error!("formatted event: {}", "runtime text");
    join_dependency::emit();
    tracing::info!(n_bytes = 4, "check");
    tracing::warn!("check n_bytes={}", 2);
    tracing::info!(target: "join.first", n_bytes = 5, state = "ready", "shared event");
    tracing::info!(target: "join.second", n_bytes = 6, ok = true, "shared event");

    assert_eq!(handle.swallowed_writes(), 0);
}
