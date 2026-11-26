use super::*;

const ABSENT: &str = "runner_serves_absent_harness";
const EMPTY: &str = "runner_serves_empty_harness";
const EXCEPT: &str = "runner_serves_except";
const LOGIN_NONE: &str = "runner_serves_login_none";

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn serves_check(&self, name: &str) -> Result<Ending, String> {
        let mut catalog = fixture::catalog();
        catalog.runners.truncate(1);
        catalog.runners[0].route_keys.clear();
        catalog.runners[0].harnesses = Some(vec![fixture::HARNESS.into()]);
        // No fixture login can be mistaken for a usable login for this provider.
        catalog.accounts.clear();
        catalog.models.truncate(2);
        let model = &catalog.models[0];
        let key = model.route_key();
        let req = with(base(), "pin", json!(key));
        let allowed = json!([{"provider": model.provider, "models": "all", "login": "none"}]);
        let ready =
            |generation: &str| json!({"generation": generation, "harnesses": {"alpha": allowed}});
        let missing = |generation: &str| json!({"generation": generation, "harnesses": {}});
        self.subject.configure_catalog(&catalog).await?;
        let initial = match name {
            ABSENT => missing("one"),
            EMPTY => ready("one"),
            EXCEPT => json!({"generation": "one", "harnesses": {"alpha": [
                {"provider": model.provider, "models": "all", "except": [model.model], "login": "none"}
            ]}}),
            LOGIN_NONE => json!({"generation": "one", "harnesses": {"alpha": [
                 {"provider": model.provider, "models": "all"}
            ]}}),
            _ => unreachable!("unknown serves check {name}"),
        };
        if !self
            .subject
            .configure_runner_serves(fixture::RUNNER_ONE, &initial)
            .await?
        {
            return Ok(Ending::NotApplicable(
                "configure_runner_serves is unsupported".into(),
            ));
        }
        let route = self.route_as(CONSUMER).await?;
        let no_pin = |context: &'static str| async {
            refused(
                &route,
                ops::ROUTE_SELECT,
                &req,
                RefusalCode::PinUnavailable,
                context,
            )
            .await
        };
        let refresh = |snapshot: Value| async move {
            if !self
                .subject
                .configure_runner_serves(fixture::RUNNER_ONE, &snapshot)
                .await?
            {
                return Err("configure_runner_serves stopped supporting updates".into());
            }
            Ok::<(), String>(())
        };
        match name {
            ABSENT => {
                no_pin("a not-ready harness places nothing").await?;
                refresh(ready("two")).await?;
                select(&route, &req, "a list arrived for the default harness").await?;
                refresh(missing("three")).await?;
                select(&route, &req, "a missing refresh keeps the last good list").await?;
            }
            EMPTY => {
                select(&route, &req, "the initial list places the model").await?;
                refresh(json!({"generation": "two", "harnesses": {"alpha": []}})).await?;
                no_pin("an explicitly empty list replaces the last good list").await?;
            }
            EXCEPT => {
                no_pin("an excepted catalog model is not placed").await?;
                let other = with(base(), "pin", json!(catalog.models[1].route_key()));
                select(&route, &other, "all still places unexcepted catalog models").await?;
            }
            LOGIN_NONE => {
                no_pin("a matching model still needs a login when login is absent").await?;
                refresh(ready("two")).await?;
                select(&route, &req, "login none places without a usable login").await?;
            }
            _ => unreachable!(),
        }
        Ok(Ending::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::select_engine::fake::{Defects, FakeSubject};

    #[tokio::test]
    async fn runner_serves_checks_reject_faulty_routers_by_name() {
        for (name, fault) in [
            (ABSENT, ABSENT),
            (ABSENT, "serves_forget_missing"),
            (EMPTY, EMPTY),
            (EXCEPT, EXCEPT),
            (LOGIN_NONE, LOGIN_NONE),
        ] {
            assert_eq!(
                Suite::new(&FakeSubject::new(Defects::default()))
                    .run(name)
                    .await,
                Ok(Ending::Passed),
                "{name}"
            );
            let faulty = FakeSubject::new(Defects {
                extension: Some(fault),
                ..Defects::default()
            });
            let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
            let report = crate::runner::run_check(&Suite::new(&faulty), spec).await;
            assert_eq!(report.check, name);
            assert!(
                matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                "{fault}: {report:?}"
            );
        }
    }

    #[tokio::test]
    async fn runner_serves_unsupported_hook_is_skipped() {
        let subject = FakeSubject::new(Defects {
            runner_serves_unsupported: true,
            ..Defects::default()
        });
        for name in [ABSENT, EMPTY, EXCEPT, LOGIN_NONE] {
            assert!(
                matches!(Suite::new(&subject).run(name).await, Ok(Ending::NotApplicable(reason))
                if reason.contains("configure_runner_serves")),
                "{name}"
            );
        }
    }
}
