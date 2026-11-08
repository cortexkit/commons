//! Optional head creation without a message or run (CONTRACT §9.5.1).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::describe::RoleDescribe;

/// True exactly when the describe lists `harness.host_start` as an extension.
pub fn advertises_host_start(describe: &RoleDescribe) -> bool {
    describe
        .extensions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|name| name == crate::ops::HARNESS_HOST_START)
}

/// The model selected before the first send. No other model fields are accepted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HostStartModel {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl HostStartModel {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            variant: None,
        }
    }

    pub fn with_variant(mut self, variant: impl Into<String>) -> Self {
        self.variant = Some(variant.into());
        self
    }
}

/// Strict `harness.host_start` parameters. The route supplies the session and
/// attested owner, never the request. `plan` is the same object as on owner sends.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HostStartRequest {
    #[serde(deserialize_with = "nonempty_send_id")]
    pub send_id: String,
    pub plan: Map<String, Value>,
    pub model: HostStartModel,
}

fn nonempty_send_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if value.is_empty() {
        Err(serde::de::Error::custom("send_id must be non-empty"))
    } else {
        Ok(value)
    }
}

impl HostStartRequest {
    pub fn new(
        send_id: impl Into<String>,
        plan: Map<String, Value>,
        model: HostStartModel,
    ) -> Self {
        Self {
            send_id: send_id.into(),
            plan,
            model,
        }
    }

    /// Name the field to report in `send_id_reuse`, or replay on an exact repeat.
    /// The caller has already looked up the earlier request by `send_id`.
    pub fn reused_field(&self, earlier: &Self) -> Option<&'static str> {
        if self.plan != earlier.plan {
            Some("plan")
        } else if self.model != earlier.model {
            Some("model")
        } else {
            None
        }
    }
}

/// A recorded head with its plan frozen and caller bound as owner. Success
/// writes no message and starts no run; the native host may start at first send.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HostStartReply {
    pub started: bool,
    pub applied_plan_digest: String,
}

impl HostStartReply {
    pub fn new(applied_plan_digest: impl Into<String>) -> Self {
        Self {
            started: true,
            applied_plan_digest: applied_plan_digest.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{describe::check_describe, ops, PROVIDES, REQUIRED_OPS};
    use serde_json::json;

    fn request() -> Value {
        json!({"send_id":"host-start:plan-a", "plan":{},
            "model":{"provider":"example", "model":"scripted"}})
    }

    #[test]
    fn host_start_strict_wire_and_reuse() {
        let raw = request();
        let first: HostStartRequest = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(&first).unwrap(), raw);
        assert_eq!(first.reused_field(&first), None);
        let mut changed = first.clone();
        changed.plan.insert("different".into(), json!(true));
        assert_eq!(changed.reused_field(&first), Some("plan"));
        changed = first.clone();
        changed.model = changed.model.with_variant("other");
        assert_eq!(changed.reused_field(&first), Some("model"));
        for (field, value) in [
            ("extra", json!(true)),
            ("send_id", json!("")),
            ("plan", json!("digest")),
            ("model", json!({"provider":"p", "model":"m", "extra":true})),
        ] {
            let mut bad = raw.clone();
            bad[field] = value;
            assert!(
                serde_json::from_value::<HostStartRequest>(bad).is_err(),
                "{field}"
            );
        }
        let reply = HostStartReply::new("plan-a");
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            json!({"started":true,"applied_plan_digest":"plan-a"})
        );
        let mut bad = serde_json::to_value(reply).unwrap();
        bad["extra"] = json!(true);
        assert!(serde_json::from_value::<HostStartReply>(bad).is_err());
    }

    fn describe() -> Value {
        json!({"majors":[{"version":PROVIDES,"ops":REQUIRED_OPS,"stability":"stable"}],
            "implementation_version":"test", "capabilities":[],
            "session_capabilities_from":"baseline"})
    }

    #[test]
    fn host_start_advertisement_is_only_the_extension() {
        let raw = describe();
        let absent = check_describe(&raw).unwrap();
        assert!(!advertises_host_start(&absent));
        assert!(serde_json::to_value(&absent)
            .unwrap()
            .get("extensions")
            .is_none());
        let built =
            absent.with_extensions(vec![ops::HARNESS_HOST_START.into(), "future.op".into()]);
        assert!(advertises_host_start(
            &check_describe(&serde_json::to_value(built).unwrap()).unwrap()
        ));
        let mut raw = raw;
        raw["capabilities"] = json!([ops::HARNESS_HOST_START]);
        raw["majors"][0]["ops"] = json!([
            ops::ROLE_DESCRIBE,
            ops::SESSION_BASELINE,
            ops::HARNESS_HOST_START
        ]);
        assert!(!advertises_host_start(&check_describe(&raw).unwrap()));
        for value in [json!([]), json!(["future.op"])] {
            raw["extensions"] = value;
            assert!(!advertises_host_start(&check_describe(&raw).unwrap()));
        }
    }

    #[test]
    fn host_start_malformed_extensions_are_refused() {
        for value in [
            json!(null),
            json!("harness.host_start"),
            json!([false]),
            json!([ops::HARNESS_HOST_START, ops::HARNESS_HOST_START]),
            json!(["future.op", "future.op"]),
        ] {
            let mut raw = describe();
            raw["extensions"] = value;
            assert!(check_describe(&raw).is_err());
        }
    }

    #[test]
    fn host_start_sample_vectors_and_host_unavailable() {
        let v: Value =
            serde_json::from_str(crate::vectors::file("host-start.json").unwrap()).unwrap();
        let describe = check_describe(&v["describe"]).unwrap();
        assert!(advertises_host_start(&describe));
        assert_eq!(
            describe.extensions.as_ref().unwrap()[1],
            "example.future_op"
        );
        let request: HostStartRequest = serde_json::from_value(v["request"].clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), v["request"]);
        let reply: HostStartReply = serde_json::from_value(v["reply"].clone()).unwrap();
        assert!(reply.started);
        assert_eq!(serde_json::to_value(reply).unwrap(), v["reply"]);
        assert_eq!(crate::errors::HOST_UNAVAILABLE, "host_unavailable");
        assert!(crate::errors::CODES.contains(&crate::errors::HOST_UNAVAILABLE));
        assert!(!crate::errors::is_retryable(
            crate::errors::HOST_UNAVAILABLE
        ));
        assert_eq!(
            v["refusal"],
            json!({"code":crate::errors::HOST_UNAVAILABLE,
            "detail":{"host_error":"unsupported_version"}})
        );
    }
}
