//! Focused probes for provider conversation identity and route preflight.
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
    /// state must have distinct markers. Harness and session are route binding
    /// fields under the session's scope, never hook body fields.
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

/// Case name for the non-session route's per-call hook refusal.
pub const HOOK_ROUTE_CASE: &str = "hook_on_non_session_route";
/// Plan composition can declare before a session or hook route exists.
pub const DECLARE_ROUTE_CASE: &str = "declare_on_module_route";

/// Adapter over real role routes. Opening a route must not send a hook.
/// The most recently opened route is used by the following calls.
pub trait RouteProbe {
    /// Open an attested, correctly role-selected route without a session.
    /// `scoped` also probes a scope that exists before the session does.
    fn open_non_session_route(&mut self, scoped: bool) -> Result<(), crate::errors::ErrorBody>;
    fn hook(
        &mut self,
        call: &crate::hook::HookCall,
    ) -> Result<crate::answer::HookAnswer, crate::errors::ErrorBody>;
    fn declare(
        &mut self,
        request: &crate::subscription::DeclareRequest,
    ) -> Result<crate::subscription::Declaration, crate::errors::ErrorBody>;
    fn describe(&mut self) -> Result<crate::describe::RoleDescribe, crate::errors::ErrorBody>;
}

/// Both non-session route kinds must bind successfully; hooks must return
/// error code `route_unbound`, and a subsequent discovery RPC must succeed.
/// This probe does not check refusal wording or discovery completeness; the
/// declaration probe validates discovery shape and required ops.
pub fn hook_on_non_session_route(adapter: &mut impl RouteProbe) -> Result<(), String> {
    use crate::{
        errors::ROUTE_UNBOUND,
        hook::{HookCall, Subject},
    };
    let call = HookCall::new(Subject::PreUser {
        blocks: vec!["hello".into()],
        mark: None,
        delivery: None,
        at_ms: None,
    });
    for scoped in [false, true] {
        adapter
            .open_non_session_route(scoped)
            .map_err(|error| format!("{HOOK_ROUTE_CASE}: bind refused: {}", error.code))?;
        match adapter.hook(&call) {
            Err(error) if error.code == ROUTE_UNBOUND => {}
            result => {
                return Err(format!(
                    "{HOOK_ROUTE_CASE}: expected per-call route_unbound, got {result:?}"
                ))
            }
        }
        adapter.describe().map_err(|error| {
            format!(
                "{HOOK_ROUTE_CASE}: route unusable after refusal: {}",
                error.code
            )
        })?;
    }
    Ok(())
}

/// `preset` must name a preset the provider supports; no universal preset
/// name is required by the role.
pub fn declare_on_module_route(adapter: &mut impl RouteProbe, preset: &str) -> Result<(), String> {
    use crate::subscription::DeclareRequest;
    for scoped in [false, true] {
        adapter
            .open_non_session_route(scoped)
            .map_err(|error| format!("{DECLARE_ROUTE_CASE}: bind refused: {}", error.code))?;
        let request = DeclareRequest::new(serde_json::Map::new()).with_preset(preset);
        let declaration = adapter
            .declare(&request)
            .map_err(|error| format!("{DECLARE_ROUTE_CASE}: declare refused: {}", error.code))?;
        if !declaration.check().is_empty() {
            return Err(format!("{DECLARE_ROUTE_CASE}: malformed declaration"));
        }
        let described = adapter
            .describe()
            .map_err(|error| format!("{DECLARE_ROUTE_CASE}: describe refused: {}", error.code))?;
        crate::describe::check_describe(
            &serde_json::to_value(described).expect("describe is JSON"),
        )
        .map_err(|problems| format!("{DECLARE_ROUTE_CASE}: unusable describe: {problems:?}"))?;
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

    #[derive(Default)]
    struct FakeRoutes {
        accepts_unbound_hooks: bool,
        requires_session_for_declare: bool,
        refuses_unbound_at_bind: bool,
    }

    impl RouteProbe for FakeRoutes {
        fn open_non_session_route(&mut self, _: bool) -> Result<(), crate::errors::ErrorBody> {
            if self.refuses_unbound_at_bind {
                return Err(crate::errors::ErrorBody::new(
                    crate::errors::ROUTE_UNBOUND,
                    "Expected a bound session",
                ));
            }
            Ok(())
        }
        fn hook(
            &mut self,
            _: &crate::hook::HookCall,
        ) -> Result<crate::answer::HookAnswer, crate::errors::ErrorBody> {
            if self.accepts_unbound_hooks {
                Ok(crate::answer::HookAnswer::Pass)
            } else {
                Err(crate::errors::ErrorBody::new(
                    crate::errors::ROUTE_UNBOUND,
                    "Expected a bound session",
                ))
            }
        }
        fn declare(
            &mut self,
            request: &crate::subscription::DeclareRequest,
        ) -> Result<crate::subscription::Declaration, crate::errors::ErrorBody> {
            use crate::subscription::{Declaration, DeclaredSubscription, Hook, Op};
            assert_eq!(
                serde_json::to_value(request).unwrap(),
                serde_json::json!({"preset":"default", "params":{}})
            );
            if self.requires_session_for_declare {
                return Err(crate::errors::ErrorBody::new(
                    crate::errors::ROUTE_UNBOUND,
                    "Expected a bound session",
                ));
            }
            Ok(Declaration::new(vec![DeclaredSubscription::new(
                Hook::PreUser,
                vec![Op::Append],
                250,
            )]))
        }
        fn describe(&mut self) -> Result<crate::describe::RoleDescribe, crate::errors::ErrorBody> {
            use crate::describe::{Major, RoleDescribe};
            Ok(RoleDescribe::new(
                vec![Major::new(
                    crate::PROVIDES,
                    crate::REQUIRED_OPS
                        .iter()
                        .map(|op| op.to_string())
                        .collect(),
                    "alpha",
                )],
                "example",
            ))
        }
    }

    #[test]
    fn faithful_route_provider_passes() {
        hook_on_non_session_route(&mut FakeRoutes::default()).unwrap();
        declare_on_module_route(&mut FakeRoutes::default(), "default").unwrap();
    }

    #[test]
    fn accepting_unbound_hooks_fails_by_name() {
        let error = hook_on_non_session_route(&mut FakeRoutes {
            accepts_unbound_hooks: true,
            ..Default::default()
        })
        .unwrap_err();
        assert!(error.starts_with(HOOK_ROUTE_CASE), "{error}");
    }

    #[test]
    fn rejecting_unbound_bind_fails_by_name() {
        let error = hook_on_non_session_route(&mut FakeRoutes {
            refuses_unbound_at_bind: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(
            error,
            format!("{HOOK_ROUTE_CASE}: bind refused: route_unbound")
        );
    }

    #[test]
    fn requiring_session_for_declare_fails_by_name() {
        let error = declare_on_module_route(
            &mut FakeRoutes {
                requires_session_for_declare: true,
                ..Default::default()
            },
            "default",
        )
        .unwrap_err();
        assert_eq!(
            error,
            format!("{DECLARE_ROUTE_CASE}: declare refused: route_unbound")
        );
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
