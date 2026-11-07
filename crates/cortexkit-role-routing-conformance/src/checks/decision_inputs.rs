use super::*;
use crate::subject::ModelInputFacts;

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn decision_input_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let fields: &[&str] = if name == "questions_requirement_honoured" {
            &["questions"]
        } else {
            &["input_tokens", "max_cost_usd"]
        };
        for field in fields {
            if !describe
                .requirement_fields
                .iter()
                .any(|listed| listed == field)
            {
                return Ok(Ending::NotApplicable(format!(
                    "router does not list requirement {field}"
                )));
            }
        }
        let catalog = fixture::catalog();
        let expected = catalog.models[0].route_key();
        let facts: Vec<_> = catalog
            .models
            .iter()
            .enumerate()
            .map(|(index, model)| {
                ModelInputFacts::new(
                    model.route_key(),
                    Some(if index == 0 { 2 } else { 1 }),
                    if index == 0 { Some(2.into()) } else { None },
                )
            })
            .collect();
        if !self.subject.configure_model_input_facts(&facts).await? {
            return Ok(Ending::NotApplicable(
                "subject has no model-input facts hook".into(),
            ));
        }
        let mut params = base();
        params["requirements"] = if name == "questions_requirement_honoured" {
            json!({"iq":0, "questions":2})
        } else {
            json!({"iq":0, "input_tokens":1000, "max_cost_usd":0.002})
        };
        let selected = select(
            &route,
            &params,
            "decision input requirement at the boundary",
        )
        .await?;
        if selected.route.route_key() != expected {
            return Err(format!(
                "{name} selected {} rather than eligible {expected}",
                selected.route.route_key()
            ));
        }
        if name == "questions_requirement_honoured" {
            params["requirements"]["questions"] = json!(3);
        } else {
            params["requirements"]["max_cost_usd"] = json!(0.001);
        }
        refused(
            &route,
            ops::ROUTE_SELECT,
            &params,
            RefusalCode::NoRoute,
            name,
        )
        .await?;
        if name == "max_cost_usd_requirement_honoured" {
            let unknown_prices: Vec<_> = catalog
                .models
                .iter()
                .map(|model| ModelInputFacts::new(model.route_key(), Some(2), None))
                .collect();
            if !self
                .subject
                .configure_model_input_facts(&unknown_prices)
                .await?
            {
                return Err("model-input facts hook disappeared".into());
            }
            params["requirements"]["max_cost_usd"] = json!(1);
            refused(
                &route,
                ops::ROUTE_SELECT,
                &params,
                RefusalCode::NoRoute,
                "unstated input prices under a cap",
            )
            .await?;
        }
        Ok(Ending::Passed)
    }
}
