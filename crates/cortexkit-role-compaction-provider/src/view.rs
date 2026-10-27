//! Views and `view.get` (CONTRACT.md §6, §7).
//!
//! A view is the provider's preferred shape for the prefix of a runner's
//! transcript through one message: a list of ranges over the runner's
//! immutable transcript, each kept as written or replaced with bytes the
//! provider supplies. Messages after the view's coverage go out unchanged. A
//! view never replaces or rewrites the runner's transcript or write-ahead
//! log; the runner keeps it beside them.

use cortexkit_role_llm_runner::archive_prunes_reasoning::{
    restored_by_edit, restored_part, PrunedPartAt,
};
use serde::{de::IgnoredAny, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::lineage::MessageRef;
use crate::{
    digest::{finish, frame, is_sha256_hex},
    notice::{is_session_ref, ViewReady},
};

/// Upper bound on a view's compact JSON encoding, in bytes. It leaves room
/// for the reply envelope under the 4 MiB transport frame.
pub const MAX_VIEW_BYTES: usize = 3_145_728;

/// The domain string that starts a view's digest input.
pub const VIEW_DIGEST_DOMAIN: &str = "ck.compaction.view.v1";

/// Upper bound on a part key, in bytes.
pub const MAX_PART_KEY_BYTES: usize = 256;

/// One edit of one part of a message inside a keep range: the message stays,
/// and only this part changes. `part` is `llm-runner/v1`'s opaque part key,
/// the key its `tags` rows and `session.resolve` use. Exactly one of `bytes`
/// and `drop` is given.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct PartEdit {
    /// The message's ordinal; inside the enclosing keep range.
    pub ordinal: u64,
    /// The message's id at that ordinal, checked against the transcript.
    pub mid: String,
    /// The part key, 1 to [`MAX_PART_KEY_BYTES`] bytes, compared byte for
    /// byte.
    pub part: String,
    /// The replacement part: UTF-8 JSON in the runner's message schema,
    /// final bytes sent exactly as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
    /// True removes the part outright. Omitted when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub drop: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl PartEdit {
    /// An edit that replaces the part with `bytes`.
    pub fn replace(
        ordinal: u64,
        mid: impl Into<String>,
        part: impl Into<String>,
        bytes: impl Into<String>,
    ) -> Self {
        Self {
            ordinal,
            mid: mid.into(),
            part: part.into(),
            bytes: Some(bytes.into()),
            drop: false,
        }
    }

    /// An edit that removes part `part` of the message at `ordinal`.
    pub fn drop(ordinal: u64, mid: impl Into<String>, part: impl Into<String>) -> Self {
        Self {
            ordinal,
            mid: mid.into(),
            part: part.into(),
            bytes: None,
            drop: true,
        }
    }
}

/// One range of a view, half-open over ordinals of the view's lineage:
/// `from` included, `to` excluded. Strict on `kind`: a runner cannot apply a
/// range it does not understand.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ViewRange {
    /// Send the transcript's messages in `[from, to)` as written, except for
    /// the parts `edits` changes. Never empty. `edits` is sorted by ordinal,
    /// then by the part's position in its message, with at most one edit per
    /// part; omitted when empty.
    Keep {
        from: u64,
        to: u64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        edits: Vec<PartEdit>,
    },
    /// Send `bytes` in place of the messages in `[from, to)`. `bytes` is a
    /// UTF-8 JSON array of messages in the runner's message schema, carried
    /// as a string so its exact bytes enter the digest. `from == to` inserts
    /// the messages before `to` and replaces nothing.
    Replace { from: u64, to: u64, bytes: String },
}

impl ViewRange {
    pub fn keep(from: u64, to: u64) -> Self {
        Self::Keep {
            from,
            to,
            edits: Vec::new(),
        }
    }

    /// A keep range whose messages change only in the parts `edits` names.
    pub fn keep_with_edits(from: u64, to: u64, edits: Vec<PartEdit>) -> Self {
        Self::Keep { from, to, edits }
    }

