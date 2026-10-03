//! What the suite needs from a module's management route.

use async_trait::async_trait;
use subc_protocol::ErrorBody;

/// How a module answered one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// A `RESPONSE` frame, with its body exactly as the module sent it.
    /// The suite reads number spellings from this text, so the adapter must
    /// not decode and re-encode it.
    Response(String),
    /// An `ERROR` frame with its body.
    Error(ErrorBody),
}

/// The route closed or broke before the module answered.
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

/// A management route to a live classifier module.
///
/// The adapter behind it must use a real route to the real module: the
/// daemon's route for a subc module, never an in-process shortcut into the
/// module's code.
#[async_trait]
pub trait ClassifierRoute: Send + Sync {
    /// Send one `REQUEST` whose body is `{"method": method, "params":
    /// params}`, with `params` inserted as the exact JSON text given, and
    /// return the terminal frame the module answered with.
    ///
    /// `params` is text, not a decoded value, because some checks depend on
    /// its exact bytes: a re-send whose object keys are in another order
    /// must reach the module in that order.
    async fn request(&self, method: &str, params: &str) -> Result<Reply, RouteFailure>;
}
