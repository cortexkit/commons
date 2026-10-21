//! Person-driven commands use a reserved op, not the model's tool surface.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use subc_protocol::{
    tool_call::{validate_call_key, ToolCallRequest},
    ErrorBody,
};

use crate::{catalog::CatalogAnswer, errors, ops};

pub const MAX_INPUT_BYTES: usize = 4096;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// A command frozen with the catalog for the lifetime of a session.
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

    pub fn check(&self) -> Result<(), ErrorBody> {
        if !valid_name(&self.name) {
            return Err(errors::invalid_request(
                "user_commands.name",
                "invalid command name",
            ));
        }
        let length = self.description.chars().count();
        if !(1..=120).contains(&length)
            || self.description.contains([
                '\r', '\n', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
            ])
        {
            return Err(errors::invalid_request(
                "user_commands.description",
                "expected one line of 1–120 characters",
            ));
        }
        if self
            .argument_hint
            .as_ref()
            .is_some_and(|hint| !(1..=60).contains(&hint.chars().count()))
        {
            return Err(errors::invalid_request(
                "user_commands.argument_hint",
                "expected 1–60 characters",
            ));
        }
        Ok(())
    }
}

pub fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name.as_bytes()[0].is_ascii_lowercase()
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Validate command declarations and reject `user_command.invoke` among model tools.
pub fn check_catalog(answer: &CatalogAnswer) -> Result<(), ErrorBody> {
    if answer
        .tools
        .iter()
        .any(|tool| tool.name == ops::USER_COMMAND_INVOKE)
    {
        return Err(errors::invalid_request(
            "tools.name",
            "user_command.invoke is reserved",
        ));
    }
    let mut names = BTreeSet::new();
    for command in &answer.user_commands {
        command.check()?;
        if !names.insert(&command.name) {
            return Err(errors::invalid_request(
                "user_commands.name",
                "duplicate command name",
            ));
        }
    }
    Ok(())
}

/// Arguments of `user_command.invoke`; input is forwarded verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserCommandArguments {
    pub command: String,
    pub input: String,
}

pub fn check_invoke(request: &ToolCallRequest) -> Result<UserCommandArguments, ErrorBody> {
    if request.name != ops::USER_COMMAND_INVOKE {
        return Err(errors::invalid_request(
            "name",
            "expected user_command.invoke",
        ));
    }
    let key = request
        .call_key
        .as_deref()
        .ok_or_else(|| errors::invalid_request("call_key", "required for user commands"))?;
    validate_call_key(key)
        .map_err(|error| errors::invalid_request("call_key", error.to_string()))?;
    let arguments: UserCommandArguments = serde_json::from_value(request.arguments.clone())
        .map_err(|error| errors::invalid_request("arguments", error.to_string()))?;
    if !valid_name(&arguments.command) {
        return Err(errors::invalid_request("command", "invalid command name"));
    }
    if arguments.input.len() > MAX_INPUT_BYTES {
        return Err(errors::invalid_request("input", "input exceeds 4096 bytes"));
    }
    Ok(arguments)
}

/// A person-only reply. Consumers retain unrecognized state values with their text.
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

    pub fn check(&self) -> Result<(), ErrorBody> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(errors::invalid_request("text", "text exceeds 64 KiB"));
        }
        Ok(())
    }
}

