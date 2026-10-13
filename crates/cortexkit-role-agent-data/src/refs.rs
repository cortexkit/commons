//! Refs, the `cutId`, and the checks a participant makes on them.
//!
//! A ref is `{ref_kind, ref}`: an opaque handle for one piece of an agent's
//! data, the same pair the orchestrator or host registered with
//! `agent.register_session`. A request carries at most [`MAX_REFS`] of them.
//! A `cutId` is 16 random bytes in lowercase hex, minted by entorhinal per
//! move.
//!
//! [`Refs`] and [`CutId`] check themselves when they decode, so a request
//! with 257 refs or a malformed `cutId` does not decode at all, and the
//! participant answers `invalid_request` (CONTRACT.md §3).
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

use serde::{Deserialize, Serialize};

use crate::errors::RefusalCode;

/// The most refs one request may carry. Each ref yields at most one item and
/// at most one external entry, so a `prepare` reply, the largest, has at
/// most 512 entries. It stays under the 3 MiB working frame as long as each
/// entry encodes to under 6 KiB (3 MiB / 512); the fixed fields of an item
/// take about 300 bytes, so the bound holds unless a participant's refs,
/// `format` or `check` run to kilobytes.
pub const MAX_REFS: usize = 256;

/// The length of a `cutId` in characters: 16 bytes as lowercase hex.
pub const CUT_ID_LEN: usize = 32;

/// The length of a `sessionRef` in characters: the 16-byte hash
/// `cortexkit_role_llm_runner::session_ref` returns, as lowercase hex.
pub const SESSION_REF_LEN: usize = 32;

/// The length of a `digest` in characters: SHA-256 as lowercase hex.
pub const DIGEST_LEN: usize = 64;

/// One piece of an agent's data, as a participant names it. Both members
/// are opaque to entorhinal. Two refs are the same ref when both members
/// are equal. The ordering (by `ref_kind`, then `ref`) exists only so ref
/// lists can be compared as sets; it means nothing on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Ref {
    pub ref_kind: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

impl Ref {
    pub fn new(ref_kind: impl Into<String>, r#ref: impl Into<String>) -> Self {
        Self {
            ref_kind: ref_kind.into(),
            r#ref: r#ref.into(),
        }
    }
}

/// The `refs` list of a request: at most [`MAX_REFS`] refs, each with a
/// non-empty `ref_kind` and `ref`, no ref listed twice. Decoding refuses a
/// list that breaks any of these rules ([`check_refs`]); an empty list is
/// allowed. Order is kept, because replies that answer per ref (`busy`,
/// `diverged`) answer in request order, but order never makes two lists
/// different requests. Idempotency compares them as sets, and so does the
/// rule that every op after a `cutId`'s first must name the same refs that
/// first op named ([`crate::step::check_ref_set`]); see
/// [`Refs::same_set`] and [`Refs::sorted`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(try_from = "Vec<Ref>", into = "Vec<Ref>")]
pub struct Refs(Vec<Ref>);

impl Refs {
    /// The refs, or `invalid_request` when [`check_refs`] refuses them.
    pub fn new(refs: Vec<Ref>) -> Result<Self, RefusalCode> {
        check_refs(&refs)?;
        Ok(Self(refs))
    }

    pub fn as_slice(&self) -> &[Ref] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Ref> {
        self.0.iter()
    }

    pub fn into_vec(self) -> Vec<Ref> {
        self.0
    }

    /// Whether `self` and `other` hold the same refs, in any order. Both
    /// lists are free of duplicates, so this is set equality.
    pub fn same_set(&self, other: &Refs) -> bool {
        self.len() == other.len() && self.sorted() == other.sorted()
    }

    /// The same refs, sorted, so two lists that are the same set compare
    /// equal.
    pub fn sorted(&self) -> Refs {
        let mut refs = self.0.clone();
        refs.sort();
        Refs(refs)
    }
}

impl TryFrom<Vec<Ref>> for Refs {
    type Error = String;

    fn try_from(refs: Vec<Ref>) -> Result<Self, Self::Error> {
        if refs.len() > MAX_REFS {
            return Err(format!(
                "refs carries {} refs; the most a request may carry is {MAX_REFS}",
                refs.len()
            ));
        }
        if let Some(index) = refs.iter().position(|r| check_ref(r).is_err()) {
            return Err(format!("refs[{index}] has an empty ref_kind or ref"));
        }
        if let Some(index) = first_duplicate(&refs) {
            return Err(format!("refs[{index}] repeats an earlier ref"));
        }
        Ok(Self(refs))
    }
}

impl From<Refs> for Vec<Ref> {
    fn from(refs: Refs) -> Self {
        refs.0
    }
}

