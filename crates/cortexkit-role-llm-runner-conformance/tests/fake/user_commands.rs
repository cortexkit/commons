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
    answers: BTreeMap<String, Value>,
    statuses: BTreeMap<String, Value>,
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
                answers: BTreeMap::new(),
                statuses: BTreeMap::new(),
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
    ) -> Reply {
        if !self.world.serves(groups::USER_COMMANDS) {
            return refuse(errors::USER_COMMANDS_UNSUPPORTED, None);
        }
        let mut sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get_mut(session) else {
            return refuse(errors::UNKNOWN_INVOCATION, None);
        };
        if sess.owner != stamp.principal && self.world.defects.user_command != Some("owner") {
            return refuse(errors::SCOPE_OWNER_MISMATCH, None);
        }
        let mut commands = self.world.user_commands.lock().unwrap();
        let Some(config) = commands.sessions.get_mut(session) else {
            return refuse(
                errors::UNKNOWN_USER_COMMAND,
                Some(json!({"command": params["command"]})),
            );
        };
        if method == ops::SESSION_USER_COMMAND_STATUS {
            return config
                .statuses
                .get(params["call_key"].as_str().unwrap_or(""))
                .cloned()
                .map(Reply::Response)
                .unwrap_or_else(|| refuse(errors::UNKNOWN_INVOCATION, None));
        }
        if !config.fixture.interactive && self.world.defects.user_command != Some("no_person") {
            return refuse(errors::NO_PERSON, None);
        }
        let command = params["command"].as_str().unwrap_or("");
        if !["ctx-status", "ctx-wrapup"].contains(&command) {
            return refuse(
                errors::UNKNOWN_USER_COMMAND,
                Some(json!({"command": command})),
            );
        }
        let invocation = params["invocation_id"].as_str().unwrap();
        let call_key = format!("command/{}/{invocation}", stamp.principal);
        if let Some(answer) = config.answers.get(invocation) {
            if self.world.defects.user_command == Some("duplicate") {
                let repeated = config.observation.provider_calls.last().unwrap().clone();
                config.observation.provider_calls.push(repeated);
            }
            return Reply::Response(answer.clone());
        }
        config.observation.provider_calls.push(json!({"name":"user_command.invoke", "arguments":{"command":command,"input":params["input"]}, "call_key":call_key}));
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
            sess.push(json!({"role":"assistant","parts":[{"type":"text","text":"command-private-reply"}]}), "", vec![]);
        }
        let answer = json!({"call_key":call_key,"reply":reply});
        config.answers.insert(invocation.into(), answer.clone());
        config.statuses.insert(call_key, status);
        Reply::Response(answer)
    }
}
