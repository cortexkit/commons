//! The session reference (contract section 14.1).
//!
//! Every module that keeps state for a runner session derives the same short
//! reference from the route bind's identity, so they agree without asking the
//! runner. The reference is a correlation key, not proof of identity: a direct
//! caller chooses its own bind fields.

use sha2::{Digest, Sha256};

/// The first 16 bytes of SHA-256 over the RFC 8785 (JCS) encoding of
/// `{"harness", "project_root", "session"}`, as 32 lowercase hex characters.
///
/// Pass the three fields exactly as the route bind carried them. In particular
/// `project_root` must not be canonicalised: a runner keys its durable state by
/// those bytes, so a resolved symlink or a dropped trailing slash names a
/// different session.
pub fn session_ref(harness: &str, project_root: &str, session: &str) -> String {
    // JCS orders members by key; these three keys are already in that order, and
    // JCS string escaping matches serde_json's string encoding (quote,
    // backslash and control characters only). Building the object by hand keeps
    // the bytes independent of serde_json's map ordering, which a workspace build
    // can change through the `preserve_order` feature.
    let jcs = format!(
        r#"{{"harness":{},"project_root":{},"session":{}}}"#,
        json_string(harness),
        json_string(project_root),
        json_string(session),
    );
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
    use super::session_ref;
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
        project_root: String,
        session: String,
    }

    #[test]
    fn matches_the_pinned_vectors() {
        let vectors: Vectors =
            serde_json::from_str(crate::vectors::file("session-ref.json").unwrap()).unwrap();
        assert!(vectors.cases.len() >= 4);
        for case in vectors.cases {
            let Bind {
                harness,
                project_root,
                session,
            } = &case.bind;
            assert_eq!(
                format!(
                    r#"{{"harness":{},"project_root":{},"session":{}}}"#,
                    super::json_string(harness),
                    super::json_string(project_root),
                    super::json_string(session)
                ),
                case.jcs,
                "{}: JCS bytes",
                case.name
            );
            assert_eq!(
                session_ref(harness, project_root, session),
                case.session_ref,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn project_root_bytes_are_not_normalised() {
        assert_ne!(
            session_ref("runner", "/tmp/project/", "s1"),
            session_ref("runner", "/tmp/project", "s1")
        );
    }
}
