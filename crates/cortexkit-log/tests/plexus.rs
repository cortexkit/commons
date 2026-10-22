use std::fs;

use cortexkit_log::{Config, Crates, LevelFilter};
use tempfile::TempDir;
use tracing_core::callsite::Callsite;
use tracing_core::field::FieldSet;
use tracing_core::metadata::{Kind, Metadata};
use tracing_core::subscriber::Interest;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, Registry};

#[path = "support/mod.rs"]
mod support;

struct OwnInfoCallsite;
struct DependencyTraceCallsite;

static OWN_INFO_CALLSITE: OwnInfoCallsite = OwnInfoCallsite;
static OWN_INFO_METADATA: Metadata<'static> = Metadata::new(
    "own_info",
    "worker",
    tracing::Level::INFO,
    None,
    None,
    Some("plexus::worker"),
    FieldSet::new(&[], tracing_core::identify_callsite!(&OWN_INFO_CALLSITE)),
    Kind::EVENT,
);

static DEPENDENCY_TRACE_CALLSITE: DependencyTraceCallsite = DependencyTraceCallsite;
static DEPENDENCY_TRACE_METADATA: Metadata<'static> = Metadata::new(
    "dependency_trace",
    "hyper_util",
    tracing::Level::TRACE,
    None,
    None,
    Some("hyper_util::client::pool"),
    FieldSet::new(
        &[],
        tracing_core::identify_callsite!(&DEPENDENCY_TRACE_CALLSITE),
    ),
    Kind::EVENT,
);

impl Callsite for OwnInfoCallsite {
    fn set_interest(&self, _interest: Interest) {}

    fn metadata(&self) -> &Metadata<'_> {
        &OWN_INFO_METADATA
    }
}

impl Callsite for DependencyTraceCallsite {
    fn set_interest(&self, _interest: Interest) {}

    fn metadata(&self) -> &Metadata<'_> {
        &DEPENDENCY_TRACE_METADATA
    }
}

#[test]
fn own_crate_events_keep_existing_logger_names() {
    let temp = TempDir::new().expect("temp directory");
    let (layer, handle) = support::layer_with_policy(
        support::config(temp.path(), "trace"),
        Crates::new().own(["plexus"]),
    )
    .expect("logger layer");
    let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "worker", "own component");
        tracing::info!(target: "plexus::engine::decode", "own module path");
    });

    let contents = support::contents(&handle);
    let lines: Vec<_> = contents.lines().collect();
    assert_eq!(lines.len(), 2, "{contents}");
    assert!(
        lines[0].contains(" plexus.worker: own component"),
        "{contents}"
    );
    assert!(lines[1].contains(" plexus: own module path"), "{contents}");
}

#[test]
fn no_crate_policy_preserves_legacy_line_bytes() {
    let temp = TempDir::new().expect("temp directory");
    let mut config = Config::in_dir("plexus", temp.path());
    config.spec = Some("trace".to_owned());
    config.clock = Some(std::sync::Arc::new(|| {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_788_604_863)
    }));
    let (layer, handle) = cortexkit_log::layer(config).expect("logger layer");
    let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "worker", "unchanged");
    });

    let expected = "2026-09-05T10:41:03.000Z INFO  plexus.worker: unchanged\n";
    assert_eq!(
        fs::read_to_string(handle.path()).expect("log segment"),
        expected
    );
}

#[test]
fn filter_fn_cannot_bypass_ck_log() {
    use tracing_subscriber::filter::filter_fn;

    let temp = TempDir::new().expect("temp directory");
    let (layer, handle) = support::layer_with_policy(
        support::config(temp.path(), "info"),
        Crates::new().own(["plexus"]),
    )
    .expect("logger layer");
    let dispatch =
        tracing::Dispatch::new(Registry::default().with(layer.with_filter(filter_fn(|_| true))));

    tracing::dispatcher::with_default(&dispatch, || {
        tracing::debug!("must be filtered");
        tracing::info!("must be written");
    });

    let contents = support::contents(&handle);
    assert_eq!(contents.lines().count(), 1, "{contents}");
    assert!(contents.contains("plexus: must be written"), "{contents}");
    assert!(!contents.contains("must be filtered"), "{contents}");
}

#[test]
fn max_level_hint_tracks_root_and_dependency_directive() {
    let temp = TempDir::new().expect("temp directory");
    let (layer, _) =
        cortexkit_log::layer(support::config(temp.path(), "info")).expect("logger layer");
    assert_eq!(
        <cortexkit_log::LogLayer as Layer<Registry>>::max_level_hint(&layer),
        Some(LevelFilter::INFO)
    );

    let (layer, _) = cortexkit_log::layer(support::config(
        temp.path(),
        "info,plexus.dep.hyper-util=debug",
    ))
    .expect("logger layer");
    let layer = layer.with_crates(Crates::new().own(["plexus"]));
    assert_eq!(
        <cortexkit_log::LogLayer as Layer<Registry>>::max_level_hint(&layer),
        Some(LevelFilter::DEBUG)
    );

    // Dependencies are capped at WARN, but the module's own crates follow the
    // root level, so the hint must not fall to the dependency cap.
    let (layer, _) =
        cortexkit_log::layer(support::config(temp.path(), "debug")).expect("logger layer");
    let layer = layer.with_crates(Crates::new().own(["plexus"]));
    assert_eq!(
        <cortexkit_log::LogLayer as Layer<Registry>>::max_level_hint(&layer),
        Some(LevelFilter::DEBUG)
    );
}

#[test]
fn register_callsite_caches_only_callsites_ck_log_can_enable() {
    let temp = TempDir::new().expect("temp directory");
    let (layer, _) = support::layer_with_policy(
        support::config(temp.path(), "info"),
        Crates::new().own(["plexus"]),
    )
    .expect("logger layer");

    let dependency_interest = <cortexkit_log::LogLayer as Layer<Registry>>::register_callsite(
        &layer,
        &DEPENDENCY_TRACE_METADATA,
    );
    assert!(dependency_interest.is_never());

    let own_interest =
        <cortexkit_log::LogLayer as Layer<Registry>>::register_callsite(&layer, &OWN_INFO_METADATA);
    assert!(own_interest.is_always());
}
