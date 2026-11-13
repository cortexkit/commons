#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RealtimePurpose {
    VoiceFront,
    Dictation,
}

wire_struct!(
    #[serde(deny_unknown_fields)]
    TokenRequest {
        call_id: String,
        agent_id: String,
        purpose: RealtimePurpose,
        model: String,
        decision_id: String
    }
);

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TokenAnswer {
    pub token: String,
    pub token_ref: String,
    pub expires_at_ms: u64,
    pub provider: String,
    pub model: String,
}
impl TokenAnswer {
    pub fn new(
        token: String,
        token_ref: String,
        expires_at_ms: u64,
        provider: String,
        model: String,
    ) -> Self {
        Self {
            token,
            token_ref,
            expires_at_ms,
            provider,
            model,
        }
    }
}
impl std::fmt::Debug for TokenAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenAnswer")
            .field("token", &"[REDACTED]")
            .field("token_ref", &self.token_ref)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .finish()
    }
}

wire_struct!(
    #[serde(deny_unknown_fields)]
    CallStartedRequest {
        call_id: String,
        token_ref: String,
        vendor_session_id: String
    }
);
wire_struct!(CallStartedAnswer {
    call_id: String,
    vendor_session_id: String
});
wire_struct!(
    #[serde(deny_unknown_fields)]
    CallEndedRequest { call_id: String }
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CallStatus {
    Ended,
}

wire_struct!(CallEndedAnswer {
    call_id: String,
    status: CallStatus
});
wire_struct!(
    #[serde(deny_unknown_fields)]
    WatchRequest { call_id: String }
);
open_enum!(WatchType { Started => "started", ToolCall => "tool_call", Ended => "ended" });
open_enum!(WatchTool { Ask => "ask", HandOff => "hand_off" });
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct WatchEvent {
    #[serde(rename = "type")]
    pub event_type: WatchType,
    pub call_id: String,
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<WatchTool>,
}
impl WatchEvent {
    pub fn new(event_type: WatchType, call_id: String, at_ms: u64) -> Self {
        Self {
            event_type,
            call_id,
            at_ms,
            tool: None,
        }
    }
    pub fn with_tool(mut self, tool: WatchTool) -> Self {
        self.tool = Some(tool);
        self
    }
}

/// Validate the tool/type relationship without closing the decoder to future members.
pub fn check_watch_event(event: &WatchEvent) -> crate::CheckResult {
    if event.tool.is_some() && event.event_type != WatchType::ToolCall {
        return Err(crate::CheckError::new("invalid_params").with_field("tool"));
    }
    Ok(())
}
