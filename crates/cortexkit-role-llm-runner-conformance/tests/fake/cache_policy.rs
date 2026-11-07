//! The fake route has only implicit caching and cannot warm its scripted provider.

use super::*;
use cortexkit_role_llm_runner::cache::{self, Cache};

pub(super) fn admit(module: &Module, session: &str, params: &Value) -> Option<Reply> {
    if !module.world.serves(groups::CACHE_POLICY) {
        return None;
    }
    let defect = module.world.defects.cache_policy;
    let plan = params.get("plan");
    let top = params.get("cache");
    if top.is_some() {
        if plan.is_some() && defect != Some("accept_both") {
            return Some(invalid("cache"));
        }
        if module.sessions.lock().unwrap().contains_key(session)
            && defect != Some("accept_continuation")
        {
            return Some(invalid("cache"));
        }
    }
    let (value, prefix) = match plan.and_then(|plan| plan.get("cache")) {
        Some(value) => (value, "plan.cache"),
        None => (top?, "cache"),
    };
    let cache: Cache = match serde_json::from_value(value.clone()) {
        Ok(cache) => cache,
        Err(_) => return Some(invalid(prefix)),
    };
    if ((prefix == "plan.cache" && defect == Some("reject_plan"))
        || (prefix == "cache" && defect == Some("reject_send")))
        && cache::check(&cache).is_ok()
        && cache.keep_warm == Some(cache::KeepWarm::Always)
    {
        return Some(invalid(prefix));
    }
    if (prefix == "plan.cache" && defect == Some("accept_plan_invalid"))
        || (prefix == "cache" && defect == Some("accept_send_invalid"))
    {
        return None;
    }
    let result = if prefix == "plan.cache" {
        cache::check(&cache)
    } else {
        cache::check_send(&cache)
    };
    if let Err(error) = result {
        return Some(invalid(error.detail["field"].as_str().unwrap()));
    }
    if cache
        .hours
        .as_ref()
        .is_some_and(|hours| hours.tz != module.world.cache_policy_zone)
    {
        return Some(invalid(&format!("{prefix}.hours.tz")));
    }
    None
}
