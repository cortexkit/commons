//! Shared identities for Claude Code steps, taken from the API request id.

/// The host record key shared by Claude Code live steps and durable usage rows.
///
/// The key is the request id verbatim: the `requestId` Claude Code writes on a
/// transcript entry, which is also the request id a runner reads from the
/// response to that call (for example `req_011CgwTtWTJC7pMsgg6bxRCn`). Both
/// sides pass what they read through this function so they agree on the key.
///
/// Returns `None` for an empty id, or one containing any whitespace or control
/// character. A real request id has neither, and such an id is never trimmed or
/// repaired: a key the two sides spelled differently would not merge, so the
/// step keeps no host record rather than a guessed one. There is no length
/// limit, matching the other step refs in this crate.
pub fn claude_code_step_ref(request_id: &str) -> Option<String> {
    if request_id.is_empty()
        || request_id
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    Some(request_id.to_owned())
}