/// Admission requires a daemon-attested scope with neither a run ID nor a flow
/// ID. Request arguments and the declared call mode cannot grant admission;
/// an absent scope is not admitted.
pub fn admitted(scope: Option<&subc_protocol::scope::ScopeStamp>) -> bool {
    scope.is_some_and(|scope| {
        scope.attributes.run_id.is_none() && scope.attributes.flow_id.is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::CatalogTool,
        late_results::{EntryKind, LateEntry},
        vectors,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    fn cases() -> Value {
        serde_json::from_str(vectors::USER_COMMANDS_JSON).unwrap()
    }

    #[test]
    fn command_catalog_bytes_and_digests_preserve_absence() {
        let data = cases();
        let prior: Value = serde_json::from_str(vectors::CALL_MODES_JSON).unwrap();
        assert_eq!(
            data["catalogs"][0]["catalog"],
            prior["digests"][0]["catalog"]
        );
        assert_eq!(
            data["catalogs"][0]["content_digest"],
            prior["digests"][0]["content_digest"]
        );
        for row in data["catalogs"].as_array().unwrap() {
            let answer: CatalogAnswer = serde_json::from_value(row["catalog"].clone()).unwrap();
            check_catalog(&answer).unwrap();
            let mut encoded = serde_json::to_value(&answer).unwrap();
            assert_eq!(
                String::from_utf8(serde_jcs::to_vec(&encoded).unwrap()).unwrap(),
                row["encoded_jcs"]
            );
            encoded.as_object_mut().unwrap().remove("catalog_digest");
            assert_eq!(
                format!("{:x}", Sha256::digest(serde_jcs::to_vec(&encoded).unwrap())),
                row["content_digest"]
            );
        }
        assert_ne!(
            data["catalogs"][0]["content_digest"],
            data["catalogs"][1]["content_digest"]
        );
        let mut empty: CatalogAnswer =
            serde_json::from_value(data["catalogs"][0]["catalog"].clone()).unwrap();
        empty.user_commands.clear();
        assert!(serde_json::to_value(empty)
            .unwrap()
            .get("user_commands")
            .is_none());
    }

    #[test]
    fn command_declaration_vectors_validate_bounds_and_namespace() {
        let data = cases();
        for row in data["commands"].as_array().unwrap() {
            serde_json::from_value::<UserCommand>(row.clone())
                .unwrap()
                .check()
                .unwrap();
        }
        for row in data["bad_commands"].as_array().unwrap() {
            assert!(
                serde_json::from_value::<UserCommand>(row.clone())
                    .unwrap()
                    .check()
                    .is_err(),
                "{row}"
            );
        }
        UserCommand::new("a".repeat(32), "é".repeat(120))
            .with_argument_hint("é".repeat(60))
            .check()
            .unwrap();
        let mut answer =
            CatalogAnswer::new("g", "d")
                .with_user_commands(vec![UserCommand::new("ok", "valid"); 2]);
        assert!(check_catalog(&answer).is_err());
        answer.user_commands.clear();
        answer.tools.push(CatalogTool::new(
            ops::USER_COMMAND_INVOKE,
            "digest",
            1,
            json!({"type":"object"}),
        ));
        assert!(check_catalog(&answer).is_err());
    }

    #[test]
    fn command_invoke_and_reply_vectors() {
        let data = cases();
        for row in data["invoke"].as_array().unwrap() {
            let request: ToolCallRequest = serde_json::from_value(row.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(check_invoke(&request).unwrap()).unwrap(),
                row["arguments"]
            );
            crate::call::check_call(&request).unwrap();
        }
        for row in data["bad_invoke"].as_array().unwrap() {
            let request: ToolCallRequest = serde_json::from_value(row.clone()).unwrap();
            assert!(crate::call::check_call(&request).is_err(), "{row}");
        }
        for row in data["replies"].as_array().unwrap() {
            let reply: UserCommandReply = serde_json::from_value(row.clone()).unwrap();
            reply.check().unwrap();
            assert_eq!(serde_json::to_value(reply).unwrap(), *row);
        }
        for row in data["bad_replies"].as_array().unwrap() {
            assert!(serde_json::from_value::<UserCommandReply>(row.clone()).is_err());
        }
        UserCommandReply::new("done", "é".repeat(32768))
            .check()
            .unwrap();
        assert!(UserCommandReply::new("done", "é".repeat(32769))
            .check()
            .is_err());
    }

    #[test]
    fn command_scope_admission_table() {
        for row in cases()["admission"].as_array().unwrap() {
            let scope: Option<subc_protocol::scope::ScopeStamp> =
                serde_json::from_value(row["scope"].clone()).unwrap();
            assert_eq!(
                admitted(scope.as_ref()),
                row["admitted"].as_bool().unwrap(),
                "{row}"
            );
        }
    }

    #[test]
    fn command_late_entries_round_trip() {
        let data = cases();
        for (row, kind) in data["late"]
            .as_array()
            .unwrap()
            .iter()
            .zip([EntryKind::Progress, EntryKind::Result])
        {
            let entry: LateEntry = serde_json::from_value(row.clone()).unwrap();
            assert_eq!(entry.kind(), kind);
            assert_eq!(serde_json::to_value(&entry).unwrap(), *row);
            if let Some(result) = entry.result {
                serde_json::from_value::<UserCommandReply>(result)
                    .unwrap()
                    .check()
                    .unwrap();
            }
        }
    }
}
