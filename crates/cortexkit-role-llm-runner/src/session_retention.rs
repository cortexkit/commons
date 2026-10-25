//! Per-session storage policy, independent of the legacy retention-in-seconds group.

use serde::{de::Error, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

/// How long the runner keeps a session, measured from durable activity.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Keep {
    None,
    Days { days: u32 },
    Forever,
}

impl Keep {
    pub fn none() -> Self {
        Self::None
    }
    pub fn days(days: u32) -> Self {
        Self::Days { days }
    }
    pub fn forever() -> Self {
        Self::Forever
    }
}

impl Serialize for Keep {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::None => json!("none"),
            Self::Forever => json!("forever"),
            Self::Days { days } => json!({"days":days}),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Keep {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let keep = Value::deserialize(d)?;
        RetentionPolicy::from_value(
            &json!({"keep":keep,"archive_after_days":0,"backup":false,"request_dumps":false}),
        )
        .map(|p| p.keep)
        .map_err(D::Error::custom)
    }
}

/// The complete policy. All four wire members are required; unknown members are refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetentionPolicy {
    pub keep: Keep,
    pub archive_after_days: u32,
    pub backup: bool,
    pub request_dumps: bool,
}

impl RetentionPolicy {
    pub fn new(keep: Keep, archive_after_days: u32, backup: bool, request_dumps: bool) -> Self {
        Self {
            keep,
            archive_after_days,
            backup,
            request_dumps,
        }
    }

    /// Decode strictly, returning the member path for an `invalid_params` detail.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let path = |member: &str| format!("retention_policy.{member}");
        let object = value.as_object().ok_or("retention_policy")?;
        for name in object.keys() {
            if !["keep", "archive_after_days", "backup", "request_dumps"].contains(&name.as_str()) {
                return Err(path(name));
            }
        }
        let keep = match object.get("keep") {
            Some(Value::String(s)) if s == "none" => Keep::None,
            Some(Value::String(s)) if s == "forever" => Keep::Forever,
            Some(Value::Object(days)) => {
                for name in days.keys() {
                    if name != "days" {
                        return Err(path(&format!("keep.{name}")));
                    }
                }
                let n = days
                    .get("days")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| path("keep.days"))?;
                Keep::Days { days: n }
            }
            _ => return Err(path("keep")),
        };
        let archive_after_days = object
            .get("archive_after_days")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| path("archive_after_days"))?;
        let backup = object
            .get("backup")
            .and_then(Value::as_bool)
            .ok_or_else(|| path("backup"))?;
        let request_dumps = object
            .get("request_dumps")
            .and_then(Value::as_bool)
            .ok_or_else(|| path("request_dumps"))?;
        let policy = Self::new(keep, archive_after_days, backup, request_dumps);
        policy.check(3650)?;
        Ok(policy)
    }

    pub fn check(&self, max_days: u32) -> Result<(), String> {
        if let Keep::Days { days } = self.keep {
            if days == 0 || days > max_days {
                return Err("retention_policy.keep.days".into());
            }
        }
        if self.archive_after_days > 3650
            || matches!(self.keep, Keep::None) && self.archive_after_days != 0
            || matches!(self.keep, Keep::Days { days } if self.archive_after_days != 0 && self.archive_after_days >= days)
        {
            return Err("retention_policy.archive_after_days".into());
        }
        if matches!(self.keep, Keep::None) && self.backup {
            return Err("retention_policy.backup".into());
        }
        Ok(())
    }
}

impl Serialize for RetentionPolicy {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let keep = match self.keep {
            Keep::None => json!("none"),
            Keep::Forever => json!("forever"),
            Keep::Days { days } => json!({"days":days}),
        };
        let mut state = serializer.serialize_struct("RetentionPolicy", 4)?;
        state.serialize_field("keep", &keep)?;
        state.serialize_field("archive_after_days", &self.archive_after_days)?;
        state.serialize_field("backup", &self.backup)?;
        state.serialize_field("request_dumps", &self.request_dumps)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for RetentionPolicy {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::from_value(&Value::deserialize(d)?).map_err(D::Error::custom)
    }
}

/// Reject explicit null rather than treating it as omission on a request.
pub(crate) fn deserialize_optional<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<RetentionPolicy>, D::Error> {
    RetentionPolicy::deserialize(d).map(Some)
}

/// Who supplied the current policy; a runner policy can be replaced once by a starter.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum RetentionSource {
    Starter,
    Runner,
    Unknown(String),
}