    pub fn replace(from: u64, to: u64, bytes: impl Into<String>) -> Self {
        Self::Replace {
            from,
            to,
            bytes: bytes.into(),
        }
    }

    pub fn from(&self) -> u64 {
        match self {
            Self::Keep { from, .. } | Self::Replace { from, .. } => *from,
        }
    }

    pub fn to(&self) -> u64 {
        match self {
            Self::Keep { to, .. } | Self::Replace { to, .. } => *to,
        }
    }
}

/// The point through which a view's replacements stand for the whole
/// transcript, where a runner may write its host's native compaction marker
/// once the view is applied (CONTRACT.md §9).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ViewCut {
    /// The provider's name for the cut, echoed in `marker_outcome`. 1 to 64
    /// bytes of `[A-Za-z0-9._-]`.
    pub cut_id: String,
    /// The last ordinal the cut covers.
    pub through_ordinal: u64,
}

impl ViewCut {
    pub fn new(cut_id: impl Into<String>, through_ordinal: u64) -> Self {
        Self {
            cut_id: cut_id.into(),
            through_ordinal,
        }
    }
}

/// Marks a provider may put in a view's `apply` member (CONTRACT.md §6,
/// §7). Open: a view carrying another mark still decodes, and a runner that
/// did not list that mark in Setup's `view_apply` refuses the view with
/// [`crate::notice::refusal_reasons::UNSUPPORTED_APPLY`].
pub mod apply_marks {
    /// Apply the view at the runner's next step boundary, with any companion
    /// edits the model provider requires there; defer it only when no
    /// companion edit makes the request acceptable at that step.
    pub const NEXT_STEP: &str = "next_step";
}

/// A view. Decoded leniently on fields; strict on each range's `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CompactionView {
    /// The session reference, `llm-runner/v1`'s `session_ref`.
    pub session_ref: String,
    /// The view's version within the session: 1 for the provider's first
    /// view, and higher for each later one.
    pub view_version: u64,
    /// The lineage whose ordinals the ranges use.
    pub lineage_id: String,
    /// The last message the view covers. The ranges tile `[first_ordinal,
    /// covers_through.ordinal + 1)`, where `first_ordinal` is the lineage's
    /// first ordinal (`llm-runner/v1`'s `session.head.first_ordinal`): 0
    /// before the session's first cut, the cut's first retained ordinal
    /// after it. Everything before `first_ordinal` is carried only by the
    /// view's head, a replace range inserted at `first_ordinal`.
    pub covers_through: MessageRef,
    pub ranges: Vec<ViewRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<ViewCut>,
    /// When the provider asks the runner to apply the view: one of
    /// [`apply_marks`], or a mark this crate does not know. Absent means
    /// unmarked: the runner chooses between deferring the view and applying
    /// it with companion edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply: Option<String>,
    /// The prompt cache's lifetime, resolved for the model of the notice the
    /// view answers (CONTRACT.md §8.1). Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_ttl: Option<crate::cache_ttl::CacheTtl>,
    /// [`CompactionView::compute_digest`], lowercase hexadecimal.
    pub digest: String,
}

/// Why a view is not acceptable, or why a runner does not apply it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ViewProblem {
    /// `session_ref` is not a session reference.
    SessionRef,
    /// `view_version` is zero.
    VersionZero,
    /// `ranges` is empty.
    NoRanges,
    /// A range does not start where the previous one ended. Where the first
    /// range starts is checked against the lineage instead
    /// ([`dispose_fetched`], [`crate::notice::refusal_reasons::RANGES_START`]).
    Gap,
    /// A range's `from` is above its `to`, or a keep range is empty.
    Inverted,
    /// The last range does not end at `covers_through.ordinal + 1`.
    CoverageEnd,
    /// A replace range's bytes are not a JSON array.
    ReplacementNotArray,
    /// The cut is malformed, beyond the coverage, not on a range boundary, or
    /// preceded by a keep range.
    Cut,
    /// A part edit's ordinal is outside its keep range.
    EditOutsideRange,
    /// A keep range's edits are not sorted by ordinal, or, against the
    /// transcript, not in the order of the parts within their message.
    EditOrder,
    /// Two edits name the same part of the same message.
    DuplicateEdit,
    /// A part edit's key is empty or longer than [`MAX_PART_KEY_BYTES`].
    EditPart,
    /// A part edit gives both `bytes` and `drop`, or neither.
    EditAction,
    /// A part edit's `bytes` are not JSON.
    EditNotJson,
    /// The view's encoding exceeds [`MAX_VIEW_BYTES`].
    TooLarge,
    /// `digest` is not lowercase hexadecimal SHA-256.
    DigestMalformed,
    /// `digest` does not match the view's content, or the digest the
    /// view-ready event announced.
    DigestMismatch,
    /// The fetched view names another session or version than the one asked
    /// for.
    NotRequested,
    /// The view's coverage is not a prefix of the runner's transcript: its
    /// lineage differs, or the message at `covers_through.ordinal` is gone or
    /// has another id.
    NotAPrefix,
}

