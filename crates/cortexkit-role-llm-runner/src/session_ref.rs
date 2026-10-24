//! The session reference (contract section 14.1).
//!
//! Every module that keeps state for a runner session derives the same short
//! reference from the runner module, harness and session, so they agree without asking the
//! runner. The reference is a correlation key, not proof of identity: a direct
//! caller chooses its own bind fields.

use sha2::{Digest, Sha256};

/// The named inputs to a hosted session's reference.
///
/// Named fields prevent legacy `(harness, project_root, session)` callers from
/// silently passing their old positional arguments to the new derivation.
/// The old call must be migrated explicitly:
///
/// ```compile_fail,E0061
/// use cortexkit_role_llm_runner::session_ref;
/// session_ref("harness-a", "/work/project", "session-1");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionIdentity<'a> {
    /// The attested runner principal's bare module id, exactly as recorded in
    /// `session_binding.runner`, never `reserved:<id>` or another display form.
    pub runner: &'a str,
    /// The session's caller-chosen harness, exactly as given.
    pub harness: &'a str,
    /// The session value, unique within the runner and harness, exactly as given.
    pub session: &'a str,
}

/// The first 16 bytes of SHA-256 over the RFC 8785 (JCS) encoding of
/// `{"harness", "runner", "session"}`, as 32 lowercase hex characters.
///
/// `identity.runner` is the attested runner principal's bare module id,
/// exactly as recorded in `session_binding.runner`, never the principal's
/// `reserved:<id>` display form or any prefix. Session values are unique per
/// runner and harness. This function does not strip prefixes or validate ids.
/// Pass every field exactly as given, without normalisation. Folder spellings
/// are deliberately absent so aliases of one project cannot split a hosted
/// session's identity.
///
/// ```
/// use cortexkit_role_llm_runner::{session_ref, SessionIdentity};
/// let reference = session_ref(&SessionIdentity {
///     runner: "runner-a",
///     harness: "harness-a",
///     session: "session-1",
/// });
/// assert_eq!(reference, "f93ec93e3559a70a8db019b29e0fa883");
/// ```
///
pub fn session_ref(identity: &SessionIdentity<'_>) -> String {
    // JCS orders members by key; these three keys are already in that order, and
    // JCS string escaping matches serde_json's string encoding (quote,
    // backslash and control characters only). Building the object by hand keeps
    // the bytes independent of serde_json's map ordering, which a workspace build
    // can change through the `preserve_order` feature.
    let jcs = format!(
        r#"{{"harness":{},"runner":{},"session":{}}}"#,
        json_string(identity.harness),
        json_string(identity.runner),
        json_string(identity.session),
    );
    hash_ref(&jcs)
}

/// The old folder-based reference, only for migrating stored references.
///
/// Modules re-key live sessions once; ended rows retain their v1 references.
/// Pass the old `(harness, project_root, session)` fields exactly as stored,
/// without normalisation. This helper will be removed in a later release.
#[deprecated(
    since = "0.7.0",
    note = "only for migrating stored refs; use session_ref"
)]
pub fn session_ref_v1(harness: &str, project_root: &str, session: &str) -> String {
    let jcs = format!(
        r#"{{"harness":{},"project_root":{},"session":{}}}"#,
        json_string(harness),
        json_string(project_root),
        json_string(session),
    );
    hash_ref(&jcs)
}

fn hash_ref(jcs: &str) -> String {
    let digest = Sha256::digest(jcs.as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("a string always serializes")
}

#[cfg(test)]
mod tests {
    use super::{session_ref, SessionIdentity};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Vectors {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        bind: Bind,
        jcs: String,
        session_ref: String,
    }

    #[derive(Deserialize)]
    struct Bind {
        harness: String,
        runner: String,
        session: String,
    }

    #[test]
    fn matches_the_pinned_vectors() {
        let vectors: Vectors =
            serde_json::from_str(crate::vectors::file("session-ref.json").unwrap()).unwrap();
        assert!(vectors.cases.len() >= 5);
        for case in vectors.cases {
            let Bind {
                harness,
                runner,
                session,
            } = &case.bind;
            assert_eq!(
                format!(
                    r#"{{"harness":{},"runner":{},"session":{}}}"#,
                    super::json_string(harness),
                    super::json_string(runner),
                    super::json_string(session)
                ),
                case.jcs,
                "{}: JCS bytes",
                case.name
            );
            assert_eq!(
                session_ref(&SessionIdentity {
                    runner,
                    harness,
                    session,
                }),
                case.session_ref,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn field_bytes_are_not_normalised() {
        let identity = SessionIdentity {
            runner: "runner-a",
            harness: "harness-a",
            session: "s1",
        };
        assert_ne!(
            session_ref(&SessionIdentity {
                runner: "runner-a ",
                ..identity
            }),
            session_ref(&identity)
        );
        assert_ne!(
            session_ref(&SessionIdentity {
                harness: "harness-a ",
                ..identity
            }),
            session_ref(&identity)
        );
        assert_ne!(
            session_ref(&SessionIdentity {
                session: "é",
                ..identity
            }),
            session_ref(&SessionIdentity {
                session: "e\u{301}",
                ..identity
            })
        );
    }

    #[test]
    fn bare_runner_id_and_principal_display_form_match_distinct_vectors() {
        let vectors: Vectors =
            serde_json::from_str(crate::vectors::file("session-ref.json").unwrap()).unwrap();
        let bare = vectors
            .cases
            .iter()
            .find(|case| case.name == "ascii")
            .unwrap();
        let display = vectors
            .cases
            .iter()
            .find(|case| case.name == "principal display form is not the bare module id")
            .unwrap();
        assert_eq!(bare.bind.runner, "runner-a");
        assert_eq!(display.bind.runner, "reserved:runner-a");
        assert_eq!(bare.bind.harness, display.bind.harness);
        assert_eq!(bare.bind.session, display.bind.session);
        assert_ne!(bare.session_ref, display.session_ref);
        for case in [bare, display] {
            assert_eq!(
                session_ref(&SessionIdentity {
                    runner: &case.bind.runner,
                    harness: &case.bind.harness,
                    session: &case.bind.session,
                }),
                case.session_ref,
                "{}",
                case.name
            );
        }
    }

    #[test]
    #[allow(deprecated)]
    fn matches_the_pinned_v1_vectors() {
        let vectors: serde_json::Value =
            serde_json::from_str(crate::vectors::file("session-ref-v1.json").unwrap()).unwrap();
        let cases = vectors["cases"].as_array().unwrap();
        assert!(cases.len() >= 4);
        for case in cases {
            let bind = &case["bind"];
            let harness = bind["harness"].as_str().unwrap();
            let project_root = bind["project_root"].as_str().unwrap();
            let session = bind["session"].as_str().unwrap();
            assert_eq!(
                format!(
                    r#"{{"harness":{},"project_root":{},"session":{}}}"#,
                    super::json_string(harness),
                    super::json_string(project_root),
                    super::json_string(session)
                ),
                case["jcs"].as_str().unwrap(),
                "{}: JCS bytes",
                case["name"]
            );
            assert_eq!(
                super::session_ref_v1(harness, project_root, session),
                case["session_ref"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}
