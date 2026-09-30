//! What the runner needs from a provider's route, and the terminal-frame rule.

use async_trait::async_trait;
use serde_json::Value;
use subc_protocol::ErrorBody;

/// One frame a provider sent in answer to a request, as the route adapter
/// observed it on the wire.
#[derive(Clone, Debug, PartialEq)]
pub enum ObservedFrame {
    /// A non-terminal frame: progress or stream data.
    Data(Value),
    /// A `RESPONSE` frame with its JSON body. Terminal.
    Response(Value),
    /// A `STREAM_END` frame. Terminal.
    StreamEnd,
    /// An `ERROR` frame with its body. Terminal.
    Error(ErrorBody),
}

impl ObservedFrame {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Data(_))
    }
}

/// Everything a provider sent in answer to one request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exchange {
    pub frames: Vec<ObservedFrame>,
}

/// The route closed or broke before the exchange could be observed, for
/// example because the module was killed.
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

/// A route to a live provider, opened by the implementation's harness under
/// a given stamp.
///
/// The adapter behind it must use a real route to the real module: the
/// daemon's route for a subc module, never an in-process shortcut into the
/// module's code. Each method sends one `REQUEST` frame whose body is `body`,
/// and returns every frame the module sent for that request's correlation,
/// in order: all frames up to the first terminal one, and any that arrive
/// after it within a short quiet window, so a second terminal frame is
/// observed rather than hidden.
#[async_trait]
pub trait ToolRoute: Send + Sync {
    async fn request(&self, body: Value) -> Result<Exchange, RouteFailure>;

    /// Send the request, then a `CANCEL` for it while the module is still
    /// working on it (after the first frame back, or after a short delay if
    /// none comes), and collect the frames as [`ToolRoute::request`] does.
    async fn request_then_cancel(&self, body: Value) -> Result<Exchange, RouteFailure>;
}

/// Why an exchange does not end in exactly one terminal frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalProblem {
    /// The module never ended the request.
    Missing,
    /// The module ended the request more than once.
    Several { count: usize },
    /// A data frame arrived after the request ended.
    FrameAfterTerminal,
}

impl std::fmt::Display for TerminalProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("no terminal frame"),
            Self::Several { count } => write!(f, "{count} terminal frames"),
            Self::FrameAfterTerminal => f.write_str("a data frame after the terminal frame"),
        }
    }
}

/// Every request gets exactly one terminal frame, and it is the last frame.
/// Returns that frame.
pub fn single_terminal(exchange: &Exchange) -> Result<&ObservedFrame, TerminalProblem> {
    let count = exchange
        .frames
        .iter()
        .filter(|frame| frame.is_terminal())
        .count();
    if count == 0 {
        return Err(TerminalProblem::Missing);
    }
    if count > 1 {
        return Err(TerminalProblem::Several { count });
    }
    let last = exchange.frames.last().expect("a terminal frame exists");
    if !last.is_terminal() {
        return Err(TerminalProblem::FrameAfterTerminal);
    }
    Ok(last)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_request_that_never_ends_or_keeps_talking_is_flagged() {
        let missing = Exchange {
            frames: vec![ObservedFrame::Data(json!(1))],
        };
        assert_eq!(single_terminal(&missing), Err(TerminalProblem::Missing));
        let trailing = Exchange {
            frames: vec![ObservedFrame::StreamEnd, ObservedFrame::Data(json!(1))],
        };
        assert_eq!(
            single_terminal(&trailing),
            Err(TerminalProblem::FrameAfterTerminal)
        );
        let good = Exchange {
            frames: vec![
                ObservedFrame::Data(json!(1)),
                ObservedFrame::Response(json!({})),
            ],
        };
        assert_eq!(
            single_terminal(&good),
            Ok(&ObservedFrame::Response(json!({})))
        );
    }
}
