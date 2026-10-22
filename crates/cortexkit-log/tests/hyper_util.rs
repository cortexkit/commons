use cortexkit_log::Crates;
use tempfile::TempDir;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

#[path = "support/mod.rs"]
mod support;

mod client {
    pub mod pool {
        use super::super::*;

        #[test]
        fn dependency_events_use_a_dedicated_label() {
            let temp = TempDir::new().expect("temp directory");
            let (layer, handle) = support::layer_with_policy(
                support::config(temp.path(), "trace"),
                Crates::new().own(["plexus"]),
            )
            .expect("logger layer");
            let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

            tracing::dispatcher::with_default(&dispatch, || {
                tracing::warn!(target: "pool", "dependency line");
            });

            let contents = support::contents(&handle);
            assert_eq!(contents.lines().count(), 1, "{contents}");
            assert!(
                contents.contains("plexus.dep.hyper-util: dependency line"),
                "{contents}"
            );
        }

        #[test]
        fn dependency_warn_cap_holds_under_debug() {
            let temp = TempDir::new().expect("temp directory");
            let (layer, handle) = support::layer_with_policy(
                support::config(temp.path(), "debug"),
                Crates::new().own(["plexus"]),
            )
            .expect("logger layer");
            let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

            tracing::dispatcher::with_default(&dispatch, || {
                tracing::trace!("trace");
                tracing::debug!("debug");
                tracing::info!("info");
                tracing::warn!("warn");
                tracing::error!("error");
            });

            let contents = support::contents(&handle);
            assert_eq!(contents.lines().count(), 2, "{contents}");
            assert!(
                contents.contains("plexus.dep.hyper-util: warn"),
                "{contents}"
            );
            assert!(
                contents.contains("plexus.dep.hyper-util: error"),
                "{contents}"
            );
            assert!(!contents.contains("trace"), "{contents}");
            assert!(!contents.contains("debug"), "{contents}");
            assert!(!contents.contains("info"), "{contents}");
        }

        #[test]
        fn dependency_directive_raises_only_that_crate() {
            let temp = TempDir::new().expect("temp directory");
            let (layer, handle) = support::layer_with_policy(
                support::config(temp.path(), "info,plexus.dep.hyper-util=debug"),
                Crates::new().own(["plexus"]),
            )
            .expect("logger layer");
            let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

            tracing::dispatcher::with_default(&dispatch, || {
                tracing::debug!("hyper-util debug");
            });

            let contents = support::contents(&handle);
            assert_eq!(contents.lines().count(), 1, "{contents}");
            assert!(
                contents.contains("plexus.dep.hyper-util: hyper-util debug"),
                "{contents}"
            );
        }

        #[test]
        fn error_root_keeps_dependencies_at_error() {
            let temp = TempDir::new().expect("temp directory");
            let (layer, handle) = support::layer_with_policy(
                support::config(temp.path(), "error"),
                Crates::new().own(["plexus"]),
            )
            .expect("logger layer");
            let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

            tracing::dispatcher::with_default(&dispatch, || {
                tracing::warn!("warning");
                tracing::error!("error");
            });

            let contents = support::contents(&handle);
            assert_eq!(contents.lines().count(), 1, "{contents}");
            assert!(
                contents.contains("plexus.dep.hyper-util: error"),
                "{contents}"
            );
        }

        #[test]
        fn off_ceiling_drops_dependency_under_trace_directive() {
            let temp = TempDir::new().expect("temp directory");
            let (layer, handle) = support::layer_with_policy(
                support::config(temp.path(), "trace,plexus.dep=trace"),
                Crates::new()
                    .own(["plexus"])
                    .ceiling("hyper_util", tracing_subscriber::filter::LevelFilter::OFF),
            )
            .expect("logger layer");
            let dispatch = tracing::Dispatch::new(Registry::default().with(layer));

            tracing::dispatcher::with_default(&dispatch, || {
                tracing::trace!("ceiling must drop this");
            });

            assert!(support::contents(&handle).is_empty());
        }
    }
}
