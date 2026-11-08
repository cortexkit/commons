use super::*;

fn eval(model: &str, metric: &str, score: f64, set: &str, time: u64) -> Value {
    json!({"kind":"transcribe", "engine":{"provider":fixture::SUBSCRIPTION_PROVIDER,"model":model},
        "metric":metric,"score":score,"set":{"id":set,"items":20},"measured_at_ms":time})
}

async fn record<R: RoutingRoute>(route: &R, params: &Value) -> Check {
    let reply: Value = answer(
        route,
        ops::ROUTE_ENGINE_EVAL,
        params,
        "reference evaluation",
    )
    .await?;
    if reply != json!({}) {
        return Err(format!(
            "expected empty evaluation acknowledgement, got {reply}"
        ));
    }
    Ok(())
}

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn engine_measurement_check(
        &self,
        name: &str,
        route: &S::Route,
        describe: &RoleDescribe,
    ) -> Result<Ending, String> {
        let group = if name.starts_with("select_engine_speed_") {
            capabilities::ENGINE_REPORTS
        } else {
            capabilities::ENGINE_EVALS
        };
        if !describe.serves(group) {
            return Ok(Ending::NotApplicable(format!(
                "role.describe does not declare {group}"
            )));
        }
        let op = if group == capabilities::ENGINE_REPORTS {
            ops::ROUTE_ENGINE_REPORT
        } else {
            ops::ROUTE_ENGINE_EVAL
        };
        if !describe.ops.iter().any(|declared| declared == op) {
            return Err(format!("{group} must declare {op}"));
        }
        self.subject
            .configure_catalog_declarations(&[module(
                MEDIA,
                vec![
                    entry("a", false),
                    entry("b", false),
                    entry("c", false),
                    entry("a-old", false),
                ],
            )])
            .await?;
        let media = self.route_as(MEDIA).await?;
        let mut params = request();
        params["requirements"]["metric"] = json!("wer");
        let winner = match name {
            "select_engine_eval_measured" => {
                self.subject
                    .configure_engine_facts(&[
                        EngineFacts::new(
                            engine::Kind::Transcribe,
                            fixture::SUBSCRIPTION_PROVIDER,
                            "a",
                        )
                        .with_cost(0),
                        EngineFacts::new(
                            engine::Kind::Transcribe,
                            fixture::SUBSCRIPTION_PROVIDER,
                            "b",
                        )
                        .with_cost(100),
                    ])
                    .await?;
                params["caller_local"] = json!([entry("caller", true)]);
                record(&media, &eval("b", "wer", 0.0, "canonical", 100)).await?;
                "b"
            }
            "select_engine_eval_higher_score" => {
                record(&media, &eval("a", "wer", 0.8, "canonical", 100)).await?;
                record(&media, &eval("b", "wer", 0.1, "canonical", 50)).await?;
                record(&media, &eval("b", "wer", 0.9, "canonical", 100)).await?;
                record(&media, &eval("b", "wer", 0.2, "canonical", 60)).await?;
                "b"
            }
            "select_engine_eval_different_set" => {
                record(&media, &eval("a", "wer", 0.8, "canonical", 200)).await?;
                record(&media, &eval("b", "wer", 0.7, "canonical", 200)).await?;
                record(&media, &eval("c", "wer", 0.99, "unrelated", 100)).await?;
                "a"
            }
            "select_engine_eval_new_model" => {
                record(&media, &eval("a-old", "wer", 1.0, "canonical", 100)).await?;
                record(&media, &eval("b", "wer", 0.5, "canonical", 100)).await?;
                self.subject
                    .configure_catalog_declarations(&[module(
                        MEDIA,
                        vec![entry("a-new", false), entry("b", false)],
                    )])
                    .await?;
                "b"
            }
            "select_engine_eval_default_metric" | "select_engine_eval_requested_metric" => {
                // Arrival order deliberately disagrees with the per-kind default.
                record(
                    &media,
                    &eval("a", "named_term_recall", 1.0, "canonical", 100),
                )
                .await?;
                record(
                    &media,
                    &eval("b", "named_term_recall", 0.1, "canonical", 100),
                )
                .await?;
                record(&media, &eval("a", "wer", 0.1, "canonical", 100)).await?;
                record(&media, &eval("b", "wer", 1.0, "canonical", 100)).await?;
                if name == "select_engine_eval_default_metric" {
                    params["requirements"] = json!({});
                    "b"
                } else {
                    params["requirements"]["metric"] = json!("named_term_recall");
                    "a"
                }
            }
            "select_engine_eval_newest_set" => {
                record(&media, &eval("a", "wer", 1.0, "old", 100)).await?;
                record(&media, &eval("b", "wer", 0.9, "old", 100)).await?;
                record(&media, &eval("a", "wer", 0.1, "new", 200)).await?;
                record(&media, &eval("b", "wer", 0.2, "new", 200)).await?;
                "b"
            }
            "select_engine_eval_authorization" => {
                let body = eval("b", "wer", 0.9, "canonical", 100);
                record(&media, &body).await?;
                let foreign = self.route_as(OTHER_MEDIA).await?;
                refused(
                    &foreign,
                    ops::ROUTE_ENGINE_EVAL,
                    &body,
                    RefusalCode::EngineNotDeclaredByCaller,
                    "foreign evaluation",
                )
                .await?;
                return Ok(Ending::Passed);
            }
            "select_engine_eval_score_range" => {
                for score in [-0.01, 1.01] {
                    let refusal = refused(
                        &media,
                        ops::ROUTE_ENGINE_EVAL,
                        &eval("b", "wer", score, "canonical", 100),
                        RefusalCode::InvalidRequest,
                        "out-of-range score",
                    )
                    .await?;
                    if refusal.detail.get("field") != Some(&json!("score")) {
                        return Err("score refusal must name score".into());
                    }
                }
                for score in [0.0, 1.0] {
                    record(&media, &eval("b", "wer", score, "canonical", 100)).await?;
                }
                return Ok(Ending::Passed);
            }
            "select_engine_speed_authorization" => {
                let decision = selected(route, &params).await?;
                let body = json!({"decision_id":decision.decision_id,"kind":"transcribe", "engine":{"provider":decision.engine.provider,"model":decision.engine.model},"outcome":"done","input":{"duration_ms":1000,"bytes":4096},"wall_ms":250,"first_output_ms":20});
                let reply: Value =
                    answer(&media, ops::ROUTE_ENGINE_REPORT, &body, "job speed report").await?;
                if reply != json!({}) {
                    return Err("expected empty speed acknowledgement".into());
                }
                let foreign = self.route_as(OTHER_MEDIA).await?;
                refused(
                    &foreign,
                    ops::ROUTE_ENGINE_REPORT,
                    &body,
                    RefusalCode::EngineNotDeclaredByCaller,
                    "foreign speed report",
                )
                .await?;
                params["privacy"] = json!("local_only");
                params["caller_local"] = json!([entry("a", true)]);
                let local = selected(route, &params).await?;
                let mut local_body = body;
                local_body["decision_id"] = json!(local.decision_id);
                local_body["engine"]["model"] = json!(local.engine.model);
                refused(
                    &media,
                    ops::ROUTE_ENGINE_REPORT,
                    &local_body,
                    RefusalCode::UnknownDecision,
                    "caller-local speed report",
                )
                .await?;
                return Ok(Ending::Passed);
            }
            _ => return Err(format!("unknown engine measurement check {name}")),
        };
        expect_engine(route, &params, winner, Some(MEDIA), None).await?;
        Ok(Ending::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Defects, FakeSubject};
    use super::*;

    macro_rules! measurement_case {
        ($name:ident) => {
            #[tokio::test]
            async fn $name() {
                let name = stringify!($name);
                let good = FakeSubject::new(Defects::default());
                assert_eq!(Suite::new(&good).run(name).await, Ok(Ending::Passed));
                let bad = FakeSubject::new(Defects {
                    engine_fault: name,
                    ..Defects::default()
                });
                let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
                let report = crate::runner::run_check(&Suite::new(&bad), spec).await;
                assert!(
                    matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                    "fault was not rejected: {report:?}"
                );
                assert_eq!(report.check, name);
                let absent = FakeSubject::new(Defects {
                    engine_fault: "without_measurements",
                    ..Defects::default()
                });
                assert!(matches!(
                    Suite::new(&absent).run(name).await,
                    Ok(Ending::NotApplicable(_))
                ));
            }
        };
    }
    measurement_case!(select_engine_eval_measured);
    measurement_case!(select_engine_eval_higher_score);
    measurement_case!(select_engine_eval_different_set);
    measurement_case!(select_engine_eval_new_model);
    measurement_case!(select_engine_eval_default_metric);
    measurement_case!(select_engine_eval_requested_metric);
    measurement_case!(select_engine_eval_newest_set);
    measurement_case!(select_engine_eval_authorization);
    measurement_case!(select_engine_eval_score_range);
    measurement_case!(select_engine_speed_authorization);
}
