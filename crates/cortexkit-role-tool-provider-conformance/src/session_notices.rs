//! Describe-shape checks only; delivery policy belongs to the runner.

use crate::{single_terminal, ObservedFrame, ToolRoute};
use cortexkit_role_tool_provider::{describe::check_describe, SESSION_NOTICES_V1};
use serde_json::json;

pub(crate) async fn check<R: ToolRoute>(route: &R) -> Result<(), String> {
    let exchange = route
        .request(json!({"name":"role.describe","arguments":{}}))
        .await
        .map_err(|error| error.message)?;
    let ObservedFrame::Response(raw) = single_terminal(&exchange).map_err(|e| e.to_string())?
    else {
        return Err("session_notices_describe: expected a describe response".into());
    };
    let describe =
        check_describe(raw).map_err(|errors| format!("session_notices_describe: {errors:?}"))?;
    if !describe
        .capabilities
        .iter()
        .any(|cap| cap == SESSION_NOTICES_V1)
    {
        return Err("session_notices_describe: declared token missing from answer".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Exchange, RouteFailure};
    use async_trait::async_trait;
    use serde_json::Value;

    struct Provider {
        malformed: bool,
    }
    #[async_trait]
    impl ToolRoute for Provider {
        async fn request(&self, _: Value) -> Result<Exchange, RouteFailure> {
            let caps = if self.malformed {
                json!([SESSION_NOTICES_V1, 7])
            } else {
                json!([SESSION_NOTICES_V1])
            };
            Ok(Exchange {
                frames: vec![ObservedFrame::Response(json!({
                    "majors":[{"version":"tool-provider/v1","ops":["role.describe","tool.catalog"],"stability":"alpha"}],
                    "implementation_version":"fake", "capabilities":caps
                }))],
            })
        }
        async fn request_then_cancel(&self, body: Value) -> Result<Exchange, RouteFailure> {
            self.request(body).await
        }
    }

    #[tokio::test]
    async fn session_notices_describe_rejects_faulty_provider_by_name() {
        check(&Provider { malformed: false }).await.unwrap();
        let error = check(&Provider { malformed: true }).await.unwrap_err();
        assert!(error.starts_with("session_notices_describe:"), "{error}");
        println!("rejected {error}");
    }
}
