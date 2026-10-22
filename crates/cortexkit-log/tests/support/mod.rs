use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use cortexkit_log::{Config, Crates, Handle, InitError, LogLayer, SegmentRetention};

pub fn config(logs_dir: &Path, spec: &str) -> Config {
    Config {
        module_id: "plexus".to_owned(),
        logs_dir: logs_dir.to_owned(),
        bound: Vec::new(),
        spec: Some(spec.to_owned()),
        retention: SegmentRetention::default(),
        redactor: None,
        clock: Some(Arc::new(|| {
            UNIX_EPOCH + Duration::from_millis(1_788_604_863_123)
        })),
    }
}

pub fn contents(handle: &cortexkit_log::Handle) -> String {
    std::fs::read_to_string(handle.path()).expect("log segment readable")
}

pub fn layer_with_policy(config: Config, crates: Crates) -> Result<(LogLayer, Handle), InitError> {
    let (layer, handle) = cortexkit_log::layer(config)?;
    Ok((layer.with_crates(crates), handle))
}
