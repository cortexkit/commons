//! Owner relay for person-only commands. Replies and progress stay out of model history.
//!
//! Runner-owned command and reply types avoid a production dependency on the
//! tool-provider crate and its release cycle. Test-only checks compare both
//! crates' encodings with the same provider vectors to detect wire-shape drift.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// A command declaration, kept separate from the model's tool definitions.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct UserCommand {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

impl UserCommand {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            argument_hint: None,
        }
    }

    pub fn with_argument_hint(mut self, hint: impl Into<String>) -> Self {
        self.argument_hint = Some(hint.into());
        self
    }

    /// Validate a declaration, returning the first invalid field's path,
    /// for example `user_commands.name`.
    pub fn check(&self) -> Result<(), &'static str> {
        if !(1..=32).contains(&self.name.len())
            || !self.name.as_bytes()[0].is_ascii_lowercase()
            || !self.name.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            })
        {
            return Err("user_commands.name");
        }
        if !(1..=120).contains(&self.description.chars().count())
            || self.description.contains([
                '\r', '\n', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
            ])
        {
            return Err("user_commands.description");
        }
        if self
            .argument_hint
            .as_ref()
            .is_some_and(|hint| !(1..=60).contains(&hint.chars().count()))
        {
            return Err("user_commands.argument_hint");
        }
        Ok(())
    }
}

/// A reply for the person. Unrecognized state strings retain their text.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct UserCommandReply {
    pub state: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
}

impl UserCommandReply {
    pub fn new(state: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            text: text.into(),
            job: None,
        }
    }

    pub fn with_job(mut self, job: impl Into<String>) -> Self {
        self.job = Some(job.into());
        self
    }

    pub fn check(&self) -> Result<(), &'static str> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err("text");
        }
        Ok(())
    }
}

/// A frozen command with its declaring module, independent of host menu aliases.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FrozenUserCommand {
    pub provider: String,
    #[serde(flatten)]
    pub command: UserCommand,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserCommandRequest {
    pub session_ref: String,
    pub command: String,
    pub input: String,
    pub invocation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserCommandAnswer {
    pub call_key: String,
    pub reply: UserCommandReply,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserCommandStatusRequest {
    pub session_ref: String,
    pub call_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserCommandStatus {
    /// State values remain open-ended, as for the provider's reply state.
    pub state: String,
    pub text: String,
    pub progress: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
}

impl UserCommandStatus {
    pub fn check(&self) -> Result<(), &'static str> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err("text");
        }
        if self.progress.len() > 20
            || self.progress.iter().map(String::len).sum::<usize>() > 64 * 1024
        {
            return Err("progress");
        }
        Ok(())
    }
}

/// A missing `interactive` defaults to true; a present value must be a boolean.
pub fn check_interactive(plan: &Map<String, Value>) -> Result<bool, &'static str> {
    match plan.get("interactive") {
        None => Ok(true),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err("plan.interactive"),
    }
}