impl ViewProblem {
    /// The problem's name as the vectors spell it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SessionRef => "session_ref",
            Self::VersionZero => "version_zero",
            Self::NoRanges => "no_ranges",
            Self::Gap => "gap",
            Self::Inverted => "inverted",
            Self::CoverageEnd => "coverage_end",
            Self::ReplacementNotArray => "replacement_not_array",
            Self::Cut => "cut",
            Self::EditOutsideRange => "edit_outside_range",
            Self::EditOrder => "edit_order",
            Self::DuplicateEdit => "duplicate_edit",
            Self::EditPart => "edit_part",
            Self::EditAction => "edit_action",
            Self::EditNotJson => "edit_not_json",
            Self::TooLarge => "too_large",
            Self::DigestMalformed => "digest_malformed",
            Self::DigestMismatch => "digest_mismatch",
            Self::NotRequested => "not_requested",
            Self::NotAPrefix => "not_a_prefix",
        }
    }
}

impl CompactionView {
    /// A view with its digest computed.
    pub fn new(
        session_ref: impl Into<String>,
        view_version: u64,
        lineage_id: impl Into<String>,
        covers_through: MessageRef,
        ranges: Vec<ViewRange>,
    ) -> Self {
        let mut view = Self {
            session_ref: session_ref.into(),
            view_version,
            lineage_id: lineage_id.into(),
            covers_through,
            ranges,
            cut: None,
            apply: None,
            cache_ttl: None,
            digest: String::new(),
        };
        view.digest = view.compute_digest();
        view
    }

    /// Set the cut and recompute the digest.
    pub fn with_cut(mut self, cut: ViewCut) -> Self {
        self.cut = Some(cut);
        self.digest = self.compute_digest();
        self
    }

    /// Set the `apply` mark (one of [`apply_marks`]) and recompute the
    /// digest.
    pub fn with_apply(mut self, apply: impl Into<String>) -> Self {
        self.apply = Some(apply.into());
        self.digest = self.compute_digest();
        self
    }

    /// Set the `cache_ttl` entry and recompute the digest.
    pub fn with_cache_ttl(mut self, entry: crate::cache_ttl::CacheTtl) -> Self {
        self.cache_ttl = Some(entry);
        self.digest = self.compute_digest();
        self
    }

