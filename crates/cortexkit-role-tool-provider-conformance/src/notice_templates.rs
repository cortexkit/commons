//! Catalog opt-in and provider bounds only, not runner delivery or composition.

use crate::{single_terminal, ObservedFrame, ToolRoute};
use cortexkit_role_tool_provider::{catalog::CatalogAnswer, errors::invalid_request_field};
use serde_json::{json, Value};

const CASE: &str = "catalog_notice_templates";

async fn catalog<R: ToolRoute>(route: &R, arguments: Value) -> Result<ObservedFrame, String> {
    let exchange = route
        .request(json!({"name":"tool.catalog", "arguments":arguments}))
        .await
        .map_err(|error| format!("{CASE}: {}", error.message))?;
    single_terminal(&exchange)
        .cloned()
        .map_err(|error| format!("{CASE}: {error}"))
}

pub(crate) async fn check<R: ToolRoute>(
    route: &R,
    mut arguments: Value,
    declares: bool,
) -> Result<(), String> {
    let request = arguments
        .as_object_mut()
        .ok_or_else(|| format!("{CASE}: catalog arguments must be an object"))?;
    request.remove("notice_templates");
    request.remove("digest_only");
    if let Some(params) = request.get_mut("params").and_then(Value::as_object_mut) {
        params.remove("capabilities");
    }
    let ObservedFrame::Response(unasked) = catalog(route, arguments.clone()).await? else {
        return Err(format!("{CASE}: unasked catalog must answer normally"));
    };
    if unasked.get("notice_templates").is_some() {
        return Err(format!("{CASE}: unasked answer carries notice_templates"));
    }
    arguments["notice_templates"] = json!(true);
    match catalog(route, arguments).await? {
        ObservedFrame::Response(answer) if declares => {
            let decoded: CatalogAnswer =
                serde_json::from_value(answer).map_err(|error| format!("{CASE}: {error}"))?;
            if decoded.notice_templates.is_none() {
                return Err(format!("{CASE}: requested answer omits notice_templates"));
            }
            Ok(())
        }
        ObservedFrame::Error(error)
            if !declares && invalid_request_field(&error) == Some("notice_templates") =>
        {
            Ok(())
        }
        _ => Err(format!(
            "{CASE}: expected {}",
            if declares {
                "a catalog response with notice_templates"
            } else {
                "invalid_request naming notice_templates from a non-declarer"
            }
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Exchange, RouteFailure};
    use async_trait::async_trait;
    use cortexkit_role_tool_provider::{
        catalog::{check_notice_templates_request, CatalogRequest},
        errors,
        vectors::CATALOG_NOTICE_TEMPLATES_JSON,
    };

    struct Provider {
        declares: bool,
        answer: Value,
        unasked: bool,
        accepts_unsupported: bool,
    }

    #[async_trait]
    impl ToolRoute for Provider {
        async fn request(&self, body: Value) -> Result<Exchange, RouteFailure> {
            let asked = body["arguments"]["notice_templates"] == true;
            let frame = if asked && !self.declares && !self.accepts_unsupported {
                ObservedFrame::Error(errors::invalid_request("notice_templates", "unsupported"))
            } else if asked || self.unasked {
                ObservedFrame::Response(self.answer.clone())
            } else {
                ObservedFrame::Response(
                    json!({"generation":"g1", "catalog_digest":"d1", "tools":[]}),
                )
            };
            Ok(Exchange {
                frames: vec![frame],
            })
        }

        async fn request_then_cancel(&self, body: Value) -> Result<Exchange, RouteFailure> {
            self.request(body).await
        }
    }

    fn provider(answer: Value) -> Provider {
        Provider {
            declares: true,
            answer,
            unasked: false,
            accepts_unsupported: false,
        }
    }

    #[tokio::test]
    async fn notice_templates_vectors_reject_faulty_providers_by_case_name() {
        let vectors: Value = serde_json::from_str(CATALOG_NOTICE_TEMPLATES_JSON).unwrap();
        for case in vectors["answers"].as_array().unwrap() {
            check(
                &provider(case["answer"].clone()),
                json!({"params":{}}),
                true,
            )
            .await
            .unwrap();
        }
        for case in vectors["refused_answers"].as_array().unwrap() {
            let error = check(
                &provider(case["answer"].clone()),
                json!({"params":{}}),
                true,
            )
            .await
            .unwrap_err();
            assert!(error.starts_with(CASE), "{error}");
            assert!(
                error.starts_with(&format!("{CASE}: {}:", case["field"].as_str().unwrap())),
                "{}: {error}",
                case["name"]
            );
        }
        for case in vectors["requests"].as_array().unwrap() {
            let request: CatalogRequest = serde_json::from_value(case["request"].clone()).unwrap();
            let provides: Vec<String> = serde_json::from_value(case["provides"].clone()).unwrap();
            check_notice_templates_request(&request, &provides).unwrap();
            assert_eq!(serde_json::to_value(request).unwrap(), case["request"]);
        }
        for case in vectors["refused_requests"].as_array().unwrap() {
            let request: CatalogRequest = serde_json::from_value(case["request"].clone()).unwrap();
            let provides: Vec<String> = serde_json::from_value(case["provides"].clone()).unwrap();
            let error = check_notice_templates_request(&request, &provides).unwrap_err();
            assert_eq!(error.code, case["error"]["code"].as_str().unwrap());
            assert_eq!(
                invalid_request_field(&error),
                case["error"]["detail"]["field"].as_str()
            );
        }
    }

    #[tokio::test]
    async fn notice_templates_opt_in_faults_fail_by_case_name() {
        let answer =
            json!({"generation":"g1", "catalog_digest":"d1", "tools":[], "notice_templates":[]});
        let mut faulty = provider(answer.clone());
        faulty.unasked = true;
        let error = check(&faulty, json!({"params":{}}), true)
            .await
            .unwrap_err();
        assert!(
            error.starts_with(CASE) && error.contains("unasked"),
            "{error}"
        );
        let missing = provider(json!({"generation":"g1", "catalog_digest":"d1", "tools":[]}));
        let error = check(&missing, json!({"params":{}}), true)
            .await
            .unwrap_err();
        assert!(
            error.starts_with(CASE) && error.contains("omits"),
            "{error}"
        );
        let mut unsupported = provider(answer);
        unsupported.declares = false;
        check(&unsupported, json!({"params":{}}), false)
            .await
            .unwrap();
        unsupported.accepts_unsupported = true;
        let error = check(&unsupported, json!({"params":{}}), false)
            .await
            .unwrap_err();
        assert!(
            error.starts_with(CASE) && error.contains("non-declarer"),
            "{error}"
        );
    }
}
