//! What the kit needs from a route to the provider.

use async_trait::async_trait;
use serde_json::Value;
use subc_protocol::ErrorBody;

/// How the provider answered one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// A `RESPONSE` frame, with its body text exactly as the provider sent
    /// it. The kit checks that it is `{"result": <answer>}`.
    Response(String),
    /// An `ERROR` frame with its body.
    Error(ErrorBody),
}

/// The route closed or broke before the provider answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteFailure {
    pub message: String,
}

impl RouteFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// A route to a live routing provider, opened under one caller identity
/// and scope stamp ([`crate::RouteStamp`]).
///
/// The adapter behind it must use a real route to the real provider: the
/// subc daemon's route (the CortexKit module router) to a provider running
/// as a module, never an in-process shortcut into the provider's code. The
/// provider reads the caller and the scope from the route, so the kit can
/// only test those rules over real routes.
#[async_trait]
pub trait RoutingRoute: Send + Sync {
    /// Send one `REQUEST` whose body is `{"method": method, "params":
    /// params}`, and return the terminal frame the provider answered with.
    async fn request(&self, method: &str, params: &Value) -> Result<Reply, RouteFailure>;
}
