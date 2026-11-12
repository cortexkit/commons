//! Consumer workload kinds and placement constraints.

use serde::{Deserialize, Serialize};

open_set! {
    /// Workload kind. Omission means `llm`; readers preserve future kinds.
    pub enum Kind {
        Llm => "llm",
        Transcribe => "transcribe",
        StreamTranscribe => "stream_transcribe",
        Synthesize => "synthesize",
        Realtime => "realtime",
        Image => "image",
        Video => "video",
        Search => "search",
    }
}

open_set! {
    /// Open list of per-harness task capabilities.
    pub enum HarnessCapability {
        OneOff => "one_off",
        ForkAnswer => "fork_answer",
        OutputCap => "output_cap",
    }
}

/// A declaration constrains the runner module, not a particular harness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
#[non_exhaustive]
pub enum Placement {
    CallerRunner(CallerRunner),
    Module { module: String },
}

/// The string arm of [`Placement`]; no other string is a placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallerRunner {
    #[serde(rename = "caller_runner")]
    CallerRunner,
}

impl Placement {
    pub fn caller_runner() -> Self {
        Self::CallerRunner(CallerRunner::CallerRunner)
    }

    pub fn module(module: impl Into<String>) -> Self {
        Self::Module {
            module: module.into(),
        }
    }
}

/// Only the attested module's root and dotted descendants belong to it.
pub fn owns_consumer_name(module: &str, name: &str) -> bool {
    name == module
        || name
            .strip_prefix(module)
            .is_some_and(|tail| tail.starts_with('.'))
}

/// Identity and scope ownership come from daemon attestations, never params.
pub fn admits_selector(
    owner: &str,
    selectors: &[String],
    caller: &str,
    scope_owner: Option<&str>,
) -> bool {
    caller == owner
        || selectors.iter().any(|selector| selector == caller)
        || scope_owner == Some(owner)
}
