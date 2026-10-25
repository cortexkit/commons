//! Shared helpers for the integration tests.
#![allow(dead_code, unused_imports)]

use cortexkit_persona::{
    AgentContext, ApprovalSet, Demand, EmbeddedTier, EnvContext, FragmentSources, GrantMap,
    HarnessContext, LoadRequest, LoadedRole, ModelContext, OsName, PersonaEngine, PersonaError,
    ProjectKey, RenderContext, Rendered, Tier, TrustClass,
};

pub const ROLE: &str = "writer";

pub fn context() -> RenderContext<serde_json::Value> {
    RenderContext {
        agent: AgentContext {
            name: Some("Ada".into()),
            agent_id: None,
            project: None,
            workspace: None,
            role: ROLE.into(),
        },
        model: ModelContext {
            provider: "example".into(),
            id: "model-1".into(),
            family: None,
            variant: "default".into(),
        },
        harness: HarnessContext {
            id: "harness".into(),
            capabilities: [("submit_tool".to_string(), true)].into(),
        },
        tools: GrantMap([("search".to_string(), true)].into()),
        mcp: GrantMap::default(),
        modules: GrantMap::default(),
        demand: Demand { iq: 50, eq: 50 },
        env: EnvContext {
            os: OsName::Darwin,
            timezone: "UTC".into(),
            locale: "en".into(),
        },
        module: serde_json::json!({"limit": 3}),
        roots: Default::default(),
        redeclared: Default::default(),
    }
}

/// A `PERSONA.md` with the three fields the first layer must carry.
pub fn full(extra: &str, body: &str) -> String {
    format!("---\nname: {ROLE}\nversion: 1\ndisplay_name: Writer\n{extra}---\n{body}")
}

/// A partial `PERSONA.md`.
pub fn partial(front: &str, body: &str) -> String {
    format!("---\n{front}---\n{body}")
}

/// An embedded tier holding `files` (logical path, text) for [`ROLE`].
pub fn tier(name: &str, trust: TrustClass, files: &[(&str, &str)]) -> Tier {
    let mut bundles = EmbeddedTier::new();
    for (path, text) in files {
        bundles.insert(ROLE, path, *text).unwrap();
    }
    Tier::embedded(name, trust, bundles).unwrap()
}

pub fn base(files: &[(&str, &str)]) -> Tier {
    tier("base", TrustClass::Operator, files)
}

pub fn engine() -> PersonaEngine {
    PersonaEngine::new("demo").unwrap()
}

pub fn load(tiers: &[Tier]) -> Result<LoadedRole, PersonaError> {
    engine().load(LoadRequest::new(ROLE, tiers, &ApprovalSet::new()))
}

pub fn load_approved(
    tiers: &[Tier],
    approvals: &ApprovalSet,
    key: &ProjectKey,
) -> Result<LoadedRole, PersonaError> {
    engine().load(LoadRequest::new(ROLE, tiers, approvals).project_key(key))
}

pub fn render(tiers: &[Tier]) -> Result<Rendered, PersonaError> {
    load(tiers)?.render(&context(), &FragmentSources::new())
}

pub fn text(tiers: &[Tier]) -> String {
    render(tiers).expect("renders").text
}
