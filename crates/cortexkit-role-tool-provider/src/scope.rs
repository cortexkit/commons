//! Scope identity as it appears inside a request's arguments.

use serde::{Deserialize, Serialize};

/// A session's scope identity, `(owner, ref, scope_epoch)`.
///
/// A provider never takes scope, owner or agent from a request's arguments as
/// authority; they come from the daemon's stamp on the route. Where a request
/// may name a scope (`tool.withdraw`'s optional `arguments.scope`), the
/// provider only checks it equals the stamp and refuses a mismatch.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct ScopeIdentity {
    /// The principal that owns the scope.
    pub owner: String,
    #[serde(rename = "ref")]
    pub scope_ref: String,
    pub scope_epoch: u64,
}
