//! Fixtures shared by the optional consumer and placement checks.

use crate::wire::price::RatesPerMtok;
use crate::{fixture, SelectionFacts};

pub(crate) fn facts() -> SelectionFacts {
    let mut facts = SelectionFacts::new().with_caller_runner(fixture::RUNNER_TWO);
    facts.kinds.insert(
        format!("{}/model-b", fixture::PRICED_PROVIDER),
        "search".into(),
    );
    facts.list_rates.insert(
        format!("{}/model-a", fixture::PRICED_PROVIDER),
        RatesPerMtok::new(4.into(), 20.into())
            .with_cache_read(1.into())
            .with_cache_write(5.into())
            .with_cache_write_ttl([("5m".into(), 5.into()), ("1h".into(), 8.into())].into()),
    );
    let subscription = format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER);
    facts.list_rates.insert(
        subscription.clone(),
        RatesPerMtok::new(20.into(), 100.into()).with_cache_write_ttl(
            [
                ("30m".into(), 30.into()),
                ("12h".into(), 40.into()),
                ("24h".into(), 50.into()),
            ]
            .into(),
        ),
    );
    facts.multipliers.insert(subscription.clone(), 20.into());
    facts.estimates.insert(subscription, 1.into());
    facts
}