impl<'a> IntoIterator for &'a Refs {
    type Item = &'a Ref;
    type IntoIter = std::slice::Iter<'a, Ref>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// A move's `cutId`: exactly [`CUT_ID_LEN`] lowercase hex characters.
/// Decoding refuses anything else ([`check_cut_id`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct CutId(String);

impl CutId {
    /// The `cutId`, or `invalid_request` when [`check_cut_id`] refuses it.
    pub fn new(cut_id: impl Into<String>) -> Result<Self, RefusalCode> {
        let cut_id = cut_id.into();
        check_cut_id(&cut_id)?;
        Ok(Self(cut_id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CutId {
    type Error = String;

    fn try_from(cut_id: String) -> Result<Self, Self::Error> {
        match check_cut_id(&cut_id) {
            Ok(()) => Ok(Self(cut_id)),
            Err(_) => Err(format!(
                "cutId {cut_id:?} is not {CUT_ID_LEN} lowercase hex characters"
            )),
        }
    }
}

impl From<CutId> for String {
    fn from(cut_id: CutId) -> Self {
        cut_id.0
    }
}

impl std::fmt::Display for CutId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `value` is exactly `len` characters of lowercase hex
/// (`0-9`, `a-f`).
fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Checks a `cutId`: 32 lowercase hex characters, nothing else. Uppercase
/// hex is refused, so one cut has one spelling. Refused `invalid_request`.
pub fn check_cut_id(cut_id: &str) -> Result<(), RefusalCode> {
    if is_lower_hex(cut_id, CUT_ID_LEN) {
        Ok(())
    } else {
        Err(RefusalCode::InvalidRequest)
    }
}

/// Checks one ref: both `ref_kind` and `ref` are non-empty, since an empty
/// handle cannot name anything registered. Refused `invalid_request`.
pub fn check_ref(r: &Ref) -> Result<(), RefusalCode> {
    if r.ref_kind.is_empty() || r.r#ref.is_empty() {
        Err(RefusalCode::InvalidRequest)
    } else {
        Ok(())
    }
}

/// Checks a request's refs: at most [`MAX_REFS`], each passing
/// [`check_ref`], and no ref listed twice (a duplicate in `prepare` would
/// give `cut.seal` two items for one stream). An empty list passes.
/// Refused `invalid_request`. Whether the participant holds each ref is a
/// separate, later check (`unknown_ref` on the source).
pub fn check_refs(refs: &[Ref]) -> Result<(), RefusalCode> {
    if refs.len() > MAX_REFS {
        return Err(RefusalCode::InvalidRequest);
    }
    refs.iter().try_for_each(check_ref)?;
    match first_duplicate(refs) {
        Some(_) => Err(RefusalCode::InvalidRequest),
        None => Ok(()),
    }
}

/// The index of the first ref that repeats an earlier one, if any.
fn first_duplicate(refs: &[Ref]) -> Option<usize> {
    let mut seen = std::collections::HashSet::with_capacity(refs.len());
    refs.iter().position(|r| !seen.insert(r))
}

/// Whether `value` is spelled like a `sessionRef`: 32 lowercase hex
/// characters, the form `cortexkit_role_llm_runner::session_ref` returns.
pub fn is_session_ref(value: &str) -> bool {
    is_lower_hex(value, SESSION_REF_LEN)
}

/// Whether `value` is spelled like a `digest`: 64 lowercase hex characters,
/// a SHA-256. The source participant computes it in `prepare`, and it must
/// equal the digest engram's session door computes over the same bytes.
pub fn is_digest(value: &str) -> bool {
    is_lower_hex(value, DIGEST_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(n: usize) -> Vec<Ref> {
        (0..n)
            .map(|i| Ref::new("broca.session", format!("ses_{i}")))
            .collect()
    }

    #[test]
    fn cut_id_must_be_32_lowercase_hex() {
        assert_eq!(check_cut_id("0123456789abcdef0123456789abcdef"), Ok(()));
        for bad in [
            "",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcdeg",
            " 123456789abcdef0123456789abcdef",
        ] {
            assert_eq!(
                check_cut_id(bad),
                Err(RefusalCode::InvalidRequest),
                "{bad:?}"
            );
            assert!(CutId::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ref_cap_is_256() {
        assert_eq!(check_refs(&refs(MAX_REFS)), Ok(()));
        assert_eq!(
            check_refs(&refs(MAX_REFS + 1)),
            Err(RefusalCode::InvalidRequest)
        );
        assert!(Refs::new(refs(MAX_REFS)).is_ok());
        assert!(Refs::new(refs(MAX_REFS + 1)).is_err());
        assert!(
            serde_json::from_value::<Refs>(serde_json::to_value(refs(MAX_REFS + 1)).unwrap())
                .is_err()
        );
    }

    #[test]
    fn empty_ref_members_are_refused() {
        assert_eq!(check_refs(&[]), Ok(()));
        assert_eq!(
            check_ref(&Ref::new("", "x")),
            Err(RefusalCode::InvalidRequest)
        );
        assert_eq!(
            check_ref(&Ref::new("k", "")),
            Err(RefusalCode::InvalidRequest)
        );
    }

    #[test]
    fn duplicate_refs_are_refused() {
        let mut dup = refs(3);
        dup.push(Ref::new("broca.session", "ses_1"));
        assert_eq!(check_refs(&dup), Err(RefusalCode::InvalidRequest));
        assert!(Refs::new(dup.clone()).is_err());
        assert!(serde_json::from_value::<Refs>(serde_json::to_value(&dup).unwrap()).is_err());
        // The same handle under another kind is another ref.
        let kinds = vec![Ref::new("a", "x"), Ref::new("b", "x")];
        assert_eq!(check_refs(&kinds), Ok(()));
        assert!(serde_json::from_value::<Refs>(serde_json::to_value(&kinds).unwrap()).is_ok());
    }

    #[test]
    fn same_set_ignores_order_only() {
        let ab = Refs::new(vec![Ref::new("k", "a"), Ref::new("k", "b")]).unwrap();
        let ba = Refs::new(vec![Ref::new("k", "b"), Ref::new("k", "a")]).unwrap();
        let a = Refs::new(vec![Ref::new("k", "a")]).unwrap();
        assert!(ab.same_set(&ba));
        assert!(!ab.same_set(&a));
        assert!(Refs::default().same_set(&Refs::default()));
    }

    #[test]
    fn session_ref_and_digest_spellings() {
        assert!(is_session_ref("00112233445566778899aabbccddeeff"));
        assert!(!is_session_ref("00112233445566778899AABBCCDDEEFF"));
        assert!(is_digest(&"a".repeat(64)));
        assert!(!is_digest(&"a".repeat(63)));
    }
}
