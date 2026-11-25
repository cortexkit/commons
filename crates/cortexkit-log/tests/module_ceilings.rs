use std::fs;

use cortexkit_log::{Config, Crates, LevelFilter, LogLayer};
use tempfile::TempDir;
use tracing::{Event, Level, Metadata};
use tracing_core::callsite::DefaultCallsite;
use tracing_core::field::FieldSet;
use tracing_core::metadata::Kind;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, Registry};

macro_rules! event_metadata {
    ($level:ident, $target:literal, $module_path:expr) => {{
        static CALLSITE: DefaultCallsite = DefaultCallsite::new(&METADATA);
        static METADATA: Metadata<'static> = Metadata::new(
            "test event",
            $target,
            Level::$level,
            None,
            None,
            $module_path,
            FieldSet::new(&[], tracing_core::identify_callsite!(&CALLSITE)),
            Kind::EVENT,
        );
        &METADATA
    }};
}

fn dispatch_event(dispatch: &tracing::Dispatch, metadata: &'static Metadata<'static>) {
    // Bypass callsite filtering to exercise the ceiling check on the write path.
    dispatch.event(&Event::new(metadata, &metadata.fields().value_set(&[])));
}

#[test]
fn module_ceilings_filter_and_write_using_the_most_specific_path() {
    let connector = event_metadata!(ERROR, "nats", Some("async_nats::connector"));
    let child = event_metadata!(ERROR, "async_nats::connector::handshake", None);
    let client = event_metadata!(WARN, "async_nats::connector", Some("async_nats::client"));
    let info = event_metadata!(INFO, "nats", Some("async_nats::client"));
    let different_prefix = event_metadata!(WARN, "nats", Some("async_nats::connector_pool"));

    let policies = [
        Crates::new()
            .ceiling("async_nats", LevelFilter::WARN)
            .ceiling("async_nats::connector", LevelFilter::OFF),
        Crates::new()
            .ceiling("async_nats::connector", LevelFilter::OFF)
            .ceiling("async_nats", LevelFilter::WARN),
    ];
    for policy in policies {
        let temp = TempDir::new().expect("temp directory");
        let mut config = Config::in_dir("example", temp.path());
        config.spec = Some("info,example.dep.async-nats=trace".to_owned());
        let (layer, handle) = cortexkit_log::layer(config).expect("logger layer");
        let layer = layer.with_crates(policy);

        for suppressed in [connector, child, info] {
            assert!(
                <LogLayer as Layer<Registry>>::register_callsite(&layer, suppressed).is_never(),
                "{:?} at {} must be filtered",
                suppressed.module_path().unwrap_or(suppressed.target()),
                suppressed.level(),
            );
        }
        for kept in [client, different_prefix] {
            assert!(<LogLayer as Layer<Registry>>::register_callsite(&layer, kept).is_always());
        }

        let dispatch = tracing::Dispatch::new(Registry::default().with(layer));
        for event in [connector, child, client, info, different_prefix] {
            dispatch_event(&dispatch, event);
        }
        let contents = fs::read_to_string(handle.path()).expect("log segment readable");
        let lines: Vec<_> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "{contents}");
        assert!(
            lines
                .iter()
                .all(|line| line.contains(" WARN  example.dep.async-nats:")),
            "{contents}"
        );
    }
}

#[test]
fn module_ceilings_on_own_crates_keep_their_logger_names() {
    let blocked = event_metadata!(ERROR, "worker", Some("example_core::retry"));
    let kept = event_metadata!(INFO, "worker", Some("example_core::run"));
    let temp = TempDir::new().expect("temp directory");
    let mut config = Config::in_dir("example", temp.path());
    config.spec = Some("off,example.worker=trace".to_owned());
    let (layer, handle) = cortexkit_log::layer(config).expect("logger layer");
    let layer = layer.with_crates(
        Crates::new()
            .own(["example_core"])
            .ceiling("example_core", LevelFilter::OFF)
            .ceiling("example_core::retry", LevelFilter::OFF),
    );

    assert!(<LogLayer as Layer<Registry>>::register_callsite(&layer, blocked).is_never());
    assert!(<LogLayer as Layer<Registry>>::register_callsite(&layer, kept).is_always());

    let dispatch = tracing::Dispatch::new(Registry::default().with(layer));
    dispatch_event(&dispatch, blocked);
    dispatch_event(&dispatch, kept);
    let contents = fs::read_to_string(handle.path()).expect("log segment readable");
    assert_eq!(contents.lines().count(), 1, "{contents}");
    assert!(contents.contains(" INFO  example.worker:"), "{contents}");
}

#[test]
fn module_ceilings_can_permit_more_than_the_crate_ceiling() {
    let enabled = event_metadata!(DEBUG, "nats", Some("async_nats::client::pool"));
    let temp = TempDir::new().expect("temp directory");
    let mut config = Config::in_dir("example", temp.path());
    config.spec = Some("off,example.dep.async-nats=trace".to_owned());
    let (layer, _) = cortexkit_log::layer(config).expect("logger layer");
    let layer = layer.with_crates(
        Crates::new()
            .ceiling("async_nats::client::pool", LevelFilter::DEBUG)
            .ceiling("async_nats::client", LevelFilter::WARN)
            .ceiling("async_nats", LevelFilter::OFF),
    );

    assert_eq!(
        <LogLayer as Layer<Registry>>::max_level_hint(&layer),
        Some(LevelFilter::DEBUG)
    );
    assert!(<LogLayer as Layer<Registry>>::register_callsite(&layer, enabled).is_always());
}