    /// The view's digest. The input is a sequence of frames, each an 8-byte
    /// big-endian length followed by that many bytes: [`VIEW_DIGEST_DOMAIN`],
    /// `session_ref`, `view_version` in decimal, `lineage_id`,
    /// `covers_through.ordinal` in decimal, `covers_through.mid`, the number
    /// of ranges in decimal; then for each range in order its `kind`, `from`
    /// and `to` in decimal and, for `replace`, its `bytes`, or, for `keep`, the
    /// number of its edits in decimal followed, for each edit in order, by its
    /// `ordinal` in decimal, its `mid`, its `part`, and either `bytes` and the
    /// bytes or `drop`; then, only when a cut is present, `cut`, its `cut_id`
    /// and its `through_ordinal` in decimal; then, only when an `apply` mark
    /// is present, `apply` and the mark; then, only when `cache_ttl` is
    /// present, `cache_ttl`, its `model`, its `ms` in decimal or `null`, and
    /// `true` or `false` for `configured`. Lowercase hexadecimal SHA-256.
    /// `digest` itself is not an input. A view without a mark or a
    /// `cache_ttl` therefore has the digest it had before they existed.
    pub fn compute_digest(&self) -> String {
        let mut hasher = Sha256::new();
        frame(&mut hasher, VIEW_DIGEST_DOMAIN.as_bytes());
        frame(&mut hasher, self.session_ref.as_bytes());
        frame(&mut hasher, self.view_version.to_string().as_bytes());
        frame(&mut hasher, self.lineage_id.as_bytes());
        frame(
            &mut hasher,
            self.covers_through.ordinal.to_string().as_bytes(),
        );
        frame(&mut hasher, self.covers_through.mid.as_bytes());
        frame(&mut hasher, self.ranges.len().to_string().as_bytes());
        for range in &self.ranges {
            match range {
                ViewRange::Keep { from, to, edits } => {
                    frame(&mut hasher, b"keep");
                    frame(&mut hasher, from.to_string().as_bytes());
                    frame(&mut hasher, to.to_string().as_bytes());
                    frame(&mut hasher, edits.len().to_string().as_bytes());
                    for edit in edits {
                        frame(&mut hasher, edit.ordinal.to_string().as_bytes());
                        frame(&mut hasher, edit.mid.as_bytes());
                        frame(&mut hasher, edit.part.as_bytes());
                        match &edit.bytes {
                            Some(bytes) => {
                                frame(&mut hasher, b"bytes");
                                frame(&mut hasher, bytes.as_bytes());
                            }
                            None => frame(&mut hasher, b"drop"),
                        }
                    }
                }
                ViewRange::Replace { from, to, bytes } => {
                    frame(&mut hasher, b"replace");
                    frame(&mut hasher, from.to_string().as_bytes());
                    frame(&mut hasher, to.to_string().as_bytes());
                    frame(&mut hasher, bytes.as_bytes());
                }
            }
        }
        if let Some(cut) = &self.cut {
            frame(&mut hasher, b"cut");
            frame(&mut hasher, cut.cut_id.as_bytes());
            frame(&mut hasher, cut.through_ordinal.to_string().as_bytes());
        }
        if let Some(apply) = &self.apply {
            frame(&mut hasher, b"apply");
            frame(&mut hasher, apply.as_bytes());
        }
        if let Some(ttl) = &self.cache_ttl {
            frame(&mut hasher, b"cache_ttl");
            frame(&mut hasher, ttl.model.as_bytes());
            let ms = ttl
                .ms
                .map_or_else(|| "null".to_owned(), |ms| ms.to_string());
            frame(&mut hasher, ms.as_bytes());
            frame(&mut hasher, if ttl.configured { b"true" } else { b"false" });
        }
        finish(hasher)
    }