impl RetentionSource {
    pub fn parse(value: &str) -> Self {
        value.to_owned().into()
    }
    pub fn starter() -> Self {
        Self::Starter
    }
    pub fn runner() -> Self {
        Self::Runner
    }
}

impl From<String> for RetentionSource {
    fn from(value: String) -> Self {
        match value.as_str() {
            "starter" => Self::Starter,
            "runner" => Self::Runner,
            _ => Self::Unknown(value),
        }
    }
}
impl From<RetentionSource> for String {
    fn from(value: RetentionSource) -> Self {
        match value {
            RetentionSource::Starter => "starter".into(),
            RetentionSource::Runner => "runner".into(),
            RetentionSource::Unknown(value) => value,
        }
    }
}

/// Storage limitations do not rewrite the accepted policy.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RetentionState {
    pub source: RetentionSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_dumps_unsupported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_limited: Option<bool>,
}

impl RetentionState {
    pub fn new(source: RetentionSource) -> Self {
        Self {
            source,
            request_dumps_unsupported: None,
            host_limited: None,
        }
    }
    pub fn with_request_dumps_unsupported(mut self) -> Self {
        self.request_dumps_unsupported = Some(true);
        self
    }
    pub fn with_host_limited(mut self) -> Self {
        self.host_limited = Some(true);
        self
    }
}

/// Release always targets the session bound to the management route.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReleaseRequest {}

