//! Hooks for controlling fixtures, never for manufacturing passing replies.

use cortexkit_role_agent_inbox::post::Attention;
use serde_json::Value;

/// Test identities that the adapter must authenticate on separate real routes.
/// FlowA and FlowB share a carrier principal, but have different stamped
/// delegated sources; OtherTarget shares FlowA's source, not its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    Provider,
    OtherProvider,
    FlowA,
    FlowB,
    OtherTarget,
}

/// The controlled daemon `scope.served_by` answer (Ended also controls the
/// scope lifecycle). Flow routes must never query this hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeStatus {
    Served,
    NotServed,
    Unknown,
    Ended,
}

/// Captured at the receiving boundary, not reconstructed from post params,
/// inbox.list, inbox.read, or the implementer's configured expected answer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Delivery {
    pub item_id: String,
    pub body: String,
    pub attention: Attention,
    pub system_role: bool,
}

impl Delivery {
    pub fn new(item_id: impl Into<String>, body: impl Into<String>, attention: Attention) -> Self {
        Self {
            item_id: item_id.into(),
            body: body.into(),
            attention,
            system_role: false,
        }
    }
    pub fn with_system_role(mut self, system_role: bool) -> Self {
        self.system_role = system_role;
        self
    }
}

/// All hooks are required. Missing hooks fail a case; nothing is silently
/// skipped. Fixtures use scope_ref `scope`, epoch 1 and target `agent` (or
/// `other-agent` for OtherTarget). Flow posts use the matching stamped flow
/// scope and target_agent. Keep receivers and observations outside the store.
pub trait AgentInboxSubject {
    /// Fresh isolated state per case. Set the clock to 1000 ms, served scope,
    /// unlimited pending quota, wake grant, and freeze automatic dispatch.
    /// Install template `echo`, rendering slots and data string leaves, links
    /// and displayed URLs. Permit it to the fixture identities.
    fn reset(&mut self, case: &'static str) -> Result<(), String>;

    /// Execute a real management/tool request as this authenticated identity.
    /// Return the unmodified successful body or refusal body `{code,message,...}`.
    /// Err is reserved for transport/adapter failures, not typed refusals.
    /// Agent tools bind the caller's target inbox, not its sender namespace.
    fn call(&mut self, caller: Caller, method: &str, params: Value) -> Result<Value, String>;

    fn set_scope_status(&mut self, status: ScopeStatus) -> Result<(), String>;

    /// Restart the implementer with its same durable storage, preserving the
    /// clock, fixture routing identities and independently captured deliveries.
    fn restart(&mut self) -> Result<(), String>;

    /// Set the implementer clock and finish due expiry work; never dispatch.
    fn advance_to(&mut self, now_ms: u64) -> Result<(), String>;

    /// Admit all currently eligible items, wait for confirmed landings, and
    /// return only new observations from the independent receiver. Ride/silent
    /// fixtures provide a next-turn/digest boundary without promoting attention.
    fn deliver(&mut self) -> Result<Vec<Delivery>, String>;

    /// Make the next delivery outcome ambiguous at the receiver, then finish
    /// reconciliation as unconfirmed. Do not synthesize a stored item.
    fn make_unconfirmed(&mut self) -> Result<(), String>;

    /// Set the pending quota for each sender/target to this many live items.
    fn set_pending_cap(&mut self, count: usize) -> Result<(), String>;
}
