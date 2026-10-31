//! Served bytes rebuilt from a host's store (CONTRACT §4.2.1).
//!
//! Read stability (§4.2) binds a `mid` to one exact `message` body for its
//! lineage. A runner that reads from a host need not keep that body: it may
//! keep the body's [`content_digest`] and a `stripped` flag, and rebuild each
//! read from the host's stored row. [`served_body`] builds the body, removing
//! the members the profile's strip list names, and [`check_served`] checks it
//! against the stored digest. On any difference the runner serves nothing:
//! it refuses the read [`SERVED_BYTES_CHANGED`](crate::errors::SERVED_BYTES_CHANGED)
//! and serves the message again only under a new lineage.
//!
//! A strip list names the members a host rewrites after a message finished
//! without changing what the model sees. Leaving them out of every served
//! body is what lets a rebuilt read match the digest. The lists belong to
//! the contract and change only in a release of this crate.

use serde::Serialize;
use serde_json::Value;

pub use crate::archive_prunes_reasoning::UnsupportedProfile;
use crate::content_digest::content_digest;
use crate::serializer_profile::{ANTHROPIC_MESSAGES, OPENCODE_AISDK, OPENCODE_V2, OWNED_BROCA, PI};

/// One member a profile leaves out of a served body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StripMember {
    /// RFC 6901 pointer to the object that holds the member.
    pub parent: &'static str,
    /// The member's name in that object.
    pub member: &'static str,
    /// Remove the member only from a message matching this condition;
    /// `None` removes it from every message.
    pub when: Option<StripCondition>,
}

/// A message matches when the string at `pointer` equals `equals`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StripCondition {
    pub pointer: &'static str,
    pub equals: &'static str,
}

/// OpenCode writes a user message's `info.summary`, its file-change summary,
/// after the turn. An assistant message's `info.summary` is a different
/// member, the finished-summary flag (§4.8), and is never removed.
const OPENCODE_AISDK_STRIP: &[StripMember] = &[StripMember {
    parent: "/info",
    member: "summary",
    when: Some(StripCondition {
        pointer: "/info/role",
        equals: "user",
    }),
}];

/// The members `profile` leaves out of a served body (CONTRACT §4.2.1).
/// A profile whose host rewrites nothing after a message finished has an
/// empty list; a profile this crate does not name is refused, so a session
/// of an unknown profile is never rebuilt by the wrong rule.
pub fn strip_list(profile: &str) -> Result<&'static [StripMember], UnsupportedProfile> {
    match profile {
        OPENCODE_AISDK => Ok(OPENCODE_AISDK_STRIP),
        OPENCODE_V2 | PI | ANTHROPIC_MESSAGES | OWNED_BROCA => Ok(&[]),
        other => Err(UnsupportedProfile::new(other)),
    }
}

/// The body a runner serves for `host_row`. With `stripped`, which every
/// message ingested since the runner adopted its profile's strip list
/// carries, each listed member is removed where its condition holds; a member
/// the row lacks is not an error. Without it, the row is the body: a message
/// served before adoption keeps the bytes it was served with.
pub fn served_body(
    profile: &str,
    host_row: &Value,
    stripped: bool,
) -> Result<Value, UnsupportedProfile> {
    let list = strip_list(profile)?;
    let mut body = host_row.clone();
    if !stripped {
        return Ok(body);
    }
    for strip in list {
        if let Some(when) = strip.when {
            if body.pointer(when.pointer).and_then(Value::as_str) != Some(when.equals) {
                continue;
            }
        }
        if let Some(Value::Object(parent)) = body.pointer_mut(strip.parent) {
            parent.remove(strip.member);
        }
    }
    Ok(body)
}

/// Why a rebuilt read can't be served.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServedBytesError {
    /// [`strip_list`] does not name the session's profile.
    UnsupportedProfile(UnsupportedProfile),
    /// The rebuilt body's digest differs from the one the `mid` was first
    /// served with: a host change the strip list does not explain. The read
    /// is refused `served_bytes_changed`; neither digest is shown to the
    /// reader.
    Changed { served: String, rebuilt: String },
}

impl std::fmt::Display for ServedBytesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedProfile(profile) => profile.fmt(f),
            Self::Changed { served, rebuilt } => write!(
                f,
                "rebuilt body digest {rebuilt} differs from the served digest {served}"
            ),
        }
    }
}

impl std::error::Error for ServedBytesError {}

/// Rebuild the body for `host_row` and check it against `served_digest`,
/// the [`content_digest`] of the body the `mid` was first served with.
/// Returns the body to serve only when the digests match.
pub fn check_served(
    profile: &str,
    host_row: &Value,
    stripped: bool,
    served_digest: &str,
) -> Result<Value, ServedBytesError> {
    let body =
        served_body(profile, host_row, stripped).map_err(ServedBytesError::UnsupportedProfile)?;
    let rebuilt = content_digest(&body);
    if rebuilt == served_digest {
        Ok(body)
    } else {
        Err(ServedBytesError::Changed {
            served: served_digest.to_owned(),
            rebuilt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn strip_lists_match_the_vector() {
        let file = vectors::load("served-bytes.json");
        let lists = file["strip_lists"].as_object().unwrap();
        for (profile, expected) in lists {
            let list = strip_list(profile).unwrap();
            assert_eq!(&serde_json::to_value(list).unwrap(), expected, "{profile}");
        }
        assert_eq!(
            lists.len(),
            5,
            "every named profile has a listed strip list"
        );
    }

    #[test]
    fn every_served_bytes_case_matches_the_vector() {
        let file = vectors::load("served-bytes.json");
        for case in vectors::cases(&file, "cases") {
            let name = case["name"].as_str().unwrap();
            let profile = case["profile"].as_str().unwrap();
            let stripped = case["stripped"].as_bool().unwrap();
            let digest = case["served_digest"].as_str().unwrap();
            let outcome = check_served(profile, &case["host_row"], stripped, digest);
            match case["outcome"].as_str().unwrap() {
                "served" => {
                    let body = outcome.unwrap_or_else(|e| panic!("{name}: {e}"));
                    assert_eq!(body, case["served_body"], "{name}");
                    assert_eq!(content_digest(&body), digest, "{name}");
                }
                "changed" => assert_eq!(
                    outcome,
                    Err(ServedBytesError::Changed {
                        served: digest.to_owned(),
                        rebuilt: case["rebuilt_digest"].as_str().unwrap().to_owned(),
                    }),
                    "{name}"
                ),
                "unsupported_profile" => assert_eq!(
                    outcome,
                    Err(ServedBytesError::UnsupportedProfile(
                        UnsupportedProfile::new(profile)
                    )),
                    "{name}"
                ),
                other => panic!("{name}: unknown outcome {other}"),
            }
        }
    }

    #[test]
    fn unstripped_body_is_the_row_byte_for_byte() {
        let file = vectors::load("served-bytes.json");
        let case = vectors::cases(&file, "cases")
            .iter()
            .find(|case| case["name"] == "user_row_stripped")
            .unwrap();
        let body = served_body(OPENCODE_AISDK, &case["host_row"], false).unwrap();
        assert_eq!(body, case["host_row"]);
        assert!(body.pointer("/info/summary").is_some());
    }
}
