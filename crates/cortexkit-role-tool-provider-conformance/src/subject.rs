//! What a provider under test supplies beyond its harness.

use std::{collections::BTreeSet, path::Path};

use async_trait::async_trait;
use cortexkit_role_harness::{Harness, HarnessError, RouteStamp};
use cortexkit_role_tool_provider::scope::ScopeIdentity;
use serde_json::Value;

/// A capability a conformance case requires of the provider or its harness.
/// A case whose requirement the subject does not declare is reported as
/// skipped with the missing names, never as passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// The harness can open routes under a scope stamp, as distinct
    /// principals (a carrier, a second carrier and the scope's owner), and
    /// under a second scope of the same owner. See
    /// [`ToolProviderSubject::scoped_principals`].
    ScopeStamp,
    /// The provider decodes the top-level `call_key` and refuses a malformed
    /// one.
    CallKey,
    /// The provider decodes the top-level `schema_pin` and checks it against
    /// its catalog.
    SchemaPin,
    /// The provider holds calls past its reply and serves `tool.withdraw`.
    /// See [`ToolProviderSubject::held_call`].
    HeldCalls,
    /// The harness spawns the provider with one tool disabled by its exact
    /// name. See [`ToolProviderSubject::disabled_tool`].
    DisableTool,
    /// The provider has a call that runs long enough to be cancelled. See
    /// [`ToolProviderSubject::slow_call`].
    Cancellation,
    /// The provider persists an approval-gated call's Prepared and Authorized
    /// states, and its harness can kill at both. See
    /// [`ToolProviderSubject::approve`].
    ApprovalExecution,
    /// The provider declares the `late_results` session capability and
    /// serves `late_results` and `late_results.ack`.
    LateResults,
}

impl Capability {
    /// The `requires` tag the report prints.
    pub fn name(self) -> &'static str {
        match self {
            Self::ScopeStamp => "scope_stamp",
            Self::CallKey => "call_key",
            Self::SchemaPin => "schema_pin",
            Self::HeldCalls => "held_calls",
            Self::DisableTool => "disable_tool",
            Self::Cancellation => "cancellation",
            Self::ApprovalExecution => "approval_execution",
            Self::LateResults => "late_results",
        }
    }
}

/// The identities the withdraw, late-result and crash cases open routes as.
/// Scoped routes are under `scope` unless a case says otherwise; the scope's
/// owner is `scope.owner`, which is also the custodian of the late results of
/// calls raised under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedPrincipals {
    pub scope: ScopeIdentity,
    /// The principal that raises the held calls, for example `reserved:broca`.
    pub carrier: String,
    /// Another principal that may carry calls under the scope.
    pub other_carrier: String,
    /// A second scope of the same owner, which the harness can also stamp
    /// routes with. A call held under `scope` is not withdrawable from it.
    pub other_scope: ScopeIdentity,
}

impl ScopedPrincipals {
    /// A route stamp for `principal` under `scope`.
    pub fn stamp(&self, principal: &str) -> RouteStamp {
        stamp_under(principal, &self.scope)
    }

    /// A route stamp for `principal` under `other_scope`.
    pub fn stamp_in_other_scope(&self, principal: &str) -> RouteStamp {
        stamp_under(principal, &self.other_scope)
    }
}

fn stamp_under(principal: &str, scope: &ScopeIdentity) -> RouteStamp {
    RouteStamp {
        principal: principal.to_owned(),
        scope: Some(cortexkit_role_harness::ScopeStamp {
            owner: scope.owner.clone(),
            scope_ref: scope.scope_ref.clone(),
            scope_epoch: scope.scope_epoch,
        }),
    }
}

/// A tool call as `(tool name, arguments)`.
pub type CallSpec = (String, Value);

/// A live tool provider under test: its harness plus the few facts the
/// runner cannot know about it. Every call the runner makes still goes over
/// the harness's real route; nothing here answers on the module's behalf.
#[async_trait]
pub trait ToolProviderSubject: Harness {
    /// The capabilities this provider and harness declare. Cases requiring
    /// anything missing are skipped, with the reason.
    fn capabilities(&self) -> BTreeSet<Capability>;

    /// The stamp for the cases that need no scope: `role.describe`,
    /// `tool.catalog` and plain calls.
    fn plain_stamp(&self) -> RouteStamp;

    /// The identities for the scoped cases. Required by
    /// [`Capability::ScopeStamp`].
    fn scoped_principals(&self) -> Option<ScopedPrincipals>;

    /// The arguments of the `tool.catalog` request: the plan item's params
    /// this provider serves and, optionally, a preset it defines. The
    /// `catalog_unknown_preset_refused` case sends these arguments with the
    /// preset replaced by one no provider defines, and expects a refusal.
    fn catalog_arguments(&self) -> Value;

    /// A call that completes promptly and successfully. Its tool must be in
    /// the catalog.
    fn quick_call(&self) -> CallSpec;

    /// A call that keeps running until cancelled. Required by
    /// [`Capability::Cancellation`].
    fn slow_call(&self) -> Option<CallSpec>;

    /// The tool the harness disables, by its exact name, in every module it
    /// spawns. Required by [`Capability::DisableTool`].
    fn disabled_tool(&self) -> Option<String>;

    /// An approval-gated call the provider holds past its reply, whose action,
    /// if it ever runs, creates the file `marker`. Required by
    /// [`Capability::HeldCalls`]. The runner watches `marker` to prove the
    /// action never ran.
    fn held_call(&self, marker: &Path) -> Option<CallSpec>;

    /// Resolve once the provider holds the call sent with `call_key`, that is,
    /// once the provider has filed the call's approval question.
    /// Implementations typically wait for that question to reach whatever
    /// stands in for the elicitation provider in their harness.
    async fn await_held(&self, call_key: &str) -> Result<(), HarnessError>;

    /// Answer "approve" to the question filed for `call_key`, through the same
    /// channel a real answer arrives on. Waits until the question exists.
    /// Required by [`Capability::ApprovalExecution`].
    async fn approve(&self, call_key: &str) -> Result<(), HarnessError>;

    /// Wait long enough that a provider that wrongly runs a held call (after
    /// a restart, or after a withdraw) would have created its marker.
    async fn settle(&self);
}