    /// Check the view on its own, in a fixed order: session reference,
    /// version, digest form, the ranges' tiling, each range's replacement or
    /// part edits, the cut, the size, and last the digest against the
    /// content. The tiling is checked from wherever the first range starts:
    /// whether that is the lineage's first ordinal needs the runner's
    /// lineage, so [`dispose_fetched`] checks it.
    pub fn check(&self) -> Result<(), ViewProblem> {
        if !is_session_ref(&self.session_ref) {
            return Err(ViewProblem::SessionRef);
        }
        if self.view_version == 0 {
            return Err(ViewProblem::VersionZero);
        }
        if !is_sha256_hex(&self.digest) {
            return Err(ViewProblem::DigestMalformed);
        }
        if self.ranges.is_empty() {
            return Err(ViewProblem::NoRanges);
        }
        let mut next = self.ranges[0].from();
        for range in &self.ranges {
            if range.from() != next {
                return Err(ViewProblem::Gap);
            }
            let empty_keep = matches!(range, ViewRange::Keep { from, to, .. } if from == to);
            if range.from() > range.to() || empty_keep {
                return Err(ViewProblem::Inverted);
            }
            match range {
                ViewRange::Replace { bytes, .. } => {
                    if serde_json::from_str::<Vec<IgnoredAny>>(bytes).is_err() {
                        return Err(ViewProblem::ReplacementNotArray);
                    }
                }
                ViewRange::Keep { from, to, edits } => check_edits(*from, *to, edits)?,
            }
            next = range.to();
        }
        let end = self.covers_through.ordinal.checked_add(1);
        if end != Some(next) {
            return Err(ViewProblem::CoverageEnd);
        }
        if let Some(cut) = &self.cut {
            self.check_cut(cut)?;
        }
        let size = serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len());
        if size > MAX_VIEW_BYTES {
            return Err(ViewProblem::TooLarge);
        }
        if self.compute_digest() != self.digest {
            return Err(ViewProblem::DigestMismatch);
        }
        Ok(())
    }

    /// A cut must fall on a range boundary, inside the coverage, with only
    /// replace ranges before it: the provider's bytes stand for every message
    /// through it.
    fn check_cut(&self, cut: &ViewCut) -> Result<(), ViewProblem> {
        let id_ok = !cut.cut_id.is_empty()
            && cut.cut_id.len() <= crate::notice::MAX_CUT_ID_BYTES
            && cut
                .cut_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        let boundary = cut.through_ordinal.checked_add(1);
        let on_boundary = self.ranges.iter().any(|r| Some(r.to()) == boundary);
        let only_replaced = self
            .ranges
            .iter()
            .take_while(|r| Some(r.to()) <= boundary)
            .all(|r| matches!(r, ViewRange::Replace { .. }));
        if id_ok
            && cut.through_ordinal <= self.covers_through.ordinal
            && on_boundary
            && only_replaced
        {
            Ok(())
        } else {
            Err(ViewProblem::Cut)
        }
    }

    /// Every part edit of every keep range, in order.
    pub fn edits(&self) -> impl Iterator<Item = &PartEdit> {
        self.ranges.iter().flat_map(|range| match range {
            ViewRange::Keep { edits, .. } => edits.as_slice(),
            ViewRange::Replace { .. } => &[],
        })
    }

    /// The ordinal the view's ranges start at: its first range's `from`, or
    /// `None` for a view without ranges.
    pub fn ranges_start(&self) -> Option<u64> {
        self.ranges.first().map(ViewRange::from)
    }

    /// Whether the view's coverage is still a prefix of the runner's
    /// transcript: same lineage; the message the runner holds at
    /// `covers_through.ordinal` has id `covers_through.mid`; and the message
    /// at each part edit's ordinal has that edit's `mid`. `mid_at(ordinal)`
    /// is the id of the message the runner holds at that ordinal. Ordinals
    /// are never reused within a lineage, so a revert that truncated below
    /// the coverage fails this check.
    pub fn check_prefix(
        &self,
        lineage_id: &str,
        mid_at: impl Fn(u64) -> Option<String>,
    ) -> Result<(), ViewProblem> {
        let holds = |ordinal: u64, mid: &str| mid_at(ordinal).as_deref() == Some(mid);
        let same_message = holds(self.covers_through.ordinal, &self.covers_through.mid);
        let edits_hold = self.edits().all(|edit| holds(edit.ordinal, &edit.mid));
        if self.lineage_id == lineage_id && same_message && edits_hold {
            Ok(())
        } else {
            Err(ViewProblem::NotAPrefix)
        }
    }

    /// Check every part edit against the part keys of the message it edits,
    /// `parts_at(ordinal)`, in the message's order. A key the message does
    /// not have is `Err(None)`, which the runner reports as
    /// `view_refused:<version>:unknown_part`. Edits of one message out of its
    /// part order are `Err(Some(ViewProblem::EditOrder))`. Call it on a view
    /// that passed [`CompactionView::check_prefix`].
    pub fn check_parts(
        &self,
        parts_at: impl Fn(u64) -> Option<Vec<String>>,
    ) -> Result<(), Option<ViewProblem>> {
        let mut previous: Option<(u64, usize)> = None;
        for edit in self.edits() {
            let parts = parts_at(edit.ordinal).unwrap_or_default();
            let position = parts
                .iter()
                .position(|part| *part == edit.part)
                .ok_or(None)?;
            if previous.is_some_and(|(ordinal, at)| ordinal == edit.ordinal && at >= position) {
                return Err(Some(ViewProblem::EditOrder));
            }
            previous = Some((edit.ordinal, position));
        }
        Ok(())
    }

    /// The history a runner sends under this view, before the runner's own
    /// provider rules (CONTRACT.md §7): for each range in order, the
    /// transcript's messages with ordinals inside a keep range, with their
    /// part edits applied, or a replace range's messages; then every
    /// transcript message after the coverage. Replacement and edit bytes go
    /// in exactly as given. A reference for tests and vectors, over a model
    /// of the runner's schema in which a message's parts are the elements of
    /// its `content` array, keyed in order by [`TranscriptMessage::parts`].
    /// Call it on a view that passed every other check.
    pub fn assemble(&self, transcript: &[TranscriptMessage]) -> Result<Vec<Value>, ViewProblem> {
        let mut out = Vec::new();
        for range in &self.ranges {
            match range {
                ViewRange::Keep { from, to, edits } => {
                    for entry in transcript
                        .iter()
                        .filter(|entry| entry.ordinal >= *from && entry.ordinal < *to)
                    {
                        out.push(entry.edited(edits)?);
                    }
                }
                ViewRange::Replace { bytes, .. } => {
                    let messages: Vec<Value> = serde_json::from_str(bytes)
                        .map_err(|_| ViewProblem::ReplacementNotArray)?;
                    out.extend(messages);
                }
            }
        }
        out.extend(
            transcript
                .iter()
                .filter(|entry| entry.ordinal > self.covers_through.ordinal)
                .map(|entry| entry.message.clone()),
        );
        Ok(out)
    }
}

