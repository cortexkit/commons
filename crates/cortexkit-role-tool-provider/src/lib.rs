//! Wire contract of the `tool-provider/v1` role.
//!
//! A tool provider is a module that serves tool definitions and executes
//! calls to them. It claims the role by listing [`PROVIDES`] in its manifest's
//! `capabilities.provides`, answers [`ops::ROLE_DESCRIBE`] and
//! [`ops::TOOL_CATALOG`], and executes calls to the tools its catalog names.
//! A provider that can hold a call past its own reply also serves
//! [`ops::TOOL_WITHDRAW`].
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document: it
//! lists every item this role pins and every item it leaves open. The types
//! here are the pinned wire shapes; where a shape is still open the type says
//! so and decodes leniently.
//!
//! Role ops travel as ordinary named requests on the provider's tool route,
//! `{name: <op>, arguments: {...}}`, like a tool call. They are never model
//! tools: a provider never lists them in its catalog.

#![forbid(unsafe_code)]

pub mod call;
pub mod catalog;
pub mod describe;
pub mod errors;
pub mod late_results;
pub mod points;
pub mod scope;
pub mod withdraw;

/// The role name as `role.describe` reports it.
pub const ROLE: &str = "tool-provider";

/// The only major this crate defines.
pub const VERSION: &str = "v1";

/// The string a provider lists in its manifest's `capabilities.provides`.
pub const PROVIDES: &str = "tool-provider/v1";

/// Names of the role's ops, as the `name` of the request that carries them.
pub mod ops {
    /// Discovery before use: the role's version, stability and ops. Required.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// The catalog-and-text fetch. Required.
    pub const TOOL_CATALOG: &str = "tool.catalog";
    /// Withdraw a call the provider holds. Required of a provider that can
    /// hold a call past its own reply; absent otherwise.
    pub const TOOL_WITHDRAW: &str = "tool.withdraw";
    /// Pull late results. Served by a provider that declares the
    /// `late_results` session capability.
    pub const LATE_RESULTS: &str = "late_results";
    /// Acknowledge late results through a cursor. Served with `late_results`.
    pub const LATE_RESULTS_ACK: &str = "late_results.ack";
}

/// Ops every `tool-provider/v1` module must list in `role.describe`. A
/// consumer refuses a module missing any of them before routing anything.
pub const REQUIRED_OPS: &[&str] = &[ops::ROLE_DESCRIBE, ops::TOOL_CATALOG];

/// The unprefixed capability tags this role defines. Each is a promise any
/// provider can make about a tool, so a preset or peer can match the tag
/// without naming a provider. Every other tag must carry a `<namespace>:`
/// prefix; see [`check_capability_tag`].
pub const DEFINED_CAPABILITY_TAGS: &[&str] = &[
    // Runs a shell command in the session's workspace and returns its output
    // and exit status.
    "shell.exec/v1",
    // Returns file contents, whole or by range.
    "code.read/v1",
    // Changes files in the workspace.
    "code.edit/v1",
    // Finds code by text, regular expression or meaning.
    "code.search/v1",
    // Lists or finds files by path pattern.
    "code.files/v1",
    // Returns the structure of a file or directory (symbols, headings).
    "code.outline/v1",
    // Answers caller, callee and impact questions about code.
    "code.callgraph/v1",
    // Returns compiler or linter diagnostics.
    "code.diagnostics/v1",
];

/// Why a capability tag is not acceptable in a catalog.
///
/// `#[non_exhaustive]` so a later check (for example on the tag's own shape)
/// can add a variant without breaking callers that match on this.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapabilityTagProblem {
    /// The tag has no `<namespace>:` prefix and this role does not define it.
    UndefinedUnprefixed,
    /// The tag has a `:` but nothing before it.
    EmptyNamespace,
}

impl std::fmt::Display for CapabilityTagProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UndefinedUnprefixed => write!(
                f,
                "an unprefixed tag must be one this role defines (DEFINED_CAPABILITY_TAGS); prefix it with a namespace"
            ),
            Self::EmptyNamespace => write!(f, "the namespace before ':' is empty"),
        }
    }
}

/// Check one capability tag: either one of [`DEFINED_CAPABILITY_TAGS`], or a
/// tag with a non-empty `<namespace>:` prefix, which anyone may define.
pub fn check_capability_tag(tag: &str) -> Result<(), CapabilityTagProblem> {
    match tag.split_once(':') {
        Some(("", _)) => Err(CapabilityTagProblem::EmptyNamespace),
        Some(_) => Ok(()),
        None if DEFINED_CAPABILITY_TAGS.contains(&tag) => Ok(()),
        None => Err(CapabilityTagProblem::UndefinedUnprefixed),
    }
}

#[cfg(test)]
mod capability_tag_tests {
    use super::*;

    #[test]
    fn defined_unprefixed_tags_are_accepted() {
        for tag in DEFINED_CAPABILITY_TAGS {
            assert_eq!(check_capability_tag(tag), Ok(()), "{tag}");
        }
    }

    #[test]
    fn namespaced_tags_are_accepted() {
        assert_eq!(check_capability_tag("acme:code.callgraph/v1"), Ok(()));
        assert_eq!(check_capability_tag("aft:safety/v1"), Ok(()));
    }

    #[test]
    fn undefined_unprefixed_tags_are_refused() {
        assert_eq!(
            check_capability_tag("code.refactor/v1"),
            Err(CapabilityTagProblem::UndefinedUnprefixed)
        );
        assert_eq!(
            check_capability_tag("context.reduce/v1"),
            Err(CapabilityTagProblem::UndefinedUnprefixed)
        );
    }

    #[test]
    fn an_empty_namespace_is_refused() {
        assert_eq!(
            check_capability_tag(":code.read/v1"),
            Err(CapabilityTagProblem::EmptyNamespace)
        );
    }
}

#[cfg(test)]
pub(crate) mod vectors {
    //! The shared test vectors, read from the repository's `test-vectors`
    //! directory so the wire crate, the conformance runner and any other
    //! implementation check the same bytes.

    pub fn load(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/../../test-vectors/tool-provider-v1/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }
}
