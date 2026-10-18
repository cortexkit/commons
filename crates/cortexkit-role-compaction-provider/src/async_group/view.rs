//! Views and `view.get` (CONTRACT.md §20.3, §20.4).
//!
//! A view is the provider's preferred shape for the prefix of a runner's
//! transcript through one message: a list of ranges over the runner's
//! immutable transcript, each kept as written or replaced with bytes the
//! provider supplies. Messages after the view's coverage go out unchanged. A
//! view never replaces or rewrites the runner's transcript or write-ahead
//! log; the runner keeps it beside them.

use serde::{de::IgnoredAny, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{finish, frame, is_sha256_hex, notice::is_session_ref, notice::ViewReady};
use crate::status::MessageRef;

/// Upper bound on a view's compact JSON encoding, in bytes. It leaves room
/// for the reply envelope under the 4 MiB transport frame.
pub const MAX_VIEW_BYTES: usize = 3_145_728;

/// The domain string that starts a view's digest input.
pub const VIEW_DIGEST_DOMAIN: &str = "ck.compaction.view.v1";

/// One range of a view, half-open over ordinals of the view's lineage:
/// `from` included, `to` excluded. Strict on `kind`: a runner cannot apply a
/// range it does not understand.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ViewRange {
    /// Send the transcript's messages in `[from, to)` as written. Never
    /// empty.
    Keep { from: u64, to: u64 },
    /// Send `bytes` in place of the messages in `[from, to)`. `bytes` is a
    /// UTF-8 JSON array of messages in the runner's message schema, carried
    /// as a string so its exact bytes enter the digest. `from == to` inserts
    /// the messages before `to` and replaces nothing.
    Replace { from: u64, to: u64, bytes: String },
}

impl ViewRange {
    pub fn keep(from: u64, to: u64) -> Self {
        Self::Keep { from, to }
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
/// once the view is applied (CONTRACT.md §20.6).
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

/// A view. Decoded leniently on fields; strict on each range's `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CompactionView {
    /// The session reference, `llm-runner/v1`'s `session_ref`.
    pub session_ref: String,
    /// Raised by the provider for every view of the session, from 1.
    pub view_version: u64,
    /// The lineage whose ordinals the ranges use.
    pub lineage_id: String,
    /// The last message the view covers. The ranges tile `[0,
    /// covers_through.ordinal + 1)`.
    pub covers_through: MessageRef,
    pub ranges: Vec<ViewRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<ViewCut>,
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
    /// The first range does not start at 0, or a range does not start where
    /// the previous one ended.
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

    /// The view's digest. The input is a sequence of frames, each an 8-byte
    /// big-endian length followed by that many bytes: [`VIEW_DIGEST_DOMAIN`],
    /// `session_ref`, `view_version` in decimal, `lineage_id`,
    /// `covers_through.ordinal` in decimal, `covers_through.mid`, the number
    /// of ranges in decimal; then for each range in order its `kind`, `from`
    /// and `to` in decimal and, for `replace`, its `bytes`; then, only when a
    /// cut is present, `cut`, its `cut_id` and its `through_ordinal` in
    /// decimal. Lowercase hexadecimal SHA-256. `digest` itself is not an
    /// input.
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
                ViewRange::Keep { from, to } => {
                    frame(&mut hasher, b"keep");
                    frame(&mut hasher, from.to_string().as_bytes());
                    frame(&mut hasher, to.to_string().as_bytes());
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
        finish(hasher)
    }

    /// Check the view on its own, in a fixed order: session reference,
    /// version, digest form, the ranges' tiling, the replacements, the cut,
    /// the size, and last the digest against the content.
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
        let mut next = 0u64;
        for range in &self.ranges {
            if range.from() != next {
                return Err(ViewProblem::Gap);
            }
            let empty_keep = matches!(range, ViewRange::Keep { from, to } if from == to);
            if range.from() > range.to() || empty_keep {
                return Err(ViewProblem::Inverted);
            }
            if let ViewRange::Replace { bytes, .. } = range {
                if serde_json::from_str::<Vec<IgnoredAny>>(bytes).is_err() {
                    return Err(ViewProblem::ReplacementNotArray);
                }
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
            && cut.cut_id.len() <= super::notice::MAX_CUT_ID_BYTES
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

    /// Whether the view's coverage is still a prefix of the runner's
    /// transcript: same lineage, and `mid_at(covers_through.ordinal)`, the
    /// id of the message the runner holds at that ordinal, equals
    /// `covers_through.mid`. Ordinals are never reused within a lineage, so a
    /// revert that truncated below the coverage fails this check.
    pub fn check_prefix(
        &self,
        lineage_id: &str,
        mid_at: impl Fn(u64) -> Option<String>,
    ) -> Result<(), ViewProblem> {
        let same_message = mid_at(self.covers_through.ordinal).as_deref()
            == Some(self.covers_through.mid.as_str());
        if self.lineage_id == lineage_id && same_message {
            Ok(())
        } else {
            Err(ViewProblem::NotAPrefix)
        }
    }

    /// The history a runner sends under this view, before the runner's own
    /// model rules (CONTRACT.md §20.4): for each range in order, the
    /// transcript's messages with ordinals inside a keep range, or a replace
    /// range's messages; then every transcript message after the coverage.
    /// `transcript` is `(ordinal, message)` in ordinal order. A reference
    /// for tests and vectors; call it on a view that passed
    /// [`CompactionView::check`].
    pub fn assemble(&self, transcript: &[(u64, Value)]) -> Result<Vec<Value>, ViewProblem> {
        let mut out = Vec::new();
        for range in &self.ranges {
            match range {
                ViewRange::Keep { from, to } => out.extend(
                    transcript
                        .iter()
                        .filter(|(ordinal, _)| ordinal >= from && ordinal < to)
                        .map(|(_, message)| message.clone()),
                ),
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
                .filter(|(ordinal, _)| *ordinal > self.covers_through.ordinal)
                .map(|(_, message)| message.clone()),
        );
        Ok(out)
    }
}

/// What a runner does with a view it fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewDisposition {
    /// Write it durably as the session's next view and render it from the
    /// next step boundary on; from then every notice names it in
    /// `applied_view_version`. The runner may still refuse it under its own
    /// structural and model rules (`view_refused`, CONTRACT.md §20.4).
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
        }
    }
}

/// The runner's checks of a view fetched for `ready`, in a fixed order: it
/// is the view asked for; it is well formed with a matching digest
/// ([`CompactionView::check`]); its digest is the one `ready` announced; its
/// version is above `current_version`, the version of the view the runner
/// currently applies (`None` before any); its coverage is a prefix of the
/// transcript ([`CompactionView::check_prefix`]). The first that fails
/// decides.
pub fn dispose_fetched(
    view: &CompactionView,
    ready: &ViewReady,
    current_version: Option<u64>,
    lineage_id: &str,
    mid_at: impl Fn(u64) -> Option<String>,
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
    match view.check_prefix(lineage_id, mid_at) {
        Ok(()) => ViewDisposition::Apply,
        Err(_) => ViewDisposition::Discard,
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

    /// The request for the view `ready` announced.
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
