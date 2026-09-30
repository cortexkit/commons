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
//! Role ops travel as ordinary tool-call requests on the provider's tool
//! route, named by the op, the same way `tool.withdraw` does.

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
}

/// Ops every `tool-provider/v1` module must list in `role.describe`. A
/// consumer refuses a module missing any of them before routing anything.
pub const REQUIRED_OPS: &[&str] = &[ops::ROLE_DESCRIBE, ops::TOOL_CATALOG];

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
