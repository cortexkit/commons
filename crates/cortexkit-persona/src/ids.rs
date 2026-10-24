//! Identifier grammars. Every id that becomes part of a path, a template name
//! or an approval key is checked here, so nothing downstream has to.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::PersonaError;

const MAX_ID_LEN: usize = 64;
const MAX_ROLE_LEN: usize = 128;

/// A bare id: lowercase ASCII letters, digits and hyphens, starting and ending
/// with a letter or digit, at most 64 bytes. Module ids, tier names and bare
/// fragment ids use it.
pub(crate) fn check_bare(what: &'static str, value: &str) -> Result<(), PersonaError> {
    let fail = |reason| {
        Err(PersonaError::InvalidId {
            what,
            value: value.to_string(),
            reason,
        })
    };
    if value.is_empty() {
        return fail("must not be empty");
    }
    if value.len() > MAX_ID_LEN {
        return fail("must be at most 64 bytes");
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return fail("may contain only lowercase letters, digits and hyphens");
    }
    if value.starts_with('-') || value.ends_with('-') {
        return fail("must start and end with a letter or digit");
    }
    Ok(())
}

/// A role id: lowercase ASCII letters, digits, `.`, `_` and `-`, starting with
/// a letter or digit, at most 128 bytes. It is a directory name in every tier,
/// so it can never be `.`, `..` or contain a separator.
pub(crate) fn check_role(value: &str) -> Result<(), PersonaError> {
    let fail = |reason| {
        Err(PersonaError::InvalidId {
            what: "role id",
            value: value.to_string(),
            reason,
        })
    };
    if value.is_empty() {
        return fail("must not be empty");
    }
    if value.len() > MAX_ROLE_LEN {
        return fail("must be at most 128 bytes");
    }
    if !value.bytes().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_' || b == b'.'
    }) {
        return fail("may contain only lowercase letters, digits, `.`, `_` and `-`");
    }
    let first = value.as_bytes()[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return fail("must start with a letter or digit");
    }
    Ok(())
}

/// A block id: a MiniJinja identifier restricted to lowercase ASCII letters,
/// digits and `_`, not starting with a digit, at most 64 bytes. Block names
/// must be identifiers in the template language, so hyphens are not allowed.
pub(crate) fn check_block(value: &str) -> Result<(), PersonaError> {
    let fail = |reason| {
        Err(PersonaError::InvalidId {
            what: "block id",
            value: value.to_string(),
            reason,
        })
    };
    if value.is_empty() {
        return fail("must not be empty");
    }
    if value.len() > MAX_ID_LEN {
        return fail("must be at most 64 bytes");
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return fail("may contain only lowercase letters, digits and `_`");
    }
    if value.as_bytes()[0].is_ascii_digit() {
        return fail("must not start with a digit");
    }
    Ok(())
}

/// One segment of a tier path or a workspace id: ASCII letters, digits, `.`,
/// `_` and `-`, at most 128 bytes, never `.` or `..`.
pub(crate) fn check_segment(what: &'static str, value: &str) -> Result<(), PersonaError> {
    let fail = |reason| {
        Err(PersonaError::InvalidId {
            what,
            value: value.to_string(),
            reason,
        })
    };
    if value.is_empty() {
        return fail("must not be empty");
    }
    if value.len() > MAX_ROLE_LEN {
        return fail("must be at most 128 bytes");
    }
    if value == "." || value == ".." {
        return fail("must not be `.` or `..`");
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        return fail("may contain only ASCII letters, digits, `.`, `_` and `-`");
    }
    Ok(())
}

/// A `/`-separated relative path made of [`check_segment`] segments.
pub(crate) fn check_relative_path(what: &'static str, value: &str) -> Result<(), PersonaError> {
    if value.is_empty() {
        return Err(PersonaError::InvalidId {
            what,
            value: value.to_string(),
            reason: "must not be empty",
        });
    }
    for segment in value.split('/') {
        check_segment(what, segment).map_err(|_| PersonaError::InvalidId {
            what,
            value: value.to_string(),
            reason: "must be `/`-separated segments of ASCII letters, digits, `.`, `_` and `-`, none of them `.` or `..`",
        })?;
    }
    Ok(())
}

/// A fragment id as declared in `fragments` and named by `include`.
///
/// A bare id (`header`) is a file in the role's own bundle. A qualified id
/// `owner/name@major` (`mc/memory-profile@1`) is supplied by the module at
/// render time and never read from a bundle.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FragmentId(String);

impl FragmentId {
    /// Parse and validate a fragment id.
    pub fn parse(value: &str) -> Result<Self, PersonaError> {
        if let Some((owner, rest)) = value.split_once('/') {
            let bad = |reason| PersonaError::InvalidId {
                what: "fragment id",
                value: value.to_string(),
                reason,
            };
            let (name, major) = rest
                .split_once('@')
                .ok_or_else(|| bad("a qualified id is `owner/name@major`"))?;
            check_bare("fragment id", owner)
                .and_then(|()| check_bare("fragment id", name))
                .map_err(|_| {
                    bad("owner and name may contain only lowercase letters, digits and hyphens")
                })?;
            let major_ok = !major.is_empty()
                && major.len() <= 9
                && major.bytes().all(|b| b.is_ascii_digit())
                && (major == "0" || !major.starts_with('0'));
            if !major_ok {
                return Err(bad("major must be a decimal integer without leading zeros"));
            }
        } else {
            check_bare("fragment id", value)?;
        }
        Ok(FragmentId(value.to_string()))
    }

    /// Whether the id is `owner/name@major`, supplied by the module.
    pub fn is_qualified(&self) -> bool {
        self.0.contains('/')
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FragmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for FragmentId {
    type Error = PersonaError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        FragmentId::parse(&value)
    }
}

impl From<FragmentId> for String {
    fn from(value: FragmentId) -> Self {
        value.0
    }
}

/// An opaque project identity supplied by the module, such as a registry id or
/// a repository's root-commit identity. Never a working directory: a moved
/// checkout keeps its approvals, and a different repository cloned into the
/// same path does not inherit them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectKey(String);

impl ProjectKey {
    /// Wrap a project identity. It must be non-empty and at most 512 bytes;
    /// the engine never interprets it.
    pub fn new(value: impl Into<String>) -> Result<Self, PersonaError> {
        let value = value.into();
        if value.is_empty() || value.len() > 512 {
            return Err(PersonaError::InvalidId {
                what: "project key",
                value,
                reason: "must be 1 to 512 bytes",
            });
        }
        Ok(ProjectKey(value))
    }

    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProjectKey {
    type Error = PersonaError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        ProjectKey::new(value)
    }
}

impl From<ProjectKey> for String {
    fn from(value: ProjectKey) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_id_grammar() {
        assert!(!FragmentId::parse("header").unwrap().is_qualified());
        assert!(FragmentId::parse("mc/memory-profile@1")
            .unwrap()
            .is_qualified());
        for bad in [
            "", "Header", "-a", "a-", "a_b", "mc/x", "mc/x@", "mc/x@01", "mc/X@1", "/x@1",
            "a/b/c@1", "mc/x@1.0",
        ] {
            assert!(FragmentId::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn role_and_segment_grammar() {
        check_role("athena.panel_member").unwrap();
        assert!(check_role(".hidden").is_err());
        assert!(check_role("a/b").is_err());
        assert!(check_segment("segment", "..").is_err());
        check_relative_path("path", ".cortexkit/demo/roles").unwrap();
        assert!(check_relative_path("path", "a/../b").is_err());
        assert!(check_relative_path("path", "/abs").is_err());
        assert!(check_relative_path("path", "a\\b").is_err());
    }
}
