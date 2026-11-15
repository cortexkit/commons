//! Search `route.select` decoding, separate from llm and media requirements.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{errors::Refusal, members::Members, select::RouteRef, select_engine::Privacy};

pub const ROLE_DECLARATIONS_KEY: &str = "search-provider/v1";
pub const REQUIREMENT_FIELDS: &[&str] = &["iq"];

/// A catalog search row, never an account or connection identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SearchRoute {
    pub provider: String,
    pub model: String,
}

impl SearchRoute {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SearchDeclarations {
    pub routes: Vec<SearchRoute>,
}

impl SearchDeclarations {
    pub fn new(routes: Vec<SearchRoute>) -> Self {
        Self { routes }
    }
}

/// A rejected slot is applied atomically: no route from it may be retained.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SearchDeclarationsError {
    Invalid { field: String },
    DuplicateRoute { index: usize },
}

impl SearchDeclarationsError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid { .. } => "invalid_declaration",
            Self::DuplicateRoute { .. } => "duplicate_route",
        }
    }
}

impl std::fmt::Display for SearchDeclarationsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid { field } => write!(f, "invalid_declaration: {field}"),
            Self::DuplicateRoute { index } => write!(f, "duplicate_route: routes[{index}]"),
        }
    }
}

impl std::error::Error for SearchDeclarationsError {}

fn valid_fact(value: &str) -> bool {
    (1..=256).contains(&value.chars().count())
}

/// Parse only the slot; nonce verification and health reporting belong to the router.
pub fn parse_search_declarations(
    value: &Value,
) -> Result<SearchDeclarations, SearchDeclarationsError> {
    let invalid = |field: &str| SearchDeclarationsError::Invalid {
        field: field.into(),
    };
    let object = value.as_object().ok_or_else(|| invalid("slot"))?;
    for key in object.keys() {
        if key != "routes" {
            return Err(invalid(key));
        }
    }
    let routes = object
        .get("routes")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("routes"))?;
    let mut seen = BTreeSet::new();
    let mut parsed = Vec::new();
    for (index, value) in routes.iter().enumerate() {
        let path = format!("routes[{index}]");
        let object = value.as_object().ok_or_else(|| invalid(&path))?;
        for key in object.keys() {
            if !["provider", "model"].contains(&key.as_str()) {
                return Err(invalid(&format!("{path}.{key}")));
            }
        }
        let fact = |key| {
            object
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| valid_fact(value))
                .ok_or_else(|| invalid(&format!("{path}.{key}")))
        };
        let provider = fact("provider")?;
        let model = fact("model")?;
        if !seen.insert((provider, model)) {
            return Err(SearchDeclarationsError::DuplicateRoute { index });
        }
        parsed.push(SearchRoute::new(provider, model));
    }
    Ok(SearchDeclarations::new(parsed))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SearchRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iq: Option<u8>,
}

impl SearchRequirements {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_iq(mut self, iq: u8) -> Self {
        self.iq = Some(iq);
        self
    }
}

/// Decoded only for a caller-sent `kind: "search"`; answers reuse `SelectMediaAnswer`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SelectSearchRequest {
    pub consumer: String,
    pub kind: String,
    pub requirements: SearchRequirements,
    pub privacy: Privacy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_providers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eligible_routes: Option<Vec<RouteRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance_left: Option<Vec<SearchRoute>>,
}

impl SelectSearchRequest {
    pub fn new(
        consumer: impl Into<String>,
        requirements: SearchRequirements,
        privacy: Privacy,
    ) -> Self {
        Self {
            consumer: consumer.into(),
            kind: "search".into(),
            requirements,
            privacy,
            pin: None,
            exclude: None,
            exclude_providers: None,
            eligible_routes: None,
            allowance_left: None,
        }
    }
}

