//! A focused probe for provider conversation identity, not a full conformance kit.
//!
//! Enable the non-default `conformance` feature to drive this rule against a
//! provider through an implementation-specific adapter.

/// The case name included in every failure returned by the probe.
pub const CASE_NAME: &str = "conversation_state_ignores_project_root";

/// The minimum adapter needed to observe a provider's per-session state.
///
/// Use a fresh test store. The adapter supplies the daemon-attested runner
/// principal's bare module id, never its `reserved:<id>` display form or any
/// prefix. Module-id arguments select real attested runner connections, never
/// body fields. Capture markers from provider state;
/// do not reconstruct them from the requested identity or route metadata.
pub trait StateProbe {
    /// Set up the session by delivering an initial hook on a route with these
    /// identity fields and folder spelling, then read an opaque, nonempty marker
    /// for the provider's session state. This role has no Setup wire op.
    /// Repeated setup for the same state must return the same marker; distinct
    /// state must have distinct markers. The harness and session below are
    /// hook body fields, not the route's possibly generic bind fields.
    fn setup_and_state_marker(
        &mut self,
        runner_module_id: &str,
        harness: &str,
        session: &str,
        project_root: &str,
    ) -> Result<String, String>;
}

/// The same runner/harness/session through folder aliases reaches the same
/// provider state; another attested runner reaches different state.
///
/// The adapter's test environment must make `/var/probe-project` and
/// `/private/var/probe-project` spellings of the same project. Neither folder
/// spelling is allowed to participate in the provider's state key.
pub fn conversation_state_ignores_project_root(
    adapter: &mut impl StateProbe,
) -> Result<(), String> {
    let mut observe = |runner, root| {
        let marker = adapter
            .setup_and_state_marker(runner, "harness-a", "session-a", root)
            .map_err(|reason| format!("{CASE_NAME}: {reason}"))?;
        if marker.is_empty() {
            return Err(format!(
                "{CASE_NAME}: provider returned an empty state marker"
            ));
        }
        Ok(marker)
    };
    let original = observe("runner-a", "/var/probe-project")?;
    let alias = observe("runner-a", "/private/var/probe-project")?;
    if original != alias {
        return Err(format!(
            "{CASE_NAME}: folder aliases reached different state"
        ));
    }
    let other_runner = observe("runner-b", "/var/probe-project")?;
    if original == other_runner {
        return Err(format!("{CASE_NAME}: different runners shared state"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeProvider {
        folder_keyed: bool,
        ignores_runner: bool,
        runner_ids: Vec<String>,
        states: BTreeMap<(String, String, String), String>,
    }

    impl StateProbe for FakeProvider {
        fn setup_and_state_marker(
            &mut self,
            runner_module_id: &str,
            harness: &str,
            session: &str,
            project_root: &str,
        ) -> Result<String, String> {
            self.runner_ids.push(runner_module_id.into());
            let identity = if self.folder_keyed {
                project_root
            } else if self.ignores_runner {
                ""
            } else {
                runner_module_id
            };
            let key = (identity.into(), harness.into(), session.into());
            let next_marker = format!("state-{}", self.states.len());
            Ok(self.states.entry(key).or_insert(next_marker).clone())
        }
    }

    #[test]
    fn faithful_provider_passes() {
        conversation_state_ignores_project_root(&mut FakeProvider::default()).unwrap();
    }

    #[test]
    fn probe_uses_bare_runner_module_ids() {
        let mut provider = FakeProvider::default();
        conversation_state_ignores_project_root(&mut provider).unwrap();
        assert_eq!(provider.runner_ids, ["runner-a", "runner-a", "runner-b"]);
    }

    #[test]
    fn folder_keyed_provider_fails_by_name() {
        let failure = conversation_state_ignores_project_root(&mut FakeProvider {
            folder_keyed: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(
            failure,
            format!("{CASE_NAME}: folder aliases reached different state")
        );
    }

    #[test]
    fn runner_blind_provider_fails_by_name() {
        let failure = conversation_state_ignores_project_root(&mut FakeProvider {
            ignores_runner: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(
            failure,
            format!("{CASE_NAME}: different runners shared state")
        );
    }
}
