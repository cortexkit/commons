//! What the suite needs from a runner's management route.

use async_trait::async_trait;
use serde_json::{json, Value};
use subc_protocol::ErrorBody;

/// How a runner answered one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// A `RESPONSE` frame with its JSON body.
    Response(Value),
    /// An `ERROR` frame with its body.
    Error(ErrorBody),
}

/// How a runner answered one `session.subscribe`.
#[derive(Clone, Debug, PartialEq)]
pub enum SubscribeOutcome {
    /// The runner refused the subscription.
    Refused(ErrorBody),
    /// Every event the runner sent, in order, until the stream went quiet.
    Events(Vec<Value>),
}

/// The route closed or broke before the runner answered, for example
/// because the runner was killed.
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

/// The body of a role request: `{method, params}`.
pub fn envelope(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": params })
}

/// A session-bound management route to a live runner, opened by the
/// subject under a given stamp.
///
/// The adapter behind it must use a real route to the real module: the
/// daemon's route for a subc module, never an in-process shortcut into the
/// runner's code.
#[async_trait]
pub trait RunnerRoute: Send + Sync {
    /// Send one `REQUEST` whose body is [`envelope`]`(method, params)` and
    /// return the terminal frame the runner answered with.
    async fn request(&self, method: &str, params: Value) -> Result<Reply, RouteFailure>;

    /// Send `session.subscribe` with `params` and collect every event the
    /// runner streams back, in order, until no event has arrived for a
    /// short quiet window; then close the subscription. The suite only
    /// subscribes to idle sessions, so the stream it sees is a replay.
    async fn subscribe(&self, params: Value) -> Result<SubscribeOutcome, RouteFailure>;
}
