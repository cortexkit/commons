use super::*;

const CHECK: &str = "select_harness_capabilities_matched";
const UNDECLARED: &str = "undeclared_select_harness_capabilities_refused";

fn runner_declaration(
    principal: &str,
    harness: &str,
    capabilities: Option<&[&str]>,
) -> CatalogModule {
    let limits = capabilities.map_or_else(|| json!({}), |names| json!({"capabilities": names}));
    CatalogModule {
        principal: principal.into(),
        nonce_verified: true,
        role_declarations: [(
            crate::wire::ROLE_DECLARATIONS_KEY.into(),
            json!({"runner": {
                "harnesses": [harness],
                "default_harness": harness,
                "harness_limits": {harness: limits}
            }}),
        )]
        .into(),
    }
}

fn request(capabilities: &[&str]) -> Value {
    json!({
        "consumer": SUMMARY,
        "requirements": {"iq": IQ, "harness_capabilities": capabilities},
        "runners": [fixture::RUNNER_ONE, fixture::RUNNER_TWO]
    })
}

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn harness_capabilities_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let group = capabilities::SELECT_HARNESS_CAPABILITIES;
        let declared = describe.serves(group);
        if (name == CHECK && !declared) || (name == UNDECLARED && declared) {
            return Ok(Ending::NotApplicable(format!(
                "role.describe {} {group}",
                if declared {
                    "declares"
                } else {
                    "does not declare"
                }
            )));
        }
        if name == UNDECLARED {
            let refusal = refused(
                &route,
                ops::ROUTE_SELECT,
                &request(&["submit_tool"]),
                RefusalCode::InvalidRequirements,
                "harness_capabilities without its group",
            )
            .await?;
            if refusal.detail.get("field") != Some(&json!("harness_capabilities")) {
                return Err(format!(
                    "undeclared harness_capabilities must name its requirement field: {refusal:?}"
                ));
            }
            if describe.serves(capabilities::SELECT_PANEL) {
                self.subject.configure_panel_class("audit", 1, 1, 0).await?;
                let params = json!({
                    "class": "audit",
                    "consumer": SUMMARY,
                    "requirements": {"harness_capabilities": ["submit_tool"]}
                });
                let refusal = refused(
                    &route,
                    ops::ROUTE_SELECT_PANEL,
                    &params,
                    RefusalCode::InvalidRequest,
                    "panel harness_capabilities without its group",
                )
                .await?;
                if refusal.detail.get("field") != Some(&json!("requirements.harness_capabilities"))
                {
                    return Err(format!(
                        "undeclared panel capability must name its requirement path: {refusal:?}"
                    ));
                }
            }
            return Ok(Ending::Passed);
        }
        if name != CHECK {
            return Err(format!("unknown harness-capability check {name}"));
        }

        let mut catalog = fixture::catalog();
        catalog.runners[1].harnesses = Some(vec!["beta".into()]);
        catalog.runners[1].default_harness = Some("beta".into());
        self.subject.configure_catalog(&catalog).await?;
        let full = [
            "one_off",
            "output_cap",
            "submit_tool",
            "final_text",
            "plans",
        ];
        self.subject
            .configure_catalog_declarations(&[
                runner_declaration(fixture::RUNNER_ONE, "alpha", None),
                runner_declaration(fixture::RUNNER_TWO, "beta", Some(&full)),
            ])
            .await?;

        // A harness with no capability declaration cannot satisfy an explicit
        // name, even though its runner and model are otherwise eligible.
        let selected = select(
            &route,
            &request(&["submit_tool", "final_text"]),
            "explicit harness capabilities",
        )
        .await?;
        if selected.runner.module != fixture::RUNNER_TWO {
            return Err(format!(
                "explicit capabilities selected undeclared runner-one/alpha: {selected:?}"
            ));
        }
        if describe.serves(capabilities::SELECT_PANEL) {
            self.subject.configure_panel_class("audit", 2, 1, 1).await?;
            let params = json!({
                "class": "audit",
                "consumer": SUMMARY,
                "requirements": {"harness_capabilities": ["submit_tool", "final_text"]},
                "runners": [fixture::RUNNER_ONE, fixture::RUNNER_TWO]
            });
            let panel: crate::wire::panel::SelectPanelAnswer = answer(
                &route,
                ops::ROUTE_SELECT_PANEL,
                &params,
                "panel harness capabilities",
            )
            .await?;
            if panel.seats.is_empty()
                || panel
                    .seats
                    .iter()
                    .any(|seat| seat.answer.runner.module != fixture::RUNNER_TWO)
            {
                return Err(format!(
                    "every panel seat must use the runner/harness pair with the explicit names: {panel:?}"
                ));
            }
        }

        // runner-one has the explicit name and output_cap, but lacks the
        // one_off inferred from tools none. Choosing it would drop one_off.
        self.subject
            .configure_catalog_declarations(&[
                runner_declaration(fixture::RUNNER_ONE, "alpha", Some(&["plans", "output_cap"])),
                runner_declaration(fixture::RUNNER_TWO, "beta", Some(&full)),
            ])
            .await?;
        let mut inferred = request(&["plans"]);
        inferred["requirements"]["tools"] = json!("none");
        let selected = select(&route, &inferred, "inferred one_off").await?;
        if selected.runner.module != fixture::RUNNER_TWO {
            return Err(format!(
                "tools none was not added to the capability union: {selected:?}"
            ));
        }

        // Keep one_off but remove output_cap. A present output floor must
        // exclude runner-one even when its value is zero.
        self.subject
            .configure_catalog_declarations(&[
                runner_declaration(fixture::RUNNER_ONE, "alpha", Some(&["plans", "one_off"])),
                runner_declaration(fixture::RUNNER_TWO, "beta", Some(&full)),
            ])
            .await?;
        let mut inferred = request(&["plans"]);
        inferred["requirements"]["max_output_tokens"] = json!(0);
        let selected = select(&route, &inferred, "inferred output_cap").await?;
        if selected.runner.module != fixture::RUNNER_TWO {
            return Err(format!(
                "a present output floor was not added to the capability union: {selected:?}"
            ));
        }

        // Neither pair declares fork_answer, and runner-one also lacks the
        // inferred output_cap. The diagnostic lists exactly those missing names.
        let mut unavailable = request(&["fork_answer"]);
        unavailable["requirements"]["max_output_tokens"] = json!(0);
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &unavailable,
            RefusalCode::NoRoute,
            "no pair with every explicit and inferred capability",
        )
        .await?;
        if refusal.detail.get("reason") != Some(&json!("harness_unavailable"))
            || refusal.detail.contains_key("reset_at_ms")
        {
            return Err(format!(
                "capability exhaustion must be untimed harness_unavailable: {refusal:?}"
            ));
        }
        let missing = refusal
            .detail
            .get("missing")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("capability refusal has no detail.missing map: {refusal:?}"))?;
        if missing.len() != 2 {
            return Err(format!(
                "detail.missing must name both rejected pairs, got {missing:?}"
            ));
        }
        for (pair, mut expected) in [
            ("runner-one/alpha", vec!["fork_answer", "output_cap"]),
            ("runner-two/beta", vec!["fork_answer"]),
        ] {
            let mut actual: Vec<String> = serde_json::from_value(missing[pair].clone())
                .map_err(|error| format!("detail.missing[{pair:?}] is not a name list: {error}"))?;
            actual.sort();
            expected.sort_unstable();
            if actual != expected {
                return Err(format!(
                    "detail.missing[{pair:?}] is {actual:?}, expected {expected:?}"
                ));
            }
        }

        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &request(&["future_task"]),
            RefusalCode::InvalidRequirements,
            "unknown harness capability",
        )
        .await?;
        if refusal.detail.get("field") != Some(&json!("harness_capabilities")) {
            return Err(format!(
                "unknown select capability must name harness_capabilities: {refusal:?}"
            ));
        }
        Ok(Ending::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::select_engine::fake::{Defects, FakeSubject};

    #[tokio::test]
    async fn harness_capabilities_pass_and_follow_group_applicability() {
        let subject = FakeSubject::new(Defects::default());
        assert_eq!(Suite::new(&subject).run(CHECK).await, Ok(Ending::Passed));
        assert!(matches!(
            Suite::new(&subject).run(UNDECLARED).await,
            Ok(Ending::NotApplicable(_))
        ));

        let subject = FakeSubject::without_harness_capabilities_control(Defects::default());
        assert_eq!(
            Suite::new(&subject).run(UNDECLARED).await,
            Ok(Ending::Passed)
        );
        assert!(matches!(
            Suite::new(&subject).run(CHECK).await,
            Ok(Ending::NotApplicable(_))
        ));
    }

    #[tokio::test]
    async fn faulty_harness_capability_routers_are_rejected_by_check_name() {
        let spec = crate::CHECKS
            .iter()
            .find(|spec| spec.name == CHECK)
            .unwrap();
        for fault in [
            "select_harness_capabilities_matched",
            "select_harness_capabilities_inferred",
            "select_harness_capabilities_missing",
            "select_harness_capabilities_unknown",
        ] {
            let subject = FakeSubject::new(Defects {
                extension: Some(fault),
                ..Defects::default()
            });
            let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
            assert_eq!(report.check, CHECK);
            assert!(
                matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                "{fault}: {report:?}"
            );
        }

        let subject = FakeSubject::without_harness_capabilities_control(Defects {
            extension: Some(UNDECLARED),
            ..Defects::default()
        });
        let spec = crate::CHECKS
            .iter()
            .find(|spec| spec.name == UNDECLARED)
            .unwrap();
        let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
        assert_eq!(report.check, UNDECLARED);
        assert!(
            matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
            "{report:?}"
        );
    }
}
