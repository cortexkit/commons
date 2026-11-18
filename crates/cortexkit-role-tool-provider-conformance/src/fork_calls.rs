//! Probe fork isolation against independently captured observable state.

use std::future::Future;

use cortexkit_role_tool_provider::{call::fork_id, errors};
use serde_json::{json, Value};

use crate::{single_terminal, ObservedFrame, ToolRoute};

pub(crate) const CASE: &str = "fork_calls_honour_marker";

pub(crate) async fn probe<R, F, Fut>(route: &R, body: Value, mut state: F) -> Result<(), String>
where
    R: ToolRoute,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let before = state().await?;
    let exchange = route
        .request(body.clone())
        .await
        .map_err(|e| format!("{CASE}: {}", e.message))?;
    match single_terminal(&exchange).map_err(|e| format!("{CASE}: {e}"))? {
        ObservedFrame::Error(error) if error.code == errors::FORK_CALL_REFUSED => {
            if error.detail.as_ref().and_then(|d| d["tool"].as_str()) != body["name"].as_str()
                || error
                    .detail
                    .as_ref()
                    .and_then(|d| d["reason"].as_str())
                    .is_none_or(str::is_empty)
            {
                return Err(format!("{CASE}: malformed fork refusal"));
            }
        }
        ObservedFrame::Error(error) => {
            return Err(format!("{CASE}: unexpected refusal {}", error.code))
        }
        ObservedFrame::Response(_) | ObservedFrame::StreamEnd => {}
        _ => return Err(format!("{CASE}: fork call did not finish")),
    }
    if state().await? != before {
        return Err(format!(
            "{CASE}: fork changed observable state or asked the person"
        ));
    }
    let mut malformed = body;
    if let Some(key) = malformed["call_key"].as_str() {
        malformed["call_key"] = json!(format!("{key}:malformed"));
    }
    malformed["fork_id"] = json!("not valid");
    assert!(fork_id(&malformed).is_err());
    let exchange = route
        .request(malformed)
        .await
        .map_err(|e| format!("{CASE}: {}", e.message))?;
    if !matches!(single_terminal(&exchange), Ok(ObservedFrame::Error(error))
        if errors::invalid_request_field(error) == Some("fork_id"))
    {
        return Err(format!("{CASE}: malformed fork_id was not refused"));
    }
    if state().await? != before {
        return Err(format!(
            "{CASE}: malformed marker caused observable changes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Exchange, RouteFailure};
    use async_trait::async_trait;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    struct Provider {
        state: Arc<AtomicU64>,
        changes: bool,
        ignores: bool,
    }
    #[async_trait]
    impl ToolRoute for Provider {
        async fn request(&self, body: Value) -> Result<Exchange, RouteFailure> {
            let frame = if !self.ignores && fork_id(&body).is_err() {
                ObservedFrame::Error(errors::invalid_request("fork_id", "bad marker"))
            } else {
                if self.changes {
                    self.state.fetch_add(1, Ordering::SeqCst);
                }
                ObservedFrame::Response(json!({"text":"found"}))
            };
            Ok(Exchange {
                frames: vec![frame],
            })
        }
        async fn request_then_cancel(&self, body: Value) -> Result<Exchange, RouteFailure> {
            self.request(body).await
        }
    }
    #[tokio::test]
    async fn fork_calls_reject_observable_change_and_ignored_marker_by_check_name() {
        for (changes, ignores) in [(false, false), (true, false), (false, true)] {
            let state = Arc::new(AtomicU64::new(0));
            let provider = Provider {
                state: state.clone(),
                changes,
                ignores,
            };
            let result = probe(&provider, json!({"name":"read", "arguments":{}, "fork_id":"fork:1"}), || {
                let state = state.clone(); async move { Ok(json!({"state_and_prompt_count":state.load(Ordering::SeqCst)})) }
            }).await;
            if changes || ignores {
                assert!(result.unwrap_err().starts_with(CASE));
            } else {
                result.unwrap();
            }
        }
    }
}
