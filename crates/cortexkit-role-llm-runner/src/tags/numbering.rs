//! Computing tag numbers: which parts count, and the number each counted
//! part shows.
//!
//! A tag number is only a name the model can see, computed from the
//! transcript: the checkpoint (the counted parts before the cut) plus the
//! part's position among the counted parts the runner renders from the
//! cut, in rendered order. Nothing durable is keyed by it; a runner keeps no
//! per-part ledger. [`counts`] is the one predicate that decides both
//! counting and rendering, so the two cannot drift apart.

use serde::{Deserialize, Serialize};

/// What a part is, as far as tag numbering cares. A runner maps each part it
/// renders onto one of these; [`counts`] then says whether it gets a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PartKind {
    /// Text in a user message: the person's, the owner's, a sender's, a wake,
    /// and a steer or reminder the runner stores as a user message.
    /// Counted.
    UserText,
    /// A whole tool result with non-empty exposed text, error text included.
    /// Counted once, on its first non-empty text block; later text and media
    /// blocks carry no number. See [`super::tool_result_kind`].
    ToolResultText,
    /// A tool result with images but no non-empty exposed text or other media.
    /// Counted once, and rendered as a short
    /// text label carrying the number ([`super::image_label`]), so a context
    /// manager can address a screenshot.
    ToolResultImageOnly,
    /// A tool result with no non-empty exposed text and at least one non-image
    /// media block. Counted once, with [`super::media_label`].
    ToolResultMediaOnly,
    /// The model's own text. Counted, on every provider (a Gemini thought
    /// signature or a ChatGPT `phase` beside it makes no difference).
    AssistantText,
    /// A text part whose exposed text is empty (after cleaning, for the
    /// model's text), or a tool result with neither non-empty exposed text nor
    /// media. A structured value alone is not text. Nothing carries a number,
    /// so it is not counted.
    EmptyText,
    /// A `turn_system` message (a template rendered at system authority).
    /// Not counted.
    TurnSystem,
    /// An image or a file in a user message. Not counted.
    UserAttachment,
    /// A reasoning or thinking block, signed or encrypted or neither. Not
    /// counted.
    Reasoning,
    /// A tool call. Not counted: its result carries the number.
    ToolCall,
    /// The history head: a compaction provider's summary of what came
    /// before the cut. Not counted.
    HistoryHead,
    /// A compaction-marker message. Not counted.
    CompactionMarker,
}

/// Whether a part of `kind` counts: it gets a number exactly when it is
/// rendered with a visible one. User text, a tool result with text or media,
/// and the model's text count; nothing else does.
pub fn counts(kind: PartKind) -> bool {
    matches!(
        kind,
        PartKind::UserText
            | PartKind::ToolResultText
            | PartKind::ToolResultImageOnly
            | PartKind::ToolResultMediaOnly
            | PartKind::AssistantText
    )
}

/// One part from the cut on, as numbering sees it: its kind and its place
/// in the history as the runner renders it. Storage order plays no part:
/// a runner that stores a part after one it renders later (a steer stored
/// while a tool call was running, rendered after that call's result, say)
/// numbers by `rendered_at`.
///
/// Non-exhaustive: build one with [`RenderedPart::new`], or decode one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RenderedPart {
    pub kind: PartKind,
    /// The part's position in the rendered history from the cut, counting
    /// every rendered part, counted or not. Distinct per part; only the
    /// order matters.
    pub rendered_at: u64,
}

impl RenderedPart {
    pub fn new(kind: PartKind, rendered_at: u64) -> Self {
        Self { kind, rendered_at }
    }
}

