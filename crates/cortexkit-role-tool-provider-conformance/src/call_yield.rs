//! Live-route hand-back checks. Optional fixtures never count as passes.

use crate::{single_terminal, CallSpec, ObservedFrame, ToolRoute};
use cortexkit_role_tool_provider::{
    call_yield::YieldAnswer,
    result::{validate, ToolResult},
    CALL_YIELD_DEADLINE_MS,
};
use futures_util::future::{select, Either};
use serde_json::{json, Value};
use std::{future::Future, time::Duration};

pub(crate) fn case_parts(name: &str) -> Option<(&str, &'static str)> {
    name.strip_prefix("call_yield_")?;
    let suffixed = crate::report::CALL_YIELD_REASONS
        .iter()
        .filter(|&&reason| reason != "person_message")
        .find_map(|&reason| {
            name.strip_suffix(reason)
                .and_then(|prefix| prefix.strip_suffix('_'))
                .map(|case| (case, reason))
        });
    suffixed.or(Some((name, "person_message")))
}

fn response(exchange: crate::Exchange) -> Result<Value, String> {
    match single_terminal(&exchange).map_err(|e| e.to_string())? {
        ObservedFrame::Response(value) => Ok(value.clone()),
        other => Err(format!("expected a response value, got {other:?}")),
    }
}

pub(crate) async fn check<
    R: ToolRoute,
    F: Future<Output = Result<(), crate::harness::HarnessError>>,
