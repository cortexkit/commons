use super::*;

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn harness_limits_check(&self, name: &str) -> Result<Ending, String> {
        let mut catalog = fixture::catalog();
        catalog.runners.truncate(1);
        self.subject.configure_catalog(&catalog).await?;
        self.subject
            .configure_catalog_declarations(&[CatalogModule {
                principal: fixture::RUNNER_ONE.into(),
                nonce_verified: true,
                role_declarations: [(
                    crate::wire::ROLE_DECLARATIONS_KEY.into(),
                    json!({"runner": {
                        "harnesses": ["alpha", "beta"], "default_harness": "alpha",
                        "harness_limits": {"alpha": {"starts_sessions": false}, "typo": {}}
                    }}),
                )]
                .into(),
            }])
            .await?;
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let selected = select(&route, &base(), name).await?;
        if selected.runner.module != fixture::RUNNER_ONE {
            return Err(format!("{name}: invalid map discarded the runner"));
        }
        if describe.serves(SELECT_HARNESS) {
            for harness in ["alpha", "beta"] {
                let params = with(base(), "harness", json!(harness));
                let selected =
                    select(&route, &params, "invalid map falls back for every harness").await?;
                if selected.runner.harness.as_deref() != Some(harness) {
                    return Err(format!("requested {harness} must be hosted and echoed"));
                }
            }
        }
        Ok(Ending::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::select_engine::fake::{Defects, FakeSubject};

    #[tokio::test]
    async fn invalid_harness_limits_fall_back_instead_of_discarding_the_runner() {
        let name = "harness_limits_invalid_falls_back";
        let subject = FakeSubject::new(Defects::default());
        assert_eq!(Suite::new(&subject).run(name).await, Ok(Ending::Passed));
        let faulty = FakeSubject::new(Defects {
            extension: Some(name),
            ..Defects::default()
        });
        let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
        let report = crate::runner::run_check(&Suite::new(&faulty), spec).await;
        assert_eq!(report.check, name);
        assert!(matches!(report.outcome, crate::CheckOutcome::Failed { .. }));
    }
}