/// Host menus cannot be used when their replies would enter model history.
pub fn host_command_name(
    module: &str,
    name: &str,
    replies_outside_history: bool,
    already_registered: bool,
) -> Option<String> {
    replies_outside_history.then(|| {
        if already_registered {
            format!("{module}-{name}")
        } else {
            name.to_owned()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        baseline::Baseline,
        capabilities,
        describe::{check_describe, Major, RoleDescribe},
        errors::UserCommandCollisionDetail,
        ops, vectors,
    };
    use serde_json::json;

    fn data() -> Value {
        serde_json::from_str(vectors::file("user-commands.json").unwrap()).unwrap()
    }

    #[test]
    fn user_command_baselines_preserve_absent_bytes() {
        let data = data();
        let old: Value = serde_json::from_str(vectors::file("baseline.json").unwrap()).unwrap();
        assert_eq!(
            data["baselines"][0]["baseline"],
            old["answers"][1]["answer"]["baseline"]
        );
        for row in data["baselines"].as_array().unwrap() {
            let baseline: Baseline = serde_json::from_value(row["baseline"].clone()).unwrap();
            assert_eq!(serde_json::to_value(&baseline).unwrap(), row["baseline"]);
            assert_eq!(
                String::from_utf8(serde_jcs::to_vec(&baseline).unwrap()).unwrap(),
                row["encoded_jcs"]
            );
        }
    }

    #[test]
    fn user_command_relay_shapes_round_trip_and_bound_progress() {
        let data = data();
        macro_rules! round_trip {
            ($ty:ty, $value:expr) => {{
                let value = $value;
                let decoded: $ty = serde_json::from_value(value.clone()).unwrap();
                assert_eq!(serde_json::to_value(decoded).unwrap(), *value);
            }};
        }
        round_trip!(UserCommandRequest, &data["request"]);
        round_trip!(UserCommandAnswer, &data["answer"]);
        round_trip!(UserCommandStatusRequest, &data["status_request"]);
        round_trip!(UserCommandCollisionDetail, &data["collision"]);
        round_trip!(
            crate::errors::UnknownUserCommandDetail,
            &data["unknown_user_command"]["detail"]
        );
        assert_eq!(
            data["unknown_user_command"]["code"],
            crate::errors::UNKNOWN_USER_COMMAND
        );
        for row in data["statuses"].as_array().unwrap() {
            round_trip!(UserCommandStatus, row);
            serde_json::from_value::<UserCommandStatus>(row.clone())
                .unwrap()
                .check()
                .unwrap();
        }
        let mut status: UserCommandStatus =
            serde_json::from_value(data["statuses"][0].clone()).unwrap();
        status.progress = vec![String::new(); 21];
        assert_eq!(status.check(), Err("progress"));
        status.progress = vec!["é".repeat(32768)];
        assert_eq!(status.check(), Ok(()));
        status.progress[0].push('x');
        assert_eq!(status.check(), Err("progress"));
    }

    #[test]
    fn user_command_interactive_and_host_menu_vectors() {
        let data = data();
        for row in data["plans"].as_array().unwrap() {
            assert_eq!(
                check_interactive(row["plan"].as_object().unwrap()),
                Ok(row["interactive"].as_bool().unwrap())
            );
        }
        for row in data["bad_plans"].as_array().unwrap() {
            assert_eq!(
                check_interactive(row.as_object().unwrap()),
                Err("plan.interactive")
            );
        }
        for row in data["host_names"].as_array().unwrap() {
            assert_eq!(
                host_command_name(
                    "magic-context",
                    "ctx-wrapup",
                    row["safe"].as_bool().unwrap(),
                    row["registered"].as_bool().unwrap()
                ),
                row["result"].as_str().map(str::to_owned)
            );
        }
    }

    #[test]
    fn user_commands_group_requires_both_owner_ops() {
        for missing in [ops::SESSION_USER_COMMAND, ops::SESSION_USER_COMMAND_STATUS] {
            let mut ops: Vec<_> = crate::REQUIRED_OPS
                .iter()
                .map(|op| (*op).to_owned())
                .collect();
            ops.extend(
                [
                    crate::ops::SESSION_USER_COMMAND,
                    crate::ops::SESSION_USER_COMMAND_STATUS,
                ]
                .into_iter()
                .filter(|op| *op != missing)
                .map(str::to_owned),
            );
            let describe = RoleDescribe::new(vec![Major::new("v1", ops, "alpha")], "fixture")
                .with_capabilities(vec![capabilities::USER_COMMANDS.into()]);
            assert!(check_describe(&serde_json::to_value(describe).unwrap()).is_err());
        }
        assert_eq!(
            serde_json::to_value(UserCommandReply::new("future", "readable")).unwrap(),
            json!({"state":"future","text":"readable"})
        );
    }
}

#[cfg(test)]
mod provider_parity_tests {
    use super::*;
    use cortexkit_role_tool_provider::{user_command as provider, vectors};
    use serde_json::json;

    fn command_case(wire: &Value, valid: bool) {
        let own = serde_json::from_value::<UserCommand>(wire.clone());
        let other = serde_json::from_value::<provider::UserCommand>(wire.clone());
        assert_eq!(own.is_ok(), other.is_ok(), "declaration decode: {wire}");
        assert_eq!(
            own.as_ref().is_ok_and(|command| command.check().is_ok()),
            valid,
            "runner declaration: {wire}"
        );
        assert_eq!(
            other.as_ref().is_ok_and(|command| command.check().is_ok()),
            valid,
            "provider declaration: {wire}"
        );
        let (Ok(own), Ok(other)) = (own, other) else {
            return;
        };
        let bytes = serde_jcs::to_vec(wire).unwrap();
        assert_eq!(serde_jcs::to_vec(&own).unwrap(), bytes);
        assert_eq!(serde_jcs::to_vec(&other).unwrap(), bytes);
        let mut frozen = wire.clone();
        frozen["provider"] = json!("declaring-provider");
        let entry: FrozenUserCommand = serde_json::from_value(frozen.clone()).unwrap();
        assert_eq!(
            serde_jcs::to_vec(&entry).unwrap(),
            serde_jcs::to_vec(&frozen).unwrap()
        );
    }

    fn reply_case(wire: &Value, valid: bool) {
        let own = serde_json::from_value::<UserCommandReply>(wire.clone());
        let other = serde_json::from_value::<provider::UserCommandReply>(wire.clone());
        assert_eq!(own.is_ok(), other.is_ok(), "reply decode: {wire}");
        assert_eq!(
            own.as_ref().is_ok_and(|reply| reply.check().is_ok()),
            valid,
            "runner reply: {wire}"
        );
        assert_eq!(
            other.as_ref().is_ok_and(|reply| reply.check().is_ok()),
            valid,
            "provider reply: {wire}"
        );
        if let (Ok(own), Ok(other)) = (own, other) {
            let bytes = serde_jcs::to_vec(wire).unwrap();
            assert_eq!(serde_jcs::to_vec(&own).unwrap(), bytes);
            assert_eq!(serde_jcs::to_vec(&other).unwrap(), bytes);
        }
    }

    /// Canonical bytes and validity are compared with the shared input cases,
    /// not only with each other, so two decoders cannot agree on a lost field.
    #[test]
    fn user_command_types_match_provider_vectors() {
        let data: Value = serde_json::from_str(vectors::USER_COMMANDS_JSON).unwrap();
        for (bucket, valid) in [("commands", true), ("bad_commands", false)] {
            for wire in data[bucket].as_array().unwrap() {
                command_case(wire, valid);
            }
        }
        for (bucket, valid) in [("replies", true), ("bad_replies", false)] {
            for wire in data[bucket].as_array().unwrap() {
                reply_case(wire, valid);
            }
        }
        for catalog in data["catalogs"].as_array().unwrap() {
            if let Some(commands) = catalog["catalog"]["user_commands"].as_array() {
                for wire in commands {
                    command_case(wire, true);
                }
            }
        }
        for entry in data["late"].as_array().unwrap() {
            if let Some(reply) = entry.get("result") {
                reply_case(reply, true);
            }
        }
    }

    #[test]
    fn user_command_types_validate_limits() {
        for (name, description, hint, valid) in [
            ("a".repeat(32), "é".repeat(120), "é".repeat(60), true),
            ("a".repeat(33), "ok".into(), "hint".into(), false),
            ("a".into(), "é".repeat(121), "hint".into(), false),
            ("a".into(), "ok".into(), "é".repeat(61), false),
            ("a".into(), "line\u{2028}break".into(), "hint".into(), false),
        ] {
            let wire = json!({"name": name, "description": description, "argument_hint": hint});
            assert_eq!(
                serde_json::from_value::<UserCommand>(wire.clone())
                    .unwrap()
                    .check()
                    .is_ok(),
                valid
            );
            assert_eq!(
                serde_json::from_value::<provider::UserCommand>(wire)
                    .unwrap()
                    .check()
                    .is_ok(),
                valid
            );
        }
        for (length, valid) in [(32768, true), (32769, false)] {
            let wire = json!({"state": "future", "text": "é".repeat(length)});
            assert_eq!(
                serde_json::from_value::<UserCommandReply>(wire.clone())
                    .unwrap()
                    .check()
                    .is_ok(),
                valid
            );
            assert_eq!(
                serde_json::from_value::<provider::UserCommandReply>(wire)
                    .unwrap()
                    .check()
                    .is_ok(),
                valid
            );
        }
    }

    #[test]
    fn unknown_user_command_refusal_detail_matches_provider() {
        let data: Value =
            serde_json::from_str(crate::vectors::file("user-commands.json").unwrap()).unwrap();
        let refusal = &data["unknown_user_command"];
        let own: crate::errors::UnknownUserCommandDetail =
            serde_json::from_value(refusal["detail"].clone()).unwrap();
        let other = cortexkit_role_tool_provider::errors::unknown_user_command(&own.command);
        assert_eq!(other.code, refusal["code"]);
        assert_eq!(
            serde_jcs::to_vec(&own).unwrap(),
            serde_jcs::to_vec(&other.detail.unwrap()).unwrap()
        );
        let errors: Value =
            serde_json::from_str(crate::vectors::file("errors.json").unwrap()).unwrap();
        let code = errors["codes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["code"] == crate::errors::UNKNOWN_USER_COMMAND)
            .unwrap();
        assert_eq!(code["detail"], refusal["detail"]);
    }
}