pub fn parse_select_search_request<C: AsRef<str>, K: AsRef<str>>(
    value: &Value,
    capabilities: &[C],
    kinds: &[K],
) -> Result<SelectSearchRequest, Refusal> {
    let members = Members::of(value)?;
    let kind: String = members.required("kind")?;
    if kind != "search"
        || !kinds.iter().any(|kind| kind.as_ref() == "search")
        || [
            crate::capabilities::SELECT_SEARCH,
            crate::capabilities::SELECT_MEDIA,
            crate::capabilities::SELECT_KIND,
        ]
        .iter()
        .any(|group| !capabilities.iter().any(|c| c.as_ref() == *group))
    {
        return Err(members.refuse("kind", "search is not served"));
    }
    for key in members.object.keys() {
        if ![
            "consumer",
            "kind",
            "requirements",
            "privacy",
            "pin",
            "exclude",
            "exclude_providers",
            "eligible_routes",
            "allowance_left",
        ]
        .contains(&key.as_str())
        {
            return Err(members.refuse(key, "unknown member"));
        }
    }
    let requirements = members
        .object
        .get("requirements")
        .and_then(Value::as_object)
        .ok_or_else(|| Refusal::invalid_requirements("requirements", "expected an object"))?;
    let requirements = Members {
        object: requirements,
        refuse: |field, message| Refusal::invalid_requirements(field, message),
    };
    for key in requirements.object.keys() {
        if key != "iq" {
            return Err(requirements.refuse(key, "unknown requirement"));
        }
    }
    let iq: Option<u8> = requirements.optional("iq")?;
    if iq.is_some_and(|n| n > 100) {
        return Err(requirements.refuse("iq", "expected 0 to 100"));
    }
    let privacy = members.required("privacy")?;
    let mut common = value.clone();
    let object = common.as_object_mut().unwrap();
    object.remove("privacy");
    object.remove("allowance_left");
    object.insert("requirements".into(), serde_json::json!({"iq":0}));
    let common = crate::select::parse_select_request_serving(&common, capabilities)?;
    if common
        .pin
        .as_ref()
        .is_some_and(|pin| !crate::select::is_plain_route_key(pin))
    {
        return Err(Refusal::pin_unavailable(
            common.pin.unwrap(),
            crate::errors::PinUnavailableReason::Invalid,
            None,
        ));
    }
    let allowance_left = match members
        .object
        .get("allowance_left")
        .filter(|v| !v.is_null())
    {
        None => None,
        Some(value) => {
            let list = value
                .as_array()
                .ok_or_else(|| members.refuse("allowance_left", "expected a list"))?;
            let mut parsed = Vec::new();
            for (index, value) in list.iter().enumerate() {
                let field = format!("allowance_left[{index}]");
                let route: SearchRoute = serde_json::from_value(value.clone())
                    .map_err(|_| members.refuse(&field, "expected a provider/model route"))?;
                if !valid_fact(&route.provider)
                    || !valid_fact(&route.model)
                    || !common.eligible_routes.as_ref().is_some_and(|routes| {
                        routes
                            .iter()
                            .any(|r| r.provider == route.provider && r.model == route.model)
                    })
                {
                    return Err(
                        members.refuse(&field, "allowance route must appear in eligible_routes")
                    );
                }
                parsed.push(route);
            }
            Some(parsed)
        }
    };
    Ok(SelectSearchRequest {
        consumer: common.consumer,
        kind,
        requirements: SearchRequirements { iq },
        privacy,
        pin: common.pin,
        exclude: common.exclude,
        exclude_providers: common.exclude_providers,
        eligible_routes: common.eligible_routes,
        allowance_left,
    })
}

/// Validate the remote module matrix and shared route policy without guessing catalog facts.
pub fn check_answer(
    request: &SelectSearchRequest,
    answer: &crate::select_media::SelectMediaAnswer,
) -> Result<(), Refusal> {
    if answer.engine.local
        || answer.engine.caller_local
        || answer.engine.module.as_ref().is_none_or(|m| m.is_empty())
        || answer.engine.index.is_some()
    {
        return Err(Refusal::invalid_request(
            "engine",
            "search requires a remote module engine",
        ));
    }
    let mut media = crate::select_media::SelectMediaRequest::new(
        &request.consumer,
        crate::select_engine::Kind::Unknown("search".into()),
        crate::select_media::MediaRequirements::default(),
        request.privacy.clone(),
    );
    media.pin = request.pin.clone();
    media.exclude = request.exclude.clone();
    media.exclude_providers = request.exclude_providers.clone();
    media.eligible_routes = request.eligible_routes.clone();
    crate::select_media::check_answer(&media, answer)
}