/// Check one keep range's edits on their own: each inside the range, sorted
/// by ordinal, one per part, with a valid key, exactly one action, and JSON
/// bytes.
fn check_edits(from: u64, to: u64, edits: &[PartEdit]) -> Result<(), ViewProblem> {
    let mut seen = std::collections::BTreeSet::new();
    let mut previous = None;
    for edit in edits {
        if edit.ordinal < from || edit.ordinal >= to {
            return Err(ViewProblem::EditOutsideRange);
        }
        if previous.is_some_and(|ordinal| edit.ordinal < ordinal) {
            return Err(ViewProblem::EditOrder);
        }
        previous = Some(edit.ordinal);
        if !seen.insert((edit.ordinal, edit.part.as_str())) {
            return Err(ViewProblem::DuplicateEdit);
        }
        if edit.part.is_empty() || edit.part.len() > MAX_PART_KEY_BYTES {
            return Err(ViewProblem::EditPart);
        }
        match (&edit.bytes, edit.drop) {
            (Some(bytes), false) => {
                if serde_json::from_str::<IgnoredAny>(bytes).is_err() {
                    return Err(ViewProblem::EditNotJson);
                }
            }
            (None, true) => {}
            _ => return Err(ViewProblem::EditAction),
        }
    }
    Ok(())
}

impl CompactionView {
    /// The pruned part this view would put pruned content back into, if any:
    /// the check behind `view_refused:<version>:pruned_reasoning_restored`
    /// (`llm-runner/v1` §9.14.1, CONTRACT.md §7). It covers every edit form a
    /// view has: each replace range's `bytes`, checked against the ordinals
    /// the range stands for, and each keep-range part edit's `bytes`, checked
    /// against its message. A `drop` edit sends nothing and restores nothing.
    ///
    /// `pruned` is the runner's record of the parts it pruned, in the order
    /// it wants them reported. `first_ordinal` is the lineage's first
    /// ordinal: the first range of a lineage starting above 0 is the view's
    /// head, so it also stands for every ordinal below it. Bytes that are not
    /// JSON are skipped here; [`CompactionView::check`] refuses them.
    pub fn restored_pruned_part<'a>(
        &self,
        pruned: &'a [PrunedPartAt],
        first_ordinal: u64,
    ) -> Option<&'a PrunedPartAt> {
        self.ranges
            .iter()
            .enumerate()
            .find_map(|(index, range)| match range {
                ViewRange::Replace { from, to, bytes } => {
                    let start = if index == 0 && first_ordinal > 0 {
                        0
                    } else {
                        *from
                    };
                    let messages: Value = serde_json::from_str(bytes).ok()?;
                    restored_part(pruned, start..*to, &messages)
                }
                ViewRange::Keep { edits, .. } => edits.iter().find_map(|edit| {
                    let part: Value = serde_json::from_str(edit.bytes.as_deref()?).ok()?;
                    restored_by_edit(pruned, edit.ordinal, &edit.part, &part)
                }),
            })
    }
}

