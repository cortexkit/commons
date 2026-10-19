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
pub mod result;
pub mod scope;
pub mod withdraw;

/// The daemon's bind-mode vocabulary and the manifest capability promising
/// that a provider reads it on every bind.
pub use subc_protocol::call_mode::{CallMode, CALL_MODES_CAPABILITY};

/// The role name as `role.describe` reports it.
pub const ROLE: &str = "tool-provider";

/// The only major this crate defines.
pub const VERSION: &str = "v1";

/// The string a provider lists in its manifest's `capabilities.provides`.
pub const PROVIDES: &str = "tool-provider/v1";

/// The optional `role.describe` capability for selecting catalog tools by
/// their capability tags.
pub const CATALOG_CAPABILITY_FILTER: &str = "tool-catalog-capability-filter/v1";

/// The route-bind capability a consumer declares when it decodes `content/v1` results.
pub const CONTENT_V1_CAPABILITY: &str = "content/v1";

/// The route-bind capability a consumer declares when it understands held replies.
pub const HELD_REPLY_V1: &str = "held-reply/v1";

/// Whether a route's bind says that its consumer decodes `content/v1` results.
///
/// Providers use this bind-scoped declaration, together with the tool's
/// catalog `result_format`, to choose the result body for each route.
pub fn consumer_reads_content_v1(consumer_capabilities: Option<&[String]>) -> bool {
    consumer_capabilities.is_some_and(|capabilities| {
        capabilities
            .iter()
            .any(|capability| capability == CONTENT_V1_CAPABILITY)
    })
}

/// Whether a route's bind permits the `held-reply/v1` marker.
///
/// Both consumer declarations are required. Providers must also check that the
/// tool uses `content/v1`, the call has a `call_key`, and they advertise
/// `late_results`. Neither per-call arguments nor `call_mode` grant support.
pub fn consumer_reads_held_reply_v1(consumer_capabilities: Option<&[String]>) -> bool {
    consumer_reads_content_v1(consumer_capabilities)
        && consumer_capabilities.is_some_and(|capabilities| {
            capabilities
                .iter()
                .any(|capability| capability == HELD_REPLY_V1)
        })
}

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

#[cfg(test)]
mod consumer_capability_tests {
    use super::{consumer_reads_content_v1, CONTENT_V1_CAPABILITY};

    #[test]
    fn absent_capabilities_do_not_declare_content_v1() {
        assert!(!consumer_reads_content_v1(None));
    }

    #[test]
    fn empty_capabilities_do_not_declare_content_v1() {
        assert!(!consumer_reads_content_v1(Some(&[])));
    }

    #[test]
    fn other_capabilities_do_not_declare_content_v1() {
        let capabilities = vec!["other/v1".to_owned(), "content/v2".to_owned()];
        assert!(!consumer_reads_content_v1(Some(&capabilities)));
    }

    #[test]
    fn content_v1_capability_declares_support() {
        let capabilities = vec!["other/v1".to_owned(), CONTENT_V1_CAPABILITY.to_owned()];
        assert!(consumer_reads_content_v1(Some(&capabilities)));
    }
}

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
    // Finds code in the session's own workspace by text, regular expression
    // or meaning. Searching code hosted elsewhere is code.search.public/v1.
    "code.search/v1",
    // Lists or finds files by path pattern.
    "code.files/v1",
    // Returns the structure of a file or directory (symbols, headings).
    "code.outline/v1",
    // Answers caller, callee and impact questions about code.
    "code.callgraph/v1",
    // Returns compiler or linter diagnostics.
    "code.diagnostics/v1",
    // Drives a web browser session: navigate, read the page and act on it.
    // The tool's own schema and contract say how; this tag promises only that
    // a browser is driven, so a preset can ask for one without naming a
    // provider. A provider adds its own namespaced tag for its specific
    // contract.
    "browser.use/v1",
    // Drives desktop applications: read their state and act on them. Promises
    // only that much, like browser.use/v1.
    "computer.use/v1",
    // Reads repository history: commits, file contents at a revision and
    // diffs between revisions. Read-only; how revisions are named and what
    // the output looks like are the provider's own.
    "git.history/v1",
    // Searches the general web and returns results. How results are ranked
    // and what each one carries are the provider's own.
    "web.search/v1",
    // Reads GitHub issues, pull requests and repository data without changing
    // anything on GitHub.
    "github.read/v1",
    // Searches code across public code hosts, outside the session's own
    // workspace.
    "code.search.public/v1",
];

