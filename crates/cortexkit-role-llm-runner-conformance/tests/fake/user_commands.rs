//! Test support recording command-provider calls, model inputs and host menus.

use super::*;
use cortexkit_role_llm_runner_conformance::wire::user_commands::{FrozenUserCommand, UserCommand};
use cortexkit_role_llm_runner_conformance::{UserCommandFixture, UserCommandObservation};

#[derive(Default)]
pub(super) struct Commands {
    pub sessions: BTreeMap<String, CommandSession>,
}

pub(super) struct CommandSession {
    fixture: UserCommandFixture,
    pub observation: UserCommandObservation,
}

impl FakeSubject {
    pub(super) fn prepare_commands(
        &self,
        session: &str,
        fixture: &UserCommandFixture,
    ) -> Result<(String, Map<String, Value>), HarnessError> {
        let menu = if fixture.interactive
            && (fixture.host_replies_outside_history
                || self.defects.user_command == Some("unsafe_host"))
        {
            vec!["ctx-status".into(), "ctx-wrapup".into()]
        } else {
            vec![]
        };
        self.world().user_commands.lock().unwrap().sessions.insert(
            session.into(),
            CommandSession {
                fixture: fixture.clone(),
                observation: UserCommandObservation {
                    host_commands: menu,
                    ..Default::default()
                },
            },
        );
        let mut fields = self.send_fields(session, true);
        fields
            .get_mut("plan")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("interactive".into(), json!(fixture.interactive));
        Ok((session.into(), fields))
    }
}

impl Module {
    pub(super) fn command_baseline(&self, session: &str) -> Baseline {
        let commands = self.world.user_commands.lock().unwrap();
        let Some(config) = commands.sessions.get(session) else {
            return baseline();
        };
        if !config.fixture.interactive {
            return baseline();
        }
        baseline().with_user_commands(
            ["ctx-status", "ctx-wrapup"]
                .into_iter()
                .map(|name| FrozenUserCommand {
                    provider: TOOL_MODULE.into(),
                    command: UserCommand::new(name, "Command description"),
                })
                .collect(),
        )
    }

    pub(super) fn command_collision(&self, session: &str) -> Option<Reply> {
        self.world.user_commands.lock().unwrap().sessions.get(session).and_then(|config| {
            (config.fixture.collision && self.world.defects.user_command != Some("collision")).then(|| refuse(errors::USER_COMMAND_COLLISION, Some(json!({"name":"ctx-status", "providers":[TOOL_MODULE,"other.tools"]}))))
        })
    }

    pub(super) fn capture_command_model_input(&self, session: &str, sess: &Sess) {
        if let Some(config) = self
            .world
            .user_commands
            .lock()
            .unwrap()
            .sessions
            .get_mut(session)
        {
            config.observation.model_inputs.push(json!({"tools":sess.tools, "messages":sess.messages.iter().map(|message| &message.body).collect::<Vec<_>>()}));
        }
    }