impl ReleaseRequest {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum ReleaseState {
    Pending,
    Deleted,
    Absent,
    Unknown(String),
}

impl ReleaseState {
    pub fn parse(value: &str) -> Self {
        value.to_owned().into()
    }
    pub fn pending() -> Self {
        Self::Pending
    }
    pub fn deleted() -> Self {
        Self::Deleted
    }
    pub fn absent() -> Self {
        Self::Absent
    }
}

impl From<String> for ReleaseState {
    fn from(value: String) -> Self {
        match value.as_str() {
            "pending" => Self::Pending,
            "deleted" => Self::Deleted,
            "absent" => Self::Absent,
            _ => Self::Unknown(value),
        }
    }
}
impl From<ReleaseState> for String {
    fn from(value: ReleaseState) -> Self {
        match value {
            ReleaseState::Pending => "pending".into(),
            ReleaseState::Deleted => "deleted".into(),
            ReleaseState::Absent => "absent".into(),
            ReleaseState::Unknown(value) => value,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReleaseReply {
    pub state: ReleaseState,
}

impl ReleaseReply {
    pub fn new(state: ReleaseState) -> Self {
        Self { state }
    }
}

/// A policy refusal's code and wire detail, for the runner's error envelope.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct PolicyRefusal {
    pub code: String,
    pub detail: Value,
}

impl PolicyRefusal {
    pub fn new(code: impl Into<String>, detail: Value) -> Self {
        Self {
            code: code.into(),
            detail,
        }
    }
}

/// Validate before writing; absence on an existing session preserves its policy.
/// `first` means the first starter send or adopt, including adoption of a hosted
/// session that already has a runner-assigned policy.
/// Pass no limits when the group is undeclared. A backup reason means that
/// single-session backup is unavailable; dumps are deliberately not a refusal.
pub fn check_policy(
    policy: Option<&RetentionPolicy>,
    legacy_retention: bool,
    limits: Option<&crate::describe::SessionRetention>,
    first: bool,
    frozen: Option<(&RetentionPolicy, RetentionSource)>,
    backup_unsupported_reason: Option<&str>,
) -> Result<(), PolicyRefusal> {
    use crate::errors;
    let invalid = |field: &str| PolicyRefusal::new(errors::INVALID_PARAMS, json!({"field":field}));
    let Some(policy) = policy else {
        return if first && limits.is_some_and(|l| l.required) {
            Err(PolicyRefusal::new(errors::RETENTION_REQUIRED, json!({})))
        } else {
            Ok(())
        };
    };
    let Some(limits) = limits else {
        return Err(invalid("retention_policy"));
    };
    if legacy_retention {
        return Err(invalid("retention_policy"));
    }
    policy
        .check(limits.max_days)
        .map_err(|field| invalid(&field))?;
    if let Some((previous, source)) = frozen {
        if source != RetentionSource::Runner && previous != policy {
            return Err(PolicyRefusal::new(
                errors::RETENTION_CONFLICT,
                json!({"frozen":previous}),
            ));
        }
    }
    if policy.backup {
        if let Some(reason) = backup_unsupported_reason {
            return Err(PolicyRefusal::new(
                errors::BACKUP_UNSUPPORTED,
                json!({"reason":reason}),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adopt::AdoptRequest,
        capabilities,
        describe::{check_describe, DescribeProblem, Major, RoleDescribe, SessionRetention},
        errors, ops,
        read::HeadMeta,
        send::SendRequest,
        PROVIDES,
    };

    fn vectors() -> Value {
        serde_json::from_str(crate::vectors::file("session-retention.json").unwrap()).unwrap()
    }

    #[test]
    fn policy_shapes_and_refusals() {
        let v = vectors();
        for policy in v["policies"].as_array().unwrap() {
            let decoded: RetentionPolicy = serde_json::from_value(policy.clone()).unwrap();
            assert_eq!(serde_json::to_value(&decoded).unwrap(), *policy);
            assert_eq!(serde_json::to_value(&decoded.keep).unwrap(), policy["keep"]);
            assert_eq!(
                serde_json::from_value::<Keep>(policy["keep"].clone()).unwrap(),
                decoded.keep
            );
        }
        for invalid in v["invalid"].as_array().unwrap() {
            assert_eq!(
                RetentionPolicy::from_value(&invalid["policy"]).unwrap_err(),
                invalid["field"].as_str().unwrap()
            );
            for request in [
                json!({"prompt":"p","send_id":"s","retention_policy":invalid["policy"]}),
                json!({"send_id":"a","plan":{},"retention_policy":invalid["policy"]}),
            ] {
                let error = if request.get("prompt").is_some() {
                    serde_json::from_value::<SendRequest>(request)
                        .unwrap_err()
                        .to_string()
                } else {
                    serde_json::from_value::<AdoptRequest>(request)
                        .unwrap_err()
                        .to_string()
                };
                assert!(
                    error.contains(invalid["field"].as_str().unwrap()),
                    "{error}"
                );
            }
        }
        for p in [
            json!({"keep":{"days":-1},"archive_after_days":0,"backup":false,"request_dumps":false}),
            json!({"keep":{"days":4294967296u64},"archive_after_days":0,"backup":false,"request_dumps":false}),
        ] {
            assert_eq!(
                RetentionPolicy::from_value(&p).unwrap_err(),
                "retention_policy.keep.days"
            );
        }
        RetentionPolicy::new(Keep::days(3650), 3649, false, false)
            .check(3650)
            .unwrap();
        assert_eq!(
            RetentionPolicy::new(Keep::days(31), 0, false, false)
                .check(30)
                .unwrap_err(),
            "retention_policy.keep.days"
        );
    }

    #[test]
    fn describe_limits_and_named_problems() {
        let v = vectors();
        let describe = RoleDescribe::new(
            vec![Major::new(
                PROVIDES,
                vec![
                    ops::ROLE_DESCRIBE.into(),
                    ops::SESSION_BASELINE.into(),
                    ops::SESSION_SEND.into(),
                    ops::SESSION_HEAD.into(),
                    ops::SESSION_RELEASE.into(),
                ],
                "alpha",
            )],
            "fixture",
        )
        .with_session_capabilities_from("baseline")
        .with_capabilities(vec![capabilities::SESSION_RETENTION.into()])
        .with_session_retention(serde_json::from_value(v["limits"].clone()).unwrap());
        let good = serde_json::to_value(describe).unwrap();
        check_describe(&good).unwrap();
        for invalid in v["bad_limits"].as_array().unwrap() {
            let mut raw = good.clone();
            raw["session_retention"] = invalid["limits"].clone();
            assert_eq!(
                check_describe(&raw).unwrap_err(),
                vec![DescribeProblem::Undecodable(
                    invalid["problem"].as_str().unwrap().into()
                )]
            );
        }
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("session_retention");
        assert_eq!(
            check_describe(&missing).unwrap_err(),
            vec![DescribeProblem::Undecodable(
                "missing_session_retention".into()
            )]
        );
    }

    #[test]
    fn requests_heads_release_and_legacy_bytes() {
        let v = vectors();
        let send: SendRequest = serde_json::from_value(v["send"].clone()).unwrap();
        assert_eq!(serde_json::to_value(send).unwrap(), v["send"]);
        let adopt: AdoptRequest = serde_json::from_value(v["adopt"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&adopt).unwrap(), v["adopt"]);
        let mut changed = adopt.clone();
        changed.retention_policy.as_mut().unwrap().request_dumps = false;
        assert_eq!(changed.reused_field(&adopt), Some("retention_policy"));
        assert_eq!(adopt.reused_field(&adopt), None);
        for head in v["head"].as_array().unwrap() {
            let decoded: HeadMeta = serde_json::from_value(head.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), *head);
        }
        assert_eq!(
            serde_json::to_value(ReleaseRequest::new()).unwrap(),
            v["release"]["request"]
        );
        assert!(
            serde_json::from_value::<ReleaseRequest>(json!({"session":"not-a-selector"})).is_err()
        );
        for answer in v["release"]["answers"].as_array().unwrap() {
            assert_eq!(
                serde_json::to_value(
                    serde_json::from_value::<ReleaseReply>(answer.clone()).unwrap()
                )
                .unwrap(),
                *answer
            );
        }
        assert_eq!(
            serde_json::to_string(&SendRequest::new("p", "s")).unwrap(),
            r#"{"prompt":"p","send_id":"s"}"#
        );
        assert_eq!(
            serde_json::to_string(&AdoptRequest::new("a", Default::default())).unwrap(),
            r#"{"send_id":"a","plan":{}}"#
        );
        assert_eq!(
            serde_json::to_string(&HeadMeta::no_lineage()).unwrap(),
            "{}"
        );
        assert!(errors::is_retryable(errors::SESSION_RELEASED));
        assert_eq!(
            serde_json::to_value(ReleaseState::parse("future")).unwrap(),
            json!("future")
        );
        assert_eq!(
            serde_json::to_value(RetentionSource::parse("future")).unwrap(),
            json!("future")
        );
        for refusal in v["refusals"].as_array().unwrap() {
            assert!(errors::CODES.contains(&refusal["code"].as_str().unwrap()));
            assert!(refusal.get("field").is_none());
        }
    }

    #[test]
    fn admission_required_conflict_backup_and_runner_replacement() {
        let limits = SessionRetention::new(30, 1, true);
        let p = RetentionPolicy::new(Keep::forever(), 0, false, true);
        let other = RetentionPolicy::new(Keep::days(1), 0, false, false);
        use super::check_policy as check;
        assert_eq!(
            check(None, false, Some(&limits), true, None, None)
                .unwrap_err()
                .code,
            errors::RETENTION_REQUIRED
        );
        check(
            None,
            false,
            Some(&SessionRetention::new(30, 1, false)),
            true,
            None,
            None,
        )
        .unwrap();
        check(
            None,
            false,
            Some(&limits),
            false,
            Some((&p, RetentionSource::Starter)),
            None,
        )
        .unwrap();
        for (limits, legacy) in [(None, false), (Some(&limits), true)] {
            assert_eq!(
                check(Some(&p), legacy, limits, true, None, None).unwrap_err(),
                PolicyRefusal::new(errors::INVALID_PARAMS, json!({"field":"retention_policy"}))
            );
        }
        let same: RetentionPolicy = serde_json::from_str(
            r#"{"request_dumps":true,"backup":false,"archive_after_days":0,"keep":"forever"}"#,
        )
        .unwrap();
        check(
            Some(&same),
            false,
            Some(&limits),
            false,
            Some((&p, RetentionSource::Starter)),
            None,
        )
        .unwrap();
        assert_eq!(
            check(
                Some(&other),
                false,
                Some(&limits),
                false,
                Some((&p, RetentionSource::Starter)),
                None
            )
            .unwrap_err(),
            PolicyRefusal::new(errors::RETENTION_CONFLICT, json!({"frozen":p}))
        );
        check(
            Some(&other),
            false,
            Some(&limits),
            false,
            Some((&p, RetentionSource::Runner)),
            None,
        )
        .unwrap();
        assert_eq!(
            check(
                Some(&p),
                false,
                Some(&limits),
                false,
                Some((&other, RetentionSource::Starter)),
                None
            )
            .unwrap_err()
            .code,
            errors::RETENTION_CONFLICT
        );
        let backup = RetentionPolicy::new(Keep::days(1), 0, true, false);
        assert_eq!(
            check(
                Some(&backup),
                false,
                Some(&limits),
                true,
                None,
                Some("single-session backup unavailable")
            )
            .unwrap_err(),
            PolicyRefusal::new(
                errors::BACKUP_UNSUPPORTED,
                json!({"reason":"single-session backup unavailable"})
            )
        );
        let bad = RetentionPolicy::new(Keep::none(), 0, true, false);
        assert_eq!(
            check(Some(&bad), false, Some(&limits), true, None, None)
                .unwrap_err()
                .detail,
            json!({"field":"retention_policy.backup"})
        );
    }
}