/// One message of the runner's transcript as the runner's checks and
/// [`CompactionView::assemble`] see it: its place, id, part keys in order,
/// and body.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct TranscriptMessage {
    pub ordinal: u64,
    pub mid: String,
    /// The message's part keys, in the order its parts appear.
    pub parts: Vec<String>,
    pub message: Value,
}

impl TranscriptMessage {
    pub fn new(ordinal: u64, mid: impl Into<String>, parts: Vec<String>, message: Value) -> Self {
        Self {
            ordinal,
            mid: mid.into(),
            parts,
            message,
        }
    }

    /// The message with the `edits` that name it applied: a replaced part
    /// becomes its parsed bytes, a dropped part is removed, and every other
    /// part stays as written.
    fn edited(&self, edits: &[PartEdit]) -> Result<Value, ViewProblem> {
        let mine: Vec<&PartEdit> = edits
            .iter()
            .filter(|edit| edit.ordinal == self.ordinal)
            .collect();
        if mine.is_empty() {
            return Ok(self.message.clone());
        }
        let content = self.message["content"]
            .as_array()
            .filter(|content| content.len() == self.parts.len())
            .ok_or(ViewProblem::EditPart)?;
        let mut parts = Vec::new();
        for (key, part) in self.parts.iter().zip(content) {
            match mine.iter().find(|edit| edit.part == *key) {
                None => parts.push(part.clone()),
                Some(edit) if edit.drop => {}
                Some(edit) => {
                    let bytes = edit.bytes.as_deref().unwrap_or_default();
                    parts.push(serde_json::from_str(bytes).map_err(|_| ViewProblem::EditNotJson)?);
                }
            }
        }
        let mut message = self.message.clone();
        message["content"] = Value::Array(parts);
        Ok(message)
    }
}

/// What a runner does with a view it fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewDisposition {
    /// Write it durably as the session's next view and render it from the
    /// next step boundary on, that is from the first model request of the
    /// next step, never partway through a step; from then every notice names it in
    /// `applied_view_version`. Under the current model provider's rules the
    /// runner may still defer it (`view_deferred`), apply it with companion
    /// edits (`view_companion`) or refuse it (`view_refused`), CONTRACT.md
    /// §7.
    Apply,
    /// Its version is not above the current view's: ignore it.
    Stale,
    /// It is malformed, does not match what was asked for, or fails its
    /// digest: do not persist or apply it, keep the current view, and report
    /// nothing. The runner may fetch it again.
    Rejected(ViewProblem),
    /// Its coverage is no longer a prefix of the transcript: drop it and
    /// report `view_discarded:<version>` with `revert_below` on the next
    /// notice.
    Discard,
    /// It edits a part key the message does not have, or its ranges do not
    /// start at the lineage's first ordinal: refuse it, report
    /// `view_refused:<version>:<reason>` with this reason (one of
    /// [`crate::notice::refusal_reasons`]) on the next notice, and keep the
    /// current view.
    Refuse(&'static str),
}

impl ViewDisposition {
    /// The disposition's name as the vectors spell it: `apply`, `stale`,
    /// `discard`, or `rejected:<problem>`.
    pub fn name(&self) -> String {
        match self {
            Self::Apply => "apply".into(),
            Self::Stale => "stale".into(),
            Self::Rejected(problem) => format!("rejected:{}", problem.name()),
            Self::Discard => "discard".into(),
            Self::Refuse(reason) => format!("refused:{reason}"),
        }
    }
}