/// Why a capability tag is not acceptable in a catalog.
///
/// `#[non_exhaustive]` so a later check can add a variant without breaking
/// callers that match on this.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapabilityTagProblem {
    /// The tag has no `<namespace>:` prefix and this role does not define it.
    UndefinedUnprefixed,
    /// The tag has a `:` but nothing before it.
    EmptyNamespace,
    /// The namespace is not lowercase letters and digits in `-`-separated
    /// words.
    MalformedNamespace,
    /// The part after the namespace is not `<name>/v<N>`: a name of
    /// `.`-separated words of lowercase letters, digits and `_`, then `/v` and
    /// a version with no leading zero.
    MalformedName,
}

impl std::fmt::Display for CapabilityTagProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UndefinedUnprefixed => write!(
                f,
                "an unprefixed tag must be one this role defines (DEFINED_CAPABILITY_TAGS); prefix it with a namespace"
            ),
            Self::EmptyNamespace => write!(f, "the namespace before ':' is empty"),
            Self::MalformedNamespace => write!(
                f,
                "the namespace must be lowercase letters and digits in '-'-separated words"
            ),
            Self::MalformedName => write!(
                f,
                "after the namespace a tag must be <name>/v<N>, for example code.callgraph/v1"
            ),
        }
    }
}

/// True for `<name>/v<N>`: `.`-separated words of `[a-z0-9_]`, then `/v` and
/// a positive version with no leading zero.
fn is_tag_name(text: &str) -> bool {
    let Some((name, version)) = text.rsplit_once("/v") else {
        return false;
    };
    let word = |w: &str| {
        !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    let version_ok = !version.is_empty()
        && !version.starts_with('0')
        && version.bytes().all(|b| b.is_ascii_digit());
    version_ok && name.split('.').all(word)
}

/// True for lowercase letters and digits in `-`-separated words.
fn is_namespace(text: &str) -> bool {
    text.split('-').all(|w| {
        !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

/// Check one capability tag: either one of [`DEFINED_CAPABILITY_TAGS`], or
/// `<namespace>:<name>/v<N>`, which anyone may define.
pub fn check_capability_tag(tag: &str) -> Result<(), CapabilityTagProblem> {
    match tag.split_once(':') {
        Some(("", _)) => Err(CapabilityTagProblem::EmptyNamespace),
        Some((namespace, _)) if !is_namespace(namespace) => {
            Err(CapabilityTagProblem::MalformedNamespace)
        }
        Some((_, name)) if !is_tag_name(name) => Err(CapabilityTagProblem::MalformedName),
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

    #[test]
    fn every_defined_tag_has_the_name_shape() {
        for tag in DEFINED_CAPABILITY_TAGS {
            assert!(is_tag_name(tag), "{tag}");
        }
    }

    #[test]
    fn known_namespaced_tags_have_the_shape() {
        for tag in [
            "aft:code.ast_grep/v1",
            "aft:git.conflicts/v1",
            "magic-context:context.reduce/v1",
            "fake:effect/v1",
            "acme:code.callgraph/v12",
        ] {
            assert_eq!(check_capability_tag(tag), Ok(()), "{tag}");
        }
    }

    #[test]
    fn a_namespaced_tag_without_a_name_and_version_is_refused() {
        for tag in [
            "acme:",
            "acme:code.read",
            "acme:code.read/v",
            "acme:code.read/v0",
            "acme:code.read/v01",
            "acme:Code.read/v1",
            "acme:code..read/v1",
            "acme:code.read/v1/extra",
        ] {
            assert_eq!(
                check_capability_tag(tag),
                Err(CapabilityTagProblem::MalformedName),
                "{tag}"
            );
        }
    }

    #[test]
    fn a_malformed_namespace_is_refused() {
        for tag in [
            "Acme:code.read/v1",
            "ac_me:code.read/v1",
            "acme-:code.read/v1",
        ] {
            assert_eq!(
                check_capability_tag(tag),
                Err(CapabilityTagProblem::MalformedNamespace),
                "{tag}"
            );
        }
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