/// The number of every part, in the order `parts` is given (storage order,
/// typically): `checkpoint` plus the part's position among the counted
/// parts in rendered order, the first counted part rendered after the cut
/// being `checkpoint + 1`; `None` for a part that does not count. Two parts
/// at the same `rendered_at` keep the order they are given in.
///
/// A part a compaction provider dropped behind its placeholder keeps its
/// number: it is still rendered, as `[dropped §N§]`, so it is still counted.
/// The only gaps are behind the cut, where the checkpoint may count parts no
/// longer rendered, or parts an earlier tagger numbered.
///
/// Refused with [`NumberingOverflow`] when the last number would pass
/// `u64::MAX`: a runner refuses such a checkpoint rather than give two parts
/// the same number. A numbering that ends exactly at `u64::MAX` is accepted.
pub fn try_number_parts(
    checkpoint: u64,
    parts: &[RenderedPart],
) -> Result<Vec<Option<u64>>, NumberingOverflow> {
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by_key(|index| parts[*index].rendered_at);
    let mut numbers = vec![None; parts.len()];
    let mut next = checkpoint;
    for index in order {
        if counts(parts[index].kind) {
            next = next.checked_add(1).ok_or_else(|| NumberingOverflow {
                checkpoint,
                counted: parts.iter().filter(|part| counts(part.kind)).count() as u64,
            })?;
            numbers[index] = Some(next);
        }
    }
    Ok(numbers)
}

/// Numbering would pass `u64::MAX`: `checkpoint` plus `counted` counted
/// parts does not fit. A runner refuses the checkpoint instead of repeating
/// a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NumberingOverflow {
    /// The checkpoint numbering started from.
    pub checkpoint: u64,
    /// How many of the parts count, every one of which needs a number.
    pub counted: u64,
}

impl std::fmt::Display for NumberingOverflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tag numbering from checkpoint {} over {} counted parts would pass u64::MAX",
            self.checkpoint, self.counted
        )
    }
}

impl std::error::Error for NumberingOverflow {}

/// Like [`try_number_parts`], except that near `u64::MAX` it saturates:
/// every counted part past the limit gets `u64::MAX`, so two parts can show
/// the same number.
#[deprecated(
    since = "0.4.1",
    note = "saturates at u64::MAX, so two parts can get one number; use try_number_parts, which refuses the overflow"
)]
pub fn number_parts(checkpoint: u64, parts: &[RenderedPart]) -> Vec<Option<u64>> {
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by_key(|index| parts[*index].rendered_at);
    let mut numbers = vec![None; parts.len()];
    let mut next = checkpoint;
    for index in order {
        if counts(parts[index].kind) {
            next = next.saturating_add(1);
            numbers[index] = Some(next);
        }
    }
    numbers
}

/// The checkpoint a runner writes on a compaction marker, in the same write
/// as the marker and never sent to the model: the counted parts before the
/// marker's cut, and the session it was written for. A key naming another
/// session was copied by a fork, and names the fork's parent.
///
/// On OpenCode it is the compaction part's `"cortexkit"` key. A marker that
/// lost its key (OpenCode's part-update route drops unknown keys) is no
/// anchor; it never reads as a checkpoint of zero.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct MarkerCheckpoint {
    pub tag_checkpoint: u64,
    pub session: String,
}

impl MarkerCheckpoint {
    pub fn new(tag_checkpoint: u64, session: impl Into<String>) -> Self {
        Self {
            tag_checkpoint,
            session: session.into(),
        }
    }
}

/// A compaction marker as anchoring sees it, whoever wrote it (the host's
/// own compaction, standalone Magic Context, or the runner).
///
/// Non-exhaustive: build one with [`Marker::new`] and the `with_*` setters,
/// or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Marker {
    /// The message holding the marker's compaction part.
    pub message: String,
    /// The first retained message, when the marker names one. Native
    /// markers that retain nothing omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_start_id: Option<String>,
    /// Whether the marker is in effect: a reverted or superseded marker is
    /// never an anchor.
    pub in_effect: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<MarkerCheckpoint>,
}

impl Marker {
    pub fn new(message: impl Into<String>, in_effect: bool) -> Self {
        Self {
            message: message.into(),
            tail_start_id: None,
            in_effect,
            checkpoint: None,
        }
    }

    pub fn with_tail_start_id(mut self, tail_start_id: impl Into<String>) -> Self {
        self.tail_start_id = Some(tail_start_id.into());
        self
    }

    pub fn with_checkpoint(mut self, checkpoint: MarkerCheckpoint) -> Self {
        self.checkpoint = Some(checkpoint);
        self
    }

