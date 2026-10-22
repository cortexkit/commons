//! What the suite needs from a module's management route.

use async_trait::async_trait;
use subc_protocol::ErrorBody;

/// How a module answered one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// A `RESPONSE` frame, with its body exactly as the module sent it.
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

/// A management route to a live extractor module.
///
/// The adapter behind it must use a real route to the real module: the route
/// the subconscious daemon opens to it, never an in-process shortcut into the
/// module's code.
#[async_trait]
pub trait ExtractorRoute: Send + Sync {
    /// Send one `REQUEST` whose body is `{"method": method, "params":
    /// params}`, with `params` inserted as the exact JSON text given, and
    /// return the terminal frame the module answered with.
    async fn request(&self, method: &str, params: &str) -> Result<Reply, RouteFailure>;
}
