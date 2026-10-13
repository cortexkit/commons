//! Idempotency: how a participant answers a request it may have answered
//! before.
//!
//! Every op that changes state (`prepare`, `export`, `import`, `activate`,
//! `abort`, `release`) is keyed by `(cutId, op)`, and the whole request is
//! compared. A repeat that is the same request gets the first reply back;
//! a repeat with any field different (another ref set, `holder`, anything
//! else) is refused `id_reused`. The reads (`describe`, `busy`, `status`,
//! `diverged`) are exempt: they are answered afresh every time.
//!
//! The comparison is on the decoded request, so member order and
//! whitespace on the wire never make two requests differ. `refs` compare as
//! a set: a driver that rebuilds its list after a crash may reorder it, and
//! a reordered list is the same request, never `id_reused` (which would
//! stop the move for good).
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

use crate::{errors::RefusalCode, refs::CutId, request::Request, Op};

/// How a request relates to an earlier one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    /// Either request is a read. Reads are not keyed; answer afresh.
    Exempt,
    /// A different `(cutId, op)`: the earlier request says nothing about
    /// this one.
    NewKey,
    /// The same key and the same request (refs compared as a set): answer
    /// with the first reply.
    Replay,
    /// The same key and a different request: refuse `id_reused`.
    Reused,
}

impl Repeat {
    /// The refusal this outcome calls for: `id_reused` for
    /// [`Repeat::Reused`], none otherwise.
    pub fn refusal(self) -> Option<RefusalCode> {
        (self == Self::Reused).then_some(RefusalCode::IdReused)
    }
}

/// The idempotency key of a request: `(cutId, op)`, or `None` for a read.
pub fn key(request: &Request) -> Option<(&CutId, Op)> {
    let op = request.op();
    if !op.changes_state() {
        return None;
    }
    request.cut_id().map(|cut_id| (cut_id, op))
}

/// Decides whether `again` repeats `first`, the request a participant
/// recorded for some `(cutId, op)`: the same request (answer with the first
/// reply), a conflicting one (`id_reused`), a different key, or a read.
pub fn compare(first: &Request, again: &Request) -> Repeat {
    match (key(first), key(again)) {
        (Some(a), Some(b)) if a == b => {
            if first.with_sorted_refs() == again.with_sorted_refs() {
                Repeat::Replay
            } else {
                Repeat::Reused
            }
        }
        (Some(_), Some(_)) => Repeat::NewKey,
        _ => Repeat::Exempt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        refs::{Ref, Refs},
        release::ReleaseRequest,
        request::CutRequest,
    };

    const CUT: &str = "0123456789abcdef0123456789abcdef";
    const OTHER_CUT: &str = "fedcba9876543210fedcba9876543210";

    fn refs(names: &[&str]) -> Refs {
        Refs::new(
            names
                .iter()
                .map(|n| Ref::new("broca.session", *n))
                .collect(),
        )
        .unwrap()
    }

    fn cut(refs_: &[&str], cut_id: &str) -> CutRequest {
        CutRequest::new(refs(refs_), CutId::new(cut_id).unwrap())
    }

    fn release(holder: &str) -> Request {
        Request::Release(ReleaseRequest::new(
            refs(&["a"]),
            CutId::new(CUT).unwrap(),
            holder,
        ))
    }

    #[test]
    fn identical_repeat_replays() {
        let first = Request::Prepare(cut(&["a", "b"], CUT));
        assert_eq!(compare(&first, &first.clone()), Repeat::Replay);
        assert_eq!(compare(&release("h1"), &release("h1")), Repeat::Replay);
    }

    #[test]
    fn reordered_refs_are_the_same_request() {
        let first = Request::Prepare(cut(&["a", "b"], CUT));
        let reordered = Request::Prepare(cut(&["b", "a"], CUT));
        assert_eq!(compare(&first, &reordered), Repeat::Replay);
        let release_ab = Request::Release(ReleaseRequest::new(
            refs(&["a", "b"]),
            CutId::new(CUT).unwrap(),
            "h1",
        ));
        let release_ba = Request::Release(ReleaseRequest::new(
            refs(&["b", "a"]),
            CutId::new(CUT).unwrap(),
            "h1",
        ));
        assert_eq!(compare(&release_ab, &release_ba), Repeat::Replay);
    }

    #[test]
    fn any_field_differing_is_id_reused() {
        let first = Request::Prepare(cut(&["a", "b"], CUT));
        let fewer = Request::Prepare(cut(&["a"], CUT));
        assert_eq!(compare(&first, &fewer), Repeat::Reused);
        assert_eq!(compare(&release("h1"), &release("h2")), Repeat::Reused);
        assert_eq!(Repeat::Reused.refusal(), Some(RefusalCode::IdReused));
        assert_eq!(Repeat::Replay.refusal(), None);
    }

    #[test]
    fn other_cut_or_op_is_a_new_key() {
        let first = Request::Prepare(cut(&["a"], CUT));
        assert_eq!(
            compare(&first, &Request::Prepare(cut(&["a"], OTHER_CUT))),
            Repeat::NewKey
        );
        assert_eq!(
            compare(&first, &Request::Export(cut(&["a"], CUT))),
            Repeat::NewKey
        );
    }

    #[test]
    fn reads_are_exempt() {
        let status = Request::Status(cut(&["a"], CUT));
        let other = Request::Status(cut(&["b"], CUT));
        assert_eq!(compare(&status, &other), Repeat::Exempt);
        let diverged = Request::Diverged(cut(&["a"], CUT));
        assert_eq!(
            compare(&diverged, &Request::Diverged(cut(&["b"], CUT))),
            Repeat::Exempt
        );
        assert_eq!(key(&status), None);
    }
}