/// The runner's checks of a view fetched for `ready`, in a fixed order: it
/// is the view asked for; it is well formed with a matching digest
/// ([`CompactionView::check`]); its digest is the one `ready` announced; its
/// version is above `current_version`, the version of the view the runner
/// currently applies (`None` before any); its coverage and part edits are a
/// prefix of the transcript ([`CompactionView::check_prefix`]); its ranges
/// start at `first_ordinal`, the current lineage's first ordinal, or it is
/// refused [`crate::notice::refusal_reasons::RANGES_START`]; every edited
/// part key exists, in its message's part order
/// ([`CompactionView::check_parts`]). The first that fails decides.
/// `message_at(ordinal)` is the message the runner holds at that ordinal of
/// `lineage_id`.
pub fn dispose_fetched(
    view: &CompactionView,
    ready: &ViewReady,
    current_version: Option<u64>,
    lineage_id: &str,
    first_ordinal: u64,
    message_at: impl Fn(u64) -> Option<TranscriptMessage>,
) -> ViewDisposition {
    if view.session_ref != ready.session_ref || view.view_version != ready.view_version {
        return ViewDisposition::Rejected(ViewProblem::NotRequested);
    }
    if let Err(problem) = view.check() {
        return ViewDisposition::Rejected(problem);
    }
    if view.digest != ready.digest {
        return ViewDisposition::Rejected(ViewProblem::DigestMismatch);
    }
    if current_version.is_some_and(|current| view.view_version <= current) {
        return ViewDisposition::Stale;
    }
    if view
        .check_prefix(lineage_id, |ordinal| message_at(ordinal).map(|m| m.mid))
        .is_err()
    {
        return ViewDisposition::Discard;
    }
    // The ranges tile from the lineage's first ordinal, and on a lineage a
    // cut started the first range is a replace carrying the history head.
    let head_first =
        first_ordinal == 0 || matches!(view.ranges.first(), Some(ViewRange::Replace { .. }));
    if view.ranges_start() != Some(first_ordinal) || !head_first {
        return ViewDisposition::Refuse(crate::notice::refusal_reasons::RANGES_START);
    }
    match view.check_parts(|ordinal| message_at(ordinal).map(|m| m.parts)) {
        Ok(()) => ViewDisposition::Apply,
        Err(None) => ViewDisposition::Refuse(crate::notice::refusal_reasons::UNKNOWN_PART),
        Err(Some(problem)) => ViewDisposition::Rejected(problem),
    }
}

/// The `view.get` request: fetch one published view. Decoded leniently.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ViewGetRequest {
    pub session_ref: String,
    pub view_version: u64,
}

impl ViewGetRequest {
    pub fn new(session_ref: impl Into<String>, view_version: u64) -> Self {
        Self {
            session_ref: session_ref.into(),
            view_version,
        }
    }

    /// The request that fetches the view a [`ViewReady`] event announced.
    pub fn for_ready(ready: &ViewReady) -> Self {
        Self::new(ready.session_ref.clone(), ready.view_version)
    }
}

/// Refusal codes of `view.get`. Open strings: a runner that meets another
/// code treats it like `unknown_version`.
pub mod view_get_codes {
    /// The provider holds no view state for this session reference.
    pub const UNKNOWN_SESSION: &str = "unknown_session";
    /// The provider does not hold this version (never published, or dropped
    /// after a newer one). The provider never answers another version.
    pub const UNKNOWN_VERSION: &str = "unknown_version";
}

/// The `view.get` answer. Strict on `answer`; lenient on fields.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ViewGetAnswer {
    /// The view, byte for byte as first published.
    View { view: CompactionView },
    /// No such view. `code` is one of [`view_get_codes`] or a code this crate
    /// does not know.
    Refuse { code: String, reason: String },
}

impl ViewGetAnswer {
    pub fn view(view: CompactionView) -> Self {
        Self::View { view }
    }

    pub fn refuse(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Refuse {
            code: code.into(),
            reason: reason.into(),
        }
    }
}
