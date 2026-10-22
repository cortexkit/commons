use cortexkit_log::Crates;
use tempfile::TempDir;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

#[path = "support/mod.rs"]
mod support;

#[test]
fn dependency_directive_does_not_raise_a_different_crate() {
    let temp = TempDir::new().expect("temp directory");
    let (layer, handle) = support::layer_with_policy(
        support::config(temp.path(), "info,plexus.dep.hyper-util=debug"),
        Crates::new().own(["plexus"]),
    )
    .expect("logger layer");
    let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

    tracing::dispatcher::with_default(&dispatch, || {
        tracing::debug!("other dependency debug");
    });

    assert!(support::contents(&handle).is_empty());
}
