use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    require, text, validate_code, validate_request_id, validate_version, Command, SubjectKind,
    ValidationError,
};

/// The same identity triple as the compaction role, without a dependency on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SessionRef {
    pub runner: String,
    pub harness: String,
    pub session: String,
}

impl SessionRef {
    pub fn new(
        runner: impl Into<String>,
        harness: impl Into<String>,
        session: impl Into<String>,
    ) -> Self {
        Self {
            runner: runner.into(),
            harness: harness.into(),
            session: session.into(),
        }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        for (field, value) in [
            ("runner", &self.runner),
            ("harness", &self.harness),
            ("session", &self.session),
        ] {
            require(text(value, 256), field)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Subject {
    Session { session: SessionRef },
    Agent { agent_id: String },
    Computer {},
    Project { project_id: String },
    Workspace { workspace_id: String },
}

impl Subject {
    pub fn session(session: SessionRef) -> Self {
        Self::Session { session }
    }
    pub fn agent(agent_id: impl Into<String>) -> Self {
        Self::Agent {
            agent_id: agent_id.into(),
        }
    }
    pub fn project(project_id: impl Into<String>) -> Self {
        Self::Project {
            project_id: project_id.into(),
        }
    }
    pub fn workspace(workspace_id: impl Into<String>) -> Self {
        Self::Workspace {
            workspace_id: workspace_id.into(),
        }
    }
    pub fn computer() -> Self {
        Self::Computer {}
    }
    pub fn kind(&self) -> SubjectKind {
        match self {
            Self::Session { .. } => SubjectKind::Session,
            Self::Agent { .. } => SubjectKind::Agent,
            Self::Computer {} => SubjectKind::Computer,
            Self::Project { .. } => SubjectKind::Project,
            Self::Workspace { .. } => SubjectKind::Workspace,
        }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        match self {
            Self::Session { session } => session.check().map_err(|e| e.at("session")),
            Self::Agent { agent_id } => require(text(agent_id, 128), "agent_id"),
            Self::Computer {} => Ok(()),
            Self::Project { project_id } => require(text(project_id, 128), "project_id"),
            Self::Workspace { workspace_id } => require(text(workspace_id, 128), "workspace_id"),
        }
    }
}

/// Forwarded, audited facts, never authority. All string vocabularies are open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Caller {
    pub app: String,
    pub admission: String,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub device: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub peer_build: Option<String>,
}

impl Caller {
    pub fn new(app: impl Into<String>, admission: impl Into<String>) -> Self {
        Self {
            app: app.into(),
            admission: admission.into(),
            device: None,
            peer_build: None,
        }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        require(!self.app.is_empty(), "app")?;
        require(!self.admission.is_empty(), "admission")?;
        if let Some(device) = &self.device {
            require(
                !device.is_empty() && device.bytes().all(|b| b.is_ascii_hexdigit()),
                "device",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DescribeRequest {}
impl DescribeRequest {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ListRequest {
    pub subject: Subject,
}
impl ListRequest {
    pub fn new(subject: Subject) -> Self {
        Self { subject }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        self.subject.check().map_err(|e| e.at("subject"))
    }
}

/// App-side params. The relay adds `caller` only after checking the route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AppPreviewRequest {
    pub module: String,
    pub id: String,
    pub subject: Subject,
    pub args: Map<String, Value>,
}
impl AppPreviewRequest {
    pub fn new(
        module: impl Into<String>,
        id: impl Into<String>,
        subject: Subject,
        args: Map<String, Value>,
    ) -> Self {
        Self {
            module: module.into(),
            id: id.into(),
            subject,
            args,
        }
    }
    pub fn check(&self, command: &Command) -> Result<Map<String, Value>, ValidationError> {
        require(!self.module.is_empty(), "module")?;
        check_call(&self.id, &self.subject, &self.args, command)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ModulePreviewRequest {
    pub id: String,
    pub subject: Subject,
    pub args: Map<String, Value>,
    pub caller: Caller,
}
impl ModulePreviewRequest {
    pub fn new(
        id: impl Into<String>,
        subject: Subject,
        args: Map<String, Value>,
        caller: Caller,
    ) -> Self {
        Self {
            id: id.into(),
            subject,
            args,
            caller,
        }
    }
    pub fn check(&self, command: &Command) -> Result<Map<String, Value>, ValidationError> {
        self.caller.check().map_err(|e| e.at("caller"))?;
        check_call(&self.id, &self.subject, &self.args, command)
    }
}

fn check_call(
    id: &str,
    subject: &Subject,
    args: &Map<String, Value>,
    command: &Command,
) -> Result<Map<String, Value>, ValidationError> {
    require(validate_code(id) && id == command.id, "id")?;
    subject.check().map_err(|e| e.at("subject"))?;
    require(subject.kind() == command.subject, "subject.kind")?;
    command.resolve_args(args)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AppRunRequest {
    pub module: String,
    pub id: String,
    pub subject: Subject,
    pub args: Map<String, Value>,
    pub preview_version: String,
    pub request_id: String,
}
impl AppRunRequest {
    pub fn new(
        preview: AppPreviewRequest,
        preview_version: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            module: preview.module,
            id: preview.id,
            subject: preview.subject,
            args: preview.args,
            preview_version: preview_version.into(),
            request_id: request_id.into(),
        }
    }
    pub fn check(&self, command: &Command) -> Result<Map<String, Value>, ValidationError> {
        require(!self.module.is_empty(), "module")?;
        check_run(&self.preview_version, &self.request_id, command)?;
        check_call(&self.id, &self.subject, &self.args, command)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ModuleRunRequest {
    pub id: String,
    pub subject: Subject,
    pub args: Map<String, Value>,
    pub preview_version: String,
    pub request_id: String,
    pub caller: Caller,
}
impl ModuleRunRequest {
    pub fn new(
        preview: ModulePreviewRequest,
        preview_version: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            id: preview.id,
            subject: preview.subject,
            args: preview.args,
            caller: preview.caller,
            preview_version: preview_version.into(),
            request_id: request_id.into(),
        }
    }
    pub fn check(&self, command: &Command) -> Result<Map<String, Value>, ValidationError> {
        self.caller.check().map_err(|e| e.at("caller"))?;
        check_run(&self.preview_version, &self.request_id, command)?;
        check_call(&self.id, &self.subject, &self.args, command)
    }
}

fn check_run(version: &str, request_id: &str, command: &Command) -> Result<(), ValidationError> {
    require(validate_version(version), "preview_version")?;
    require(validate_request_id(request_id), "request_id")?;
    require(command.offers_run(), "id")
}
