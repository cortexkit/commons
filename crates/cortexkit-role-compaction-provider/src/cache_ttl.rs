//! The prompt cache's lifetime (CONTRACT.md §8.1): `cache_ttl` entries a
//! provider sends, and the order in which a runner decides the lifetime it
//! judges `ttl_expired` by.

use serde::{Deserialize, Serialize};

/// A provider's statement of the prompt cache's lifetime for one model:
/// `{model, ms, configured}`. It rides on Setup's `ready` answer (for
/// Setup's model), on every `compaction.ride` answer whose cause is
/// `model_change`, `nothing` included, and on every view (for the model of
/// the notice the view answers).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CacheTtl {
    /// The model the lifetime belongs to. A runner uses the entry only for a
    /// request to exactly this model.
    pub model: String,
    /// The lifetime in milliseconds. `null` with `configured: true` means
    /// the cache never expires as far as the runner's declarations go: it
    /// never declares `ttl_expired` for this model. Always encoded, `null`
    /// included.
    pub ms: Option<u64>,
    /// `true` when the session's configuration sets the lifetime, which then
    /// overrides what the runner observed; `false` when it is the provider's
    /// default for the model, which yields to an observed lifetime.
    pub configured: bool,
}

impl CacheTtl {
    pub fn new(model: impl Into<String>, ms: Option<u64>, configured: bool) -> Self {
        Self {
            model: model.into(),
            ms,
            configured,
        }
    }
}

/// The lifetime a runner judges `ttl_expired` by for a request to `model`,
/// or `None` when it must not declare `ttl_expired` at all.
///
/// `entries` are the `cache_ttl` entries the runner received for the
/// session, oldest first; only those naming `model` count, and the newest of
/// them wins, so a previous model's value is never kept. `observed_ms` is
/// the lifetime the runner saw in the final options of its own request, when
/// it saw one. In order:
///
/// 1. a `configured: true` entry: its `ms`, where `null` means never;
/// 2. the observed lifetime;
/// 3. a `configured: false` entry, the provider's default for the model;
/// 4. otherwise nothing, so no `ttl_expired`.
pub fn lifetime(model: &str, entries: &[CacheTtl], observed_ms: Option<u64>) -> Option<u64> {
    let newest = entries.iter().rev().find(|entry| entry.model == model);
    match newest {
        Some(entry) if entry.configured => entry.ms,
        _ => observed_ms.or_else(|| newest.and_then(|entry| entry.ms)),
    }
}

/// `true` when a request to `model`, `idle_ms` after the previous request
/// reached the cache, finds the prompt cache expired by [`lifetime`]'s
/// rule: the lifetime is known and the idle time is at least that long.
pub fn expired(model: &str, entries: &[CacheTtl], observed_ms: Option<u64>, idle_ms: u64) -> bool {
    lifetime(model, entries, observed_ms).is_some_and(|ms| idle_ms >= ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_is_encoded_and_absent_ms_decodes_as_null() {
        let never = CacheTtl::new("acme/model-1", None, true);
        assert_eq!(
            serde_json::to_value(&never).unwrap(),
            serde_json::json!({"model": "acme/model-1", "ms": null, "configured": true})
        );
        let decoded: CacheTtl =
            serde_json::from_value(serde_json::json!({"model": "m", "configured": false})).unwrap();
        assert_eq!(decoded, CacheTtl::new("m", None, false));
        assert!(!expired(
            "m",
            &[CacheTtl::new("m", None, true)],
            Some(1),
            u64::MAX
        ));
        assert!(expired("m", &[], Some(300_000), 300_000));
    }
}