    pub(super) fn user_command(
        &self,
        stamp: &RouteStamp,
        session: &str,
        method: &str,
        params: Value,
    ) -> Result<Reply, Killed> {
        if !self.world.serves(groups::USER_COMMANDS) {
            return Ok(refuse(errors::USER_COMMANDS_UNSUPPORTED, None));
        }
        let _gate = self.user_command_gate.lock().unwrap();
        let mut sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get_mut(session) else {
            return Ok(refuse(errors::UNKNOWN_INVOCATION, None));
        };
        if sess.owner != stamp.principal && self.world.defects.user_command != Some("owner") {
            return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
        }
        if method == ops::SESSION_USER_COMMAND_STATUS {
            let call_key = params["call_key"].as_str().unwrap_or("");
            return Ok(sess
                .user_commands
                .values()
                .find(|command| command.call_key == call_key)
                .and_then(|command| command.status.clone())
                .map(Reply::Response)
                .unwrap_or_else(|| refuse(errors::UNKNOWN_INVOCATION, None)));
        }
        let mut commands = self.world.user_commands.lock().unwrap();
        let Some(config) = commands.sessions.get_mut(session) else {
            return Ok(refuse(
                errors::UNKNOWN_USER_COMMAND,
                Some(json!({"command":params["command"]})),
            ));
        };
        if !config.fixture.interactive && self.world.defects.user_command != Some("no_person") {
            return Ok(refuse(errors::NO_PERSON, None));
        }
        let command = params["command"].as_str().unwrap_or("").to_owned();
        if !["ctx-status", "ctx-wrapup"].contains(&command.as_str()) {
            return Ok(refuse(
                errors::UNKNOWN_USER_COMMAND,
                Some(json!({"command":command})),
            ));
        }
        let input = params["input"].as_str().unwrap_or("").to_owned();
        let invocation = params["invocation_id"].as_str().unwrap().to_owned();
        let call_key = format!("command/{}/{invocation}", stamp.principal);
        if let Some(stored) = sess.user_commands.get(&invocation).cloned() {
            if self.world.defects.user_command == Some("duplicate") {
                if let Some(repeated) = config.observation.provider_calls.last().cloned() {
                    config.observation.provider_calls.push(repeated);
                }
            }
            let retry_unknown = self.world.defects.user_command == Some("retry_after_crash")
                && stored
                    .answer
                    .as_ref()
                    .is_some_and(|answer| answer["reply"]["state"] == "outcome_unknown");
            drop(commands);
            drop(sessions);
            if retry_unknown {
                self.world
                    .user_commands
                    .lock()
                    .unwrap()
                    .sessions
                    .get_mut(session)
                    .unwrap()
                    .observation
                    .provider_calls
                    .push(json!({"name":"user_command.invoke", "arguments":{"command":stored.command,"input":stored.input}, "call_key":stored.call_key}));
                self.commit(session, json!({"kind":"user_command_forwarded"}))?;
            }
            return Ok(stored
                .answer
                .map(Reply::Response)
                .unwrap_or_else(|| refuse(errors::UNKNOWN_INVOCATION, None)));
        }
        drop(commands);
        drop(sessions);

        self.commit(
            session,
            json!({
                "kind":"user_command_reservation",
                "invocation":invocation,
                "call_key":call_key,
                "command":command,
                "input":input,
            }),
        )?;
        self.world
            .user_commands
            .lock()
            .unwrap()
            .sessions
            .get_mut(session)
            .unwrap()
            .observation
            .provider_calls
            .push(json!({"name":"user_command.invoke", "arguments":{"command":command,"input":input}, "call_key":call_key}));
        let (reply, mut status) = if command == "ctx-wrapup" {
            (
                json!({"state":"accepted","text":"command-private-reply"}),
                json!({"state":"done","text":"command-private-result","progress":["command-progress-1","command-progress-2"]}),
            )
        } else {
            (
                json!({"state":"done","text":"command-private-reply"}),
                json!({"state":"done","text":"command-private-reply","progress":[]}),
            )
        };
        if self.world.defects.user_command == Some("progress") {
            status["progress"] = json!(["command-progress-2", "command-progress-1"]);
        }
        if self.world.defects.user_command == Some("history") {
            if let Some(sess) = self.sessions.lock().unwrap().get_mut(session) {
                sess.push(
                    json!({"role":"assistant","parts":[{"type":"text","text":"command-private-reply"}]}),
                    "",
                    vec![],
                );
            }
        }
        self.commit(session, json!({"kind":"user_command_forwarded"}))?;
        let answer = json!({"call_key":call_key,"reply":reply});
        self.commit(
            session,
            json!({
                "kind":"user_command_result",
                "invocation":invocation,
                "answer":answer,
                "status":status,
            }),
        )?;
        Ok(Reply::Response(answer))
    }
}
