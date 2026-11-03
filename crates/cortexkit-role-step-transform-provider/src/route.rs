//! Transport-independent checks on daemon-attested route metadata.
//!
//! These types are not request bodies. Adapters obtain their values from the
//! daemon, never from JSON supplied by a caller.

use crate::{
    errors::{self, ErrorBody},
    ops, PROVIDES,
};

/// The session identity bound to a route under the session's scope.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SessionBinding {
    pub harness: String,
    pub session: String,
}

impl SessionBinding {
    pub fn new(harness: impl Into<String>, session: impl Into<String>) -> Self {
        Self {
            harness: harness.into(),
            session: session.into(),
        }
    }
}

/// Metadata supplied by the daemon when a route opens.
/// The project comes from the scope stamp, not a hook or a folder-based key.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Route {
    pub attested_runner: Option<String>,
    /// Selected role/version strings, in `PROVIDES` spelling. Must contain
    /// exactly `step-transform-provider/v2` and no other entry.
    pub roles: Vec<String>,
    pub session_binding: Option<SessionBinding>,
    /// Whether the daemon stamped the route with the session's scope.
    pub scoped: bool,
}

impl Route {
    /// A module-level route; discovery and declaration need no session.
    pub fn new(attested_runner: Option<String>, roles: Vec<String>) -> Self {
        Self {
            attested_runner,
            roles,
            session_binding: None,
            scoped: false,
        }
    }

    pub fn with_session(mut self, binding: SessionBinding, scoped: bool) -> Self {
        self.session_binding = Some(binding);
        self.scoped = scoped;
        self
    }

    /// Check caller attestation and the selected role at bind. Non-session
    /// routes are allowed for discovery and declaration; session scope and
    /// binding are checked per hook call.
    pub fn check_bind(&self) -> Result<(), ErrorBody> {
        if self.attested_runner.is_none() {
            return Err(ErrorBody::new(
                errors::UNATTESTED_RUNNER,
                "No caller module attested",
            ));
        }
        if self.roles.len() != 1 || self.roles[0] != PROVIDES {
            return Err(ErrorBody::new(
                errors::UNKNOWN_ROUTE_ROLE,
                "Expected exactly one served role",
            ));
        }
        Ok(())
    }

    /// After successful bind admission, check session binding and scope for
    /// `transform.hook`. Other methods pass this check; dispatch validation
    /// is separate. The adapter must keep the route usable after a per-call
    /// refusal.
    pub fn check_call(&self, method: &str) -> Result<(), ErrorBody> {
        if method == ops::TRANSFORM_HOOK && (self.session_binding.is_none() || !self.scoped) {
            return Err(ErrorBody::new(
                errors::ROUTE_UNBOUND,
                "Expected a bound session",
            ));
        }
        Ok(())
    }
}
