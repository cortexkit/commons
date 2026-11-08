//! Shared identities for Pi journal entries, independent of session file paths.

/// Borrowed fields as Pi wrote them, with the provider resolved by the caller.
///
/// The caller classifies operations and assistant messages from Pi's entry
/// shape. `timestamp` is the entry's own timestamp, not `message.timestamp`;
/// counts are raw usage counts, before separating reasoning from output.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct PiEntry<'a> {
    /// The journal entry id, unique within its session file.
    pub id: Option<&'a str>,
    /// The entry's own RFC 3339 timestamp, if present.
    pub timestamp: Option<&'a str>,
    /// Whether the entry records an operation.
    pub is_operation: bool,
    /// Whether the entry contains an assistant message.
    pub is_assistant_message: bool,
    /// The assistant message's raw `responseId`, if present.
    pub response_id: Option<&'a str>,
    /// Provider name already resolved by the caller.
    pub provider: &'a str,
    /// The recorded model, if present.
    pub model: Option<&'a str>,
    /// Raw input usage, including the distinction between missing and zero.
    pub input: Option<u64>,
    /// Raw output usage, before subtracting reasoning.
    pub output: Option<u64>,
    /// Raw cache-read usage.
    pub cache_read: Option<u64>,
    /// Raw cache-write usage.
    pub cache_write: Option<u64>,
    /// Reasoning usage included in raw output, if present.
    pub reasoning: Option<u64>,
}

impl<'a> PiEntry<'a> {
    /// Start an unclassified entry with no optional fields or counts.
    /// Set the public fields to the values from the journal before deriving a key.
    pub fn new(provider: &'a str) -> Self {
        Self {
            id: None,
            timestamp: None,
            is_operation: false,
            is_assistant_message: false,
            response_id: None,
            provider,
            model: None,
            input: None,
            output: None,
            cache_read: None,
            cache_write: None,
            reasoning: None,
        }
    }
}

/// The host record key shared by Pi live steps and durable usage rows.
///
/// Operations use `pi:operation:<id>` and require a non-blank id. Otherwise,
/// assistant messages with a non-blank response id use `pi:response:<responseId>`.
/// Otherwise a non-blank entry id and a model yield
/// `pi:message:<id>:<ts>:<provider>:<model>:<input>:<output>:<cacheRead>:<cacheWrite>`.
/// The first applicable rule wins; operations without an id do not fall through.
/// Ids are checked with `trim` but kept verbatim; components have no escaping.
///
/// `ts` uses exactly `chrono::DateTime::parse_from_rfc3339(timestamp).ok()`
/// followed by `timestamp_millis()`, or `missing` for absent or refused input.
/// Chrono accepts some spelling variants; it truncates submillisecond digits
/// rather than rounding them. Always use the entry timestamp, not the message
/// timestamp. Counts use `Option<u64>`'s Debug spelling (`Some(N)` or `None`).
/// Reasoning is subtracted from output when both are present and it fits;
/// otherwise output is `None` when reasoning is present. Missing model or id
/// yields no message key. Copies of one call in forked session files share a key.
pub fn pi_step_ref(entry: &PiEntry<'_>) -> Option<String> {
    let id = entry.id.filter(|id| !id.trim().is_empty());
    if entry.is_operation {
        return id.map(|id| format!("pi:operation:{id}"));
    }
    if entry.is_assistant_message {
        if let Some(response_id) = entry.response_id.filter(|id| !id.trim().is_empty()) {
            return Some(format!("pi:response:{response_id}"));
        }
    }
    let (id, model) = (id?, entry.model?);
    let timestamp = entry
        .timestamp
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.timestamp_millis().to_string())
        .unwrap_or_else(|| "missing".to_owned());
    let output = match entry.reasoning {
        Some(reasoning) => entry
            .output
            .and_then(|output| output.checked_sub(reasoning)),
        None => entry.output,
    };
    Some(format!(
        "pi:message:{id}:{timestamp}:{}:{model}:{:?}:{output:?}:{:?}:{:?}",
        entry.provider, entry.input, entry.cache_read, entry.cache_write,
    ))
}
