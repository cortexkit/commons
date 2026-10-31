use cortexkit_role_compaction_provider::{
    lineage::MessageRef,
    view::{
        dispose_pending, CompactionView, TranscriptMessage, ViewDisposition, ViewProblem, ViewRange,
    },
};
use serde_json::json;

#[test]
fn pending_disposal_rejects_wrong_session_version_and_own_digest() {
    let session_ref = "0123456789abcdef0123456789abcdef";
    let view = CompactionView::new(
        session_ref,
        1,
        "lineage",
        MessageRef {
            ordinal: 0,
            mid: "m0".into(),
        },
        vec![ViewRange::keep(0, 1)],
    );
    let message_at = |ordinal| {
        (ordinal == 0)
            .then(|| TranscriptMessage::new(0, "m0", vec![], json!({"role":"user","content":[]})))
    };
    assert_eq!(
        dispose_pending(&view, session_ref, 1, None, "lineage", 0, message_at),
        ViewDisposition::Apply
    );

    let mut wrong_version = view.clone();
    wrong_version.view_version = 2;
    wrong_version.digest = wrong_version.compute_digest();
    let mut wrong_session = view.clone();
    wrong_session.session_ref = "fedcba9876543210fedcba9876543210".into();
    wrong_session.digest = wrong_session.compute_digest();
    for served in [wrong_version, wrong_session] {
        assert_eq!(served.check(), Ok(()));
        assert_eq!(
            dispose_pending(&served, session_ref, 1, None, "lineage", 0, message_at),
            ViewDisposition::Rejected(ViewProblem::NotRequested)
        );
    }

    let mut wrong_digest = view;
    wrong_digest.digest = "0".repeat(64);
    assert_eq!(
        dispose_pending(
            &wrong_digest,
            session_ref,
            1,
            None,
            "lineage",
            0,
            message_at
        ),
        ViewDisposition::Rejected(ViewProblem::DigestMismatch)
    );
    // The view's own digest is checked before a stale-version disposal.
    assert_eq!(
        dispose_pending(
            &wrong_digest,
            session_ref,
            1,
            Some(1),
            "lineage",
            0,
            message_at
        ),
        ViewDisposition::Rejected(ViewProblem::DigestMismatch)
    );
}