>(
    name: &str,
    carrier: &R,
    other: &R,
    key: &str,
    spec: CallSpec,
    ready: F,
) -> Result<(), String> {
    let (name, reason) = case_parts(name).ok_or("unknown call-yield case reason")?;
    let call = carrier.request(json!({"name":spec.0,"arguments":spec.1,"call_key":key}));
    let mut call = Box::pin(call);
    match select(call.as_mut(), Box::pin(ready)).await {
        Either::Left(_) => return Err("fixture returned before a yield could be sent".into()),
        Either::Right((ready, _)) => ready.map_err(|e| e.to_string())?,
    }
    let route = if name == "call_yield_non_carrier_unknown_call" {
        other
    } else {
        carrier
    };
    let body = json!({"name":"tool.yield","arguments":{"call_key":key,"reason":reason}});
    let yielding = route.request(body.clone());
    let (answer, early_final) = match select(call.as_mut(), Box::pin(yielding)).await {
        Either::Left((finished, yielding)) => (yielding.await, Some(finished)),
        Either::Right((answer, _)) => (answer, None),
    };
    let answer: YieldAnswer = serde_json::from_value(response(answer.map_err(|e| e.message)?)?)
        .map_err(|e| e.to_string())?;
    if name == "call_yield_non_carrier_unknown_call" {
        return if answer == YieldAnswer::UnknownCall {
            Ok(())
        } else {
            Err(format!("non-carrier yield answered {answer:?}"))
        };
    }
    if name == "call_yield_untagged_not_yieldable" {
        return if matches!(answer, YieldAnswer::NotYieldable { .. }) {
            Ok(())
        } else {
            Err(format!("untagged call yielded: {answer:?}"))
        };
    }
    if answer != YieldAnswer::Accepted {
        return Err(format!("yieldable fixture did not accept: {answer:?}"));
    }
    let final_frame = match early_final {
        Some(frame) => frame,
        None => tokio::time::timeout(Duration::from_millis(CALL_YIELD_DEADLINE_MS), call)
            .await
            .map_err(|_| "accepted yield sent no final frame within 5 s")?,
    }
    .map_err(|e| e.message)?;
    let value = response(final_frame)?;
    if let Some(held) = value.get("held") {
        if held["reason"] != "running" || held["yielded"] != true {
            return Err("yielded held reply lacks running/true".into());
        }
        let result: ToolResult = serde_json::from_value(value).map_err(|e| e.to_string())?;
        validate(&result).map_err(|e| e.to_string())?;
    }
    if name == "call_yield_repeat_unknown_call" {
        let repeated: YieldAnswer = serde_json::from_value(response(
            carrier.request(body).await.map_err(|e| e.message)?,
        )?)
        .map_err(|e| e.to_string())?;
        if repeated != YieldAnswer::UnknownCall {
            return Err(format!("repeated yield answered {repeated:?}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Exchange, RouteFailure};
    use async_trait::async_trait;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[derive(Clone, Copy, Default)]
    enum Fault {
        #[default]
        None,
        NoFinal,
        Untagged,
        Repeat,
        NonCarrier,
        PersonOnly,
    }
    struct Provider {
        open: AtomicBool,
        final_ready: AtomicBool,
        tagged: bool,
        fault: Fault,
    }
    struct Route {
        provider: Arc<Provider>,
        carrier: bool,
    }
    fn reply(value: Value) -> Result<Exchange, RouteFailure> {
        Ok(Exchange {
            frames: vec![ObservedFrame::Response(value)],
        })
    }
    #[async_trait]
    impl ToolRoute for Route {
        async fn request(&self, body: Value) -> Result<Exchange, RouteFailure> {
            let p = &self.provider;
            if body["name"] == "tool.yield" {
                if !self.carrier && !matches!(p.fault, Fault::NonCarrier) {
                    return reply(json!({"answer":"unknown_call"}));
                }
                if !p.tagged && !matches!(p.fault, Fault::Untagged) {
                    return reply(json!({"answer":"not_yieldable"}));
                }
                if p.final_ready.load(Ordering::SeqCst) && !matches!(p.fault, Fault::Repeat) {
                    return reply(json!({"answer":"unknown_call"}));
                }
                if matches!(p.fault, Fault::PersonOnly)
                    && body["arguments"]["reason"] != "person_message"
                {
                    return reply(
                        json!({"answer":"not_yieldable","reason":"reason_not_supported"}),
                    );
                }
                if !matches!(p.fault, Fault::NoFinal) {
                    p.final_ready.store(true, Ordering::SeqCst);
                }
                return reply(json!({"answer":"accepted"}));
            }
            p.open.store(true, Ordering::SeqCst);
            while !p.final_ready.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            reply(
                json!({"content":[{"type":"text","text":"Work continues."}],"held":{"reason":"running","yielded":true}}),
            )
        }
        async fn request_then_cancel(&self, body: Value) -> Result<Exchange, RouteFailure> {
            self.request(body).await
        }
    }
    async fn run(name: &str, fault: Fault) -> Result<(), String> {
        let provider = Arc::new(Provider {
            open: AtomicBool::new(false),
            final_ready: AtomicBool::new(false),
            tagged: case_parts(name).unwrap().0 != "call_yield_untagged_not_yieldable",
            fault,
        });
        let carrier = Route {
            provider: provider.clone(),
            carrier: true,
        };
        let other = Route {
            provider: provider.clone(),
            carrier: false,
        };
        check(
            name,
            &carrier,
            &other,
            "call-1",
            ("work".into(), json!({})),
            async {
                while !provider.open.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Ok(())
            },
        )
        .await
        .map_err(|error| format!("{name}: {error}"))
    }
    #[tokio::test]
    async fn call_yield_faulty_providers_fail_each_named_case() {
        for (case, fault) in [
            ("call_yield_accepted_final_deadline", Fault::NoFinal),
            ("call_yield_untagged_not_yieldable", Fault::Untagged),
            ("call_yield_repeat_unknown_call", Fault::Repeat),
            ("call_yield_non_carrier_unknown_call", Fault::NonCarrier),
        ] {
            for reason in crate::report::CALL_YIELD_REASONS {
                let name = case_name(case, reason);
                assert!(crate::CASES.iter().any(|spec| spec.name == name));
                run(&name, Fault::None).await.unwrap();
                assert!(run(&name, fault).await.unwrap_err().starts_with(&name));
            }
        }
    }

    #[tokio::test]
    async fn call_yield_reasons_reject_person_only_provider() {
        for reason in ["person_message", "urgent_notice"] {
            let name = case_name("call_yield_accepted_final_deadline", reason);
            assert!(
                crate::CASES.iter().any(|spec| spec.name == name),
                "missing {name}"
            );
            run(&name, Fault::None).await.unwrap();
            let result = run(&name, Fault::PersonOnly).await;
            if reason == "person_message" {
                result.unwrap();
            } else {
                let error = result.unwrap_err();
                assert!(error.starts_with(&name), "{error}");
                assert!(error.contains("reason_not_supported"), "{error}");
            }
        }
    }

    fn case_name(case: &str, reason: &str) -> String {
        if reason == "person_message" {
            case.to_owned()
        } else {
            format!("{case}_{reason}")
        }
    }

    #[test]
    fn call_yield_each_reason_has_all_cases() {
        for reason in crate::report::CALL_YIELD_REASONS {
            for case in [
                "call_yield_accepted_final_deadline",
                "call_yield_untagged_not_yieldable",
                "call_yield_repeat_unknown_call",
                "call_yield_non_carrier_unknown_call",
            ] {
                let name = case_name(case, reason);
                assert!(
                    crate::CASES.iter().any(|spec| spec.name == name),
                    "missing {name}"
                );
                assert_eq!(case_parts(&name), Some((case, *reason)));
            }
        }
    }
}
