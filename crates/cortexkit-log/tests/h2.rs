use cortexkit_log::Crates;
use tempfile::TempDir;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

#[path = "support/mod.rs"]
mod support;

#[test]
fn crate_root_dependency_target_is_not_a_component() {
    let temp = TempDir::new().expect("temp directory");
    let (layer, handle) = support::layer_with_policy(
        support::config(temp.path(), "trace"),
        Crates::new().own(["plexus"]),
    )
    .expect("logger layer");
    let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

    tracing::dispatcher::with_default(&dispatch, || {
        tracing::warn!(target: "h2", "crate root event");
    });

    let contents = support::contents(&handle);
    assert_eq!(contents.lines().count(), 1, "{contents}");
    assert!(
        contents.contains("plexus.dep.h2: crate root event"),
        "{contents}"
    );
}
