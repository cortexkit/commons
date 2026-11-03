//! Provider-result accounting before rendering. Select the measure from the
//! negotiated result format, never from a JSON body's appearance. Late replies
//! use the same measure on their payload, not their late-result envelope.
//! Tool-provider has no runner dependency, so importing its measure shares
//! accounting without introducing a dependency cycle.

use cortexkit_role_tool_provider::result::{self, ContentPart, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use result::MAX_MODEL_VISIBLE_BYTES;

pub const OUTPUT_DISCARDED: &str = "The tool ran, but its output exceeded 64 KiB and was discarded. The call must not be repeated just to see the output.";
pub const INTERIM_DISCARDED: &str = "The tool is still pending; its interim output exceeded 64 KiB and was discarded. Its late result will still be collected.";

/// Metadata carried by a native synthetic size-error result. The renderer
/// chooses its host's error representation, without losing this measurement.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SizeError {
    pub reason: String,
    pub bytes: usize,
    pub limit: usize,
}

impl SizeError {
    pub fn new(bytes: usize) -> Self {
        Self {
            reason: crate::errors::tool_result_reasons::TOOL_RESULT_TOO_LARGE.into(),
            bytes,
            limit: MAX_MODEL_VISIBLE_BYTES,
        }
    }
}

fn size_error(bytes: usize) -> Option<SizeError> {
    (bytes > MAX_MODEL_VISIBLE_BYTES).then(|| SizeError::new(bytes))
}

/// Replace an oversized negotiated result, without changing a held call into
/// a completed call. Returns whether output was discarded. Never dispatches a
/// call; the runner records this result instead of retrying or truncating it.
pub fn enforce(result: &mut ToolResult) -> bool {
    let bytes = result::model_visible_bytes(result);
    if bytes <= MAX_MODEL_VISIBLE_BYTES {
        return false;
    }
    let message = if result.held.is_some() {
        INTERIM_DISCARDED
    } else {
        OUTPUT_DISCARDED
    };
    result.content = vec![ContentPart::text(message)];
    result.structured_content = Some(
        serde_json::to_value(SizeError::new(bytes))
            .unwrap()
            .as_object()
            .unwrap()
            .clone(),
    );
    if result.held.is_none() {
        result.is_error = Some(true);
        result.error_code = Some(crate::errors::tool_result_reasons::TOOL_RESULT_TOO_LARGE.into());
    }
    true
}

/// Legacy results count strings as UTF-8 and other values as canonical JSON.
/// On `true`, store a native error result with [`OUTPUT_DISCARDED`], not a
/// shortened success. A held reply stays pending instead.
pub fn legacy_oversized(body: &Value) -> bool {
    legacy_size_error(body).is_some()
}

pub fn legacy_size_error(body: &Value) -> Option<SizeError> {
    size_error(result::legacy_model_visible_bytes(body))
}

/// Refusals count only their message, remain refusals, and never imply that
/// execution occurred. Replace an oversized message with a size error.
pub fn refusal_oversized(message: &str) -> bool {
    refusal_size_error(message).is_some()
}

pub fn refusal_size_error(message: &str) -> Option<SizeError> {
    size_error(message.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_limit_is_inclusive_and_discards_whole_output() {
        let mut result = ToolResult::new(vec![ContentPart::text("é".repeat(32_768))]);
        assert!(!enforce(&mut result));
        result.content.push(ContentPart::text("!"));
        assert!(enforce(&mut result));
        assert_eq!(result.content, vec![ContentPart::text(OUTPUT_DISCARDED)]);
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.structured_content.as_ref().unwrap()["bytes"], 65_537);
        assert_eq!(result.structured_content.as_ref().unwrap()["limit"], 65_536);
        assert!(result.held.is_none());
        assert!(!legacy_oversized(&Value::String("x".repeat(65_536))));
        assert!(legacy_oversized(&Value::String("x".repeat(65_537))));
        assert!(!refusal_oversized(&"x".repeat(65_536)));
        assert!(refusal_oversized(&"x".repeat(65_537)));
    }

    #[test]
    fn oversized_interim_output_does_not_complete_a_held_call() {
        let mut result = ToolResult::new(vec![ContentPart::text("x".repeat(65_537))]);
        result.held = Some(result::HeldReply::new("person_wait"));
        assert!(enforce(&mut result));
        assert_eq!(result.content, vec![ContentPart::text(INTERIM_DISCARDED)]);
        assert_eq!(result.held.unwrap().reason, "person_wait");
        assert_eq!(result.is_error, None);
        assert_eq!(result.error_code, None);
    }
}
