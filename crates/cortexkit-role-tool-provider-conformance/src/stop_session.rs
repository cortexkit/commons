//! Stop-session isolation is checked against independent live-work observations.
use crate::{single_terminal, CaseOutcome, ObservedFrame, ToolProviderSubject, ToolRoute};
use cortexkit_role_tool_provider::stop_session::StopSessionReply;
use serde_json::{json, Value};

/// Fresh real routes and live provider-owned work for one stop-session case.
#[non_exhaustive]
pub struct StopSessionFixture<R> {
    pub runner: R,
    pub other_principal: R,
    pub primary_work: String,
    pub other_work: String,
}
impl<R> StopSessionFixture<R> {
    pub fn new(
        runner: R,
        other_principal: R,
        primary_work: impl Into<String>,
        other_work: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            other_principal,
            primary_work: primary_work.into(),
            other_work: other_work.into(),
        }
    }
}

pub(crate) async fn run<S: ToolProviderSubject>(
    subject: &S,
    handle: &S::Handle,
    name: &str,
    request_id: &str,
) -> CaseOutcome
where
    S::Route: ToolRoute,
{
    let result = async {
        let Some(fixture) = subject
            .stop_session_fixture(handle)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        if !subject
            .stop_session_work_running(handle, &fixture.primary_work)
            .await
            .map_err(|e| e.to_string())?
            || !subject
                .stop_session_work_running(handle, &fixture.other_work)
                .await
                .map_err(|e| e.to_string())?
        {
            return Err("stop-session fixture did not seed two live sessions".into());
        }
        let body = request(name, request_id);
        let route = if name == "stop_session_other_principal_isolated" {
            &fixture.other_principal
        } else {
            &fixture.runner
        };
        let exchange = route.request(body).await.map_err(|e| e.message)?;
        let primary_running = subject
            .stop_session_work_running(handle, &fixture.primary_work)
            .await
            .map_err(|e| e.to_string())?;
        let other_running = subject
            .stop_session_work_running(handle, &fixture.other_work)
            .await
            .map_err(|e| e.to_string())?;
        check(
            name,
            &exchange,
            primary_running,
            other_running,
            &fixture.primary_work,
        )?;
        Ok(Some(()))
    }
    .await;
    match result {
        Ok(Some(())) => CaseOutcome::Passed,
        Ok(None) => CaseOutcome::Inapplicable {
            reason: "adapter supplies no stop-session provider fixture".into(),
        },
        Err(reason) => CaseOutcome::Failed { reason },
    }
}
fn request(name: &str, id: &str) -> Value {
    let mut body = json!({"name":"tool.stop_session","arguments":{"request_id":id}});
    if name == "stop_session_body_address_refused" {
        body["arguments"]["session"] = json!("other-session");
    }
    body
}
fn check(
    name: &str,
    exchange: &crate::Exchange,
    primary_running: bool,
    other_running: bool,
    primary_id: &str,
) -> Result<(), String> {
    let frame = single_terminal(exchange).map_err(|e| e.to_string())?;
    if name == "stop_session_body_address_refused" {
        return if matches!(frame, ObservedFrame::Error(error) if error.code == "invalid_request" && error.detail.as_ref().is_some_and(|detail| detail["field"] == "arguments.session"))
            && primary_running
            && other_running
        {
            Ok(())
        } else {
            Err("body session was not refused before changing work".into())
        };
    }
    let ObservedFrame::Response(body) = frame else {
        return Err("stop-session expected a response".into());
    };
    let reply: StopSessionReply =
        serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
    reply.validate()?;
    if name == "stop_session_other_principal_isolated"
        && (!primary_running
            || reply
                .items
                .iter()
                .any(|item| item.id.as_deref() == Some(primary_id)))
    {
        return Err("another principal stopped or listed the runner's work".into());
    }
    if !other_running {
        return Err("stop-session stopped another session's work".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Exchange;
    use cortexkit_role_tool_provider::{
        call::ToolCallRequest,
        stop_session::{check_request, SessionKey},
    };
    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        Principal,
        Body,
        Label,
        Items,
        Session,
    }
    struct FakeProvider {
        fault: Fault,
        primary: bool,
        other: bool,
    }
    impl FakeProvider {
        fn stop(&mut self, name: &str) -> Exchange {
            let principal = if name == "stop_session_other_principal_isolated" {
                "other"
            } else {
                "runner"
            };
            let request: ToolCallRequest = serde_json::from_value(request(name, "stop-1")).unwrap();
            if self.fault != Fault::Body {
                if let Err(error) = check_request(
                    &request,
                    Some(&SessionKey::new(principal, "app", "session")),
                ) {
                    return Exchange {
                        frames: vec![ObservedFrame::Error(error)],
                    };
                }
            }
            if request.arguments.get("session").is_some() && self.fault == Fault::Body {
                self.other = false;
            }
            let mut items = vec![];
            if principal == "runner" || self.fault == Fault::Principal {
                self.primary = false;
                items.push(json!({"kind":"call","id":"work-1","label":if self.fault == Fault::Label { "x".repeat(121) } else { "Work".into() },"outcome":"stopped"}));
            }
            if self.fault == Fault::Items {
                items.resize(
                    65,
                    json!({"kind":"call","label":"Work","outcome":"stopped"}),
                );
            }
            if self.fault == Fault::Session {
                self.other = false;
            }
            Exchange {
                frames: vec![ObservedFrame::Response(json!({"items":items}))],
            }
        }
    }
    #[test]
    fn stop_session_faulty_providers_fail_each_named_case() {
        for (name, fault) in [
            ("stop_session_other_principal_isolated", Fault::Principal),
            ("stop_session_body_address_refused", Fault::Body),
            ("stop_session_label_bounded", Fault::Label),
            ("stop_session_items_bounded", Fault::Items),
            ("stop_session_other_session_isolated", Fault::Session),
        ] {
            let mut provider = FakeProvider {
                fault: Fault::None,
                primary: true,
                other: true,
            };
            let reply = provider.stop(name);
            check(name, &reply, provider.primary, provider.other, "work-1").unwrap();
            let mut provider = FakeProvider {
                fault,
                primary: true,
                other: true,
            };
            let reply = provider.stop(name);
            let failure = check(name, &reply, provider.primary, provider.other, "work-1")
                .map_err(|error| format!("{name}: {error}"))
                .unwrap_err();
            assert!(failure.starts_with(name), "{failure}");
        }
    }
}