    /// The message the marker cuts at, keyed on the field and not on who
    /// wrote the marker: `tail_start_id` when present, otherwise the message
    /// holding the compaction part.
    pub fn cut(&self) -> &str {
        self.tail_start_id.as_deref().unwrap_or(&self.message)
    }
}

/// A checkpoint a compaction provider puts on its Setup answer for a
/// session it moved from standalone Magic Context: `{tag_checkpoint, cut}`,
/// `cut` the message the standalone marker cuts at. Permanent: it rides
/// every `ready` Setup answer for that session and never
/// changes.
///
/// Non-exhaustive: build one with [`SetupCheckpoint::new`], or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SetupCheckpoint {
    pub tag_checkpoint: u64,
    pub cut: String,
}

impl SetupCheckpoint {
    pub fn new(tag_checkpoint: u64, cut: impl Into<String>) -> Self {
        Self {
            tag_checkpoint,
            cut: cut.into(),
        }
    }
}

/// Where numbering starts: a checkpoint and the message it is anchored at.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Anchor {
    /// An in-effect marker carrying a checkpoint for this session.
    Marker { checkpoint: u64, cut: String },
    /// The Setup checkpoint, at its own cut message.
    Setup { checkpoint: u64, cut: String },
    /// The start of the session. `new_lineage` is true when this anchor
    /// replaces a Setup checkpoint whose cut message no longer exists (a
    /// revert went past it): the numbers change, so the runner moves the
    /// session to a new lineage.
    SessionStart { checkpoint: u64, new_lineage: bool },
}

/// The anchor numbering starts from, in the fallback order: the newest
/// in-effect marker carrying a checkpoint for `session`, then earlier
/// in-effect markers carrying one, then the Setup checkpoint at its cut
/// when `message_exists` says that message still exists, then the session
/// start, with the Setup checkpoint's number (on a new lineage) if there
/// was one, else zero. `markers` is newest first.
///
/// `Err` names the parent session when any in-effect marker's checkpoint
/// was written for another session: the session is a fork or a clone of a
/// managed session, which a runner refuses `fork_unsupported`.
pub fn anchor(
    session: &str,
    markers: &[Marker],
    setup: Option<&SetupCheckpoint>,
    message_exists: impl Fn(&str) -> bool,
) -> Result<Anchor, String> {
    let keyed = markers
        .iter()
        .filter(|marker| marker.in_effect)
        .filter_map(|marker| Some((marker, marker.checkpoint.as_ref()?)));
    if let Some((_, foreign)) = keyed
        .clone()
        .find(|(_, checkpoint)| checkpoint.session != session)
    {
        return Err(foreign.session.clone());
    }
    if let Some((marker, checkpoint)) = keyed.into_iter().next() {
        return Ok(Anchor::Marker {
            checkpoint: checkpoint.tag_checkpoint,
            cut: marker.cut().to_owned(),
        });
    }
    Ok(match setup {
        Some(setup) if message_exists(&setup.cut) => Anchor::Setup {
            checkpoint: setup.tag_checkpoint,
            cut: setup.cut.clone(),
        },
        Some(setup) => Anchor::SessionStart {
            checkpoint: setup.tag_checkpoint,
            new_lineage: true,
        },
        None => Anchor::SessionStart {
            checkpoint: 0,
            new_lineage: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_parts_rendered_with_a_number_count() {
        use PartKind::*;
        for kind in [UserText, ToolResultText, ToolResultImageOnly, AssistantText] {
            assert!(counts(kind), "{kind:?}");
        }
        for kind in [
            EmptyText,
            TurnSystem,
            UserAttachment,
            Reasoning,
            ToolCall,
            HistoryHead,
            CompactionMarker,
        ] {
            assert!(!counts(kind), "{kind:?}");
        }
    }

    #[test]
    fn a_marker_without_its_key_is_no_anchor_and_never_zero() {
        let markers = [
            Marker::new("m9", true).with_tail_start_id("m7"),
            Marker::new("m4", true).with_checkpoint(MarkerCheckpoint::new(12, "s")),
        ];
        assert_eq!(
            anchor("s", &markers, None, |_| true),
            Ok(Anchor::Marker {
                checkpoint: 12,
                cut: "m4".into()
            })
        );
    }
}
