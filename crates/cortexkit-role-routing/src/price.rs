//! Selection prices estimate value, never money spent. The caller settles
//! usage against these rates; the runner reports usage only.

use serde::{Deserialize, Serialize};
use serde_json::Number;
use std::collections::BTreeMap;

open_set! {
    /// Closed producer price bases; record readers retain unfamiliar strings.
    pub enum PriceBasis {
        Metered => "metered",
        SubscriptionEquivalent => "subscription_equivalent",
    }
}

/// USD-equivalent rates per million tokens, using the contract's JSON numbers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RatesPerMtok {
    pub input: Number,
    pub output: Number,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<Number>,
    /// Catalog lifetime strings, verbatim. A lifetime bucket never falls back
    /// to the plain cache-write rate when its entry is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_ttl: Option<BTreeMap<String, Number>>,
}

impl RatesPerMtok {
    pub fn new(input: Number, output: Number) -> Self {
        Self {
            input,
            output,
            cache_read: None,
            cache_write: None,
            cache_write_ttl: None,
        }
    }
    pub fn with_cache_read(mut self, rate: Number) -> Self {
        self.cache_read = Some(rate);
        self
    }
    pub fn with_cache_write(mut self, rate: Number) -> Self {
        self.cache_write = Some(rate);
        self
    }
    pub fn with_cache_write_ttl(mut self, rates: BTreeMap<String, Number>) -> Self {
        self.cache_write_ttl = Some(rates);
        self
    }

    pub fn cache_write_rate(&self, lifetime: Option<&str>) -> Option<&Number> {
        match lifetime {
            Some(ttl) => self.cache_write_ttl.as_ref()?.get(ttl),
            None => self.cache_write.as_ref(),
        }
    }
}

/// Metered list rates or subscription list rates divided by the stated value
/// multiplier. Plan pressure is not a price and never changes these rates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Price {
    pub basis: PriceBasis,
    pub rates_per_mtok: RatesPerMtok,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<Number>,
    /// Per-task estimate for the caller's pre-call reserve, not money spent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_task_usd: Option<Number>,
}

impl Price {
    pub fn metered(rates: RatesPerMtok) -> Self {
        Self {
            basis: PriceBasis::Metered,
            rates_per_mtok: rates,
            multiplier: None,
            estimated_task_usd: None,
        }
    }
    /// `rates` must already be divided by `multiplier` by the router.
    pub fn subscription_equivalent(rates: RatesPerMtok, multiplier: Number) -> Self {
        Self {
            basis: PriceBasis::SubscriptionEquivalent,
            rates_per_mtok: rates,
            multiplier: Some(multiplier),
            estimated_task_usd: None,
        }
    }
    pub fn with_estimated_task_usd(mut self, estimate: Number) -> Self {
        self.estimated_task_usd = Some(estimate);
        self
    }

    /// Producer validation. Decoding remains tolerant of future record values.
    pub fn check(&self) -> Result<(), &'static str> {
        match (&self.basis, &self.multiplier) {
            (PriceBasis::Metered, None) => {}
            (PriceBasis::SubscriptionEquivalent, Some(n))
                if n.as_f64().is_some_and(|n| n > 0.0 && n.is_finite()) => {}
            _ => return Err("price.multiplier"),
        }
        let nonnegative = |n: &Number| n.as_f64().is_some_and(|n| n >= 0.0 && n.is_finite());
        let rates = &self.rates_per_mtok;
        if ![
            Some(&rates.input),
            Some(&rates.output),
            rates.cache_read.as_ref(),
            rates.cache_write.as_ref(),
        ]
        .into_iter()
        .flatten()
        .all(nonnegative)
            || rates.cache_write_ttl.as_ref().is_some_and(|map| {
                map.iter()
                    .any(|(ttl, rate)| ttl.is_empty() || !nonnegative(rate))
            })
        {
            return Err("price.rates_per_mtok");
        }
        if self
            .estimated_task_usd
            .as_ref()
            .is_some_and(|n| !nonnegative(n))
        {
            return Err("price.estimated_task_usd");
        }
        Ok(())
    }
}
