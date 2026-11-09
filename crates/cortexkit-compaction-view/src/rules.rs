//! Checking a view against the provider rules a runner declared, and the
//! rules a runner declares for a model unless it knows better. The runner
//! refuses a view that breaks a rule by name. A view may explicitly authorize
//! stripping invalidated signatures that arrived after it was built.

use std::collections::BTreeSet;

use cortexkit_role_compaction::part::Seal;
use cortexkit_role_compaction::read::Entry;
use cortexkit_role_compaction::rules::{EditCost, ProviderRules, RuleBreak, SignedParts};
use cortexkit_role_compaction::view::{Edit, EditOp, InsertAt, View};

/// Claude models whose signed thinking Anthropic binds to the request
/// prefix, as family, major and minor. The source is Anthropic's
/// preserved-thinking page ("Keeping the prefix unchanged"), which applies the
/// check to "Claude 5.1 and later models"; add a row for each new model of
/// that generation. Accounts created on or after 2026-08-31 enforce the
/// binding on the Claude API, Vertex and Bedrock alike, so the route does not
/// matter. Mythos 5.1, Fable 5.2 and Mythos 5.2 enforce it on every account.
pub const BOUND_SIGNED_MODELS: &[(&str, u32, u32)] = &[
    ("fable", 5, 1),
    ("fable", 5, 2),
    ("mythos", 5, 1),
    ("mythos", 5, 2),
    ("opus", 5, 5),
    ("sonnet", 5, 5),
    ("haiku", 5, 5),
];

/// The rules a runner declares for a model unless it knows better: signed
/// parts are bound for [`BOUND_SIGNED_MODELS`], and an edit costs a re-read
/// from the changed byte. A runner overrides `history_edit_cost` with
/// `full_resend` on a route it knows chains its requests, and may override
/// `signed_parts` for an account it knows is not bound.
pub fn defaults(model: &str) -> ProviderRules {
    let signed_parts = if is_bound_signed_model(model) {
        SignedParts::Bound
    } else {
        SignedParts::None
    };
    ProviderRules::new(signed_parts, EditCost::Suffix)
}

/// Whether a model id names one of [`BOUND_SIGNED_MODELS`] on any route:
/// the family, an optional `-`, `_` or `.`, the major, one of them, the
/// minor, bounded on both sides so `sonnet-5-50` or `notsonnet-5-5` never
/// match. The trailing boundary also accepts Vertex `@date` and Bedrock `:0`
/// suffixes. Case does not matter.
pub fn is_bound_signed_model(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    let bytes = model.as_bytes();
    let separator = |b: u8| matches!(b, b'-' | b'_' | b'.');
    BOUND_SIGNED_MODELS.iter().any(|(family, major, minor)| {
        let (major, minor) = (major.to_string(), minor.to_string());
        model.match_indices(family).any(|(start, _)| {
            if start > 0 && !matches!(bytes[start - 1], b'-' | b'_' | b'.' | b':' | b'/') {
                return false;
            }
            let mut at = start + family.len();
            if bytes.get(at).copied().is_some_and(separator) {
                at += 1;
            }
            if !model[at..].starts_with(&major) {
                return false;
            }
            at += major.len();
            if !bytes.get(at).copied().is_some_and(separator) {
                return false;
            }
            at += 1;
            if !model[at..].starts_with(&minor) {
                return false;
            }
            at += minor.len();
            bytes
                .get(at)
                .is_none_or(|&b| matches!(b, b'-' | b'_' | b'.' | b':' | b'/' | b'@'))
        })
    })
}

/// The runner's signed floor (compaction contract W3.5): the message it goes
/// through. After a tool-set change on a route whose model binds signed
/// reasoning to the request prefix, the runner strips every signed part at or
/// before that message from each request. Those parts are never sent, so the
/// checks treat them as an already-removed oldest run: never invalid, and
/// never the earlier side of a gap between kept signed parts (contract rule
/// PR2). Dropping one is not an edit to later parts. Edits before the floor
/// still invalidate the signed parts after it. A floor that names no message
/// in the entries covers nothing, which keeps the checks strict, never looser.
///
/// The floor decides validity only: covered parts stay in render plans, and
/// removing them from the request is the runner's strip. It enters no
/// checksum. Every check takes one, so a caller can't give it to one check
/// and forget another; [`Floor::none`] behaves as before the floor existed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Floor<'a> {
    through: Option<&'a str>,
}

impl<'a> Floor<'a> {
    /// No signed floor: every signed part is checked.
    pub const fn none() -> Self {
        Self { through: None }
    }

    /// A signed floor through the message `mid`.
    pub const fn through(mid: &'a str) -> Self {
        Self { through: Some(mid) }
    }

    /// The message the floor goes through, when there is one.
    pub const fn mid(&self) -> Option<&'a str> {
        self.through
    }
}

/// Check a view's edits against the rules.
///
/// An edit already in the view a signed part was generated under (its
/// `since` is not later than that view's version) was part of the history the
/// part was signed against, so it does not invalidate the part.
/// `floor` is the runner's signed floor; see [`Floor`].
/// Use [`crate::render::plan_view`] to honor a view's newer-signature policy.
pub fn check(
    entries: &[Entry],
    edits: &[Edit],
    rules: &ProviderRules,
    floor: Floor<'_>,
) -> Result<(), RuleBreak> {
    match invalid_signed_parts(entries, edits, rules, floor)
        .into_iter()
        .next()
    {
        Some((mid, part)) => Err(RuleBreak::SignedPartInvalid { mid, part }),
        None => Ok(()),
    }
}

/// The signed parts a view keeps that its own edits invalidate, in
/// transcript order. A provider adds a drop for each of them; since a drop is
/// itself an edit, it repeats until this returns nothing.
/// [`crate::render::plan_view`] also handles authorized newer-signature drops.
///
/// `floor` is the runner's signed floor; see [`Floor`]. Signed parts it
/// covers are never invalid.
pub fn invalid_signed_parts(
    entries: &[Entry],
    edits: &[Edit],
    rules: &ProviderRules,
    floor: Floor<'_>,
) -> Vec<(String, String)> {
    // An unknown `signed_parts` value is treated as `bound` (W1.5).
    if rules.signed_parts == SignedParts::None {
        return Vec::new();
    }
    let layout = Layout::new(entries, floor);

    // Positions of edits other than leaving out a signed part, with the
    // version that introduced each.
    let mut changes: Vec<(Position, u64)> = Vec::new();
    // Signed parts left out, with the version that left each out.
    let mut signed_drops: std::collections::HashMap<(usize, usize), u64> = Default::default();
    for edit in edits {
        let since = edit.since;
        match &edit.op {
            EditOp::Drop {
                mid,
                part: Some(part),
            } => {
                if let Some(position) = layout.part(mid, part) {
                    if layout.is_signed(position) {
                        signed_drops.insert((position.entry, position.slot), since);
                    } else {
                        changes.push((position, since));
                    }
                }
            }
            EditOp::Drop { mid, part: None } => {
                if let Some(position) = layout.message(mid) {
                    changes.push((position, since));
                }
            }
            EditOp::Replace { mid, part, .. } => {
                if let Some(position) = layout.part(mid, part) {
                    changes.push((position, since));
                }
            }
            // A cut removes the earliest messages, so it changes what every
            // later part was signed after.
            EditOp::Cut { through } => {
                if layout.message(through).is_some() {
                    changes.push((Position::BEFORE_ALL, since));
                }
            }
            EditOp::Insert { at, .. } => {
                let position = match at {
                    InsertAt::Start => Some(Position::BEFORE_ALL),
                    InsertAt::Before(mid) => layout.message(mid),
                    // Refused as `unsupported_edit` anyway; on the safe side
                    // here, it changes everything after it.
                    _ => Some(Position::BEFORE_ALL),
                };
                if let Some(position) = position {
                    changes.push((position, since));
                }
            }
            // An op the runner doesn't know: refused as `unsupported_edit`
            // anyway, and on the safe side here it changes everything.
            _ => changes.push((Position::BEFORE_ALL, since)),
        }
    }
    let dropped_messages: BTreeSet<usize> = edits
        .iter()
        .filter_map(|edit| match &edit.op {
            EditOp::Drop { mid, part: None } => {
                layout.message(mid).map(|position| vec![position.entry])
            }
            EditOp::Cut { through } => layout
                .message(through)
                .map(|last| (0..=last.entry).collect()),
            _ => None,
        })
        .flatten()
        .collect();

    // Every signed part, in transcript order.
    let signed: Vec<Position> = layout.signed();
    let mut invalid = Vec::new();
    for (index, &position) in signed.iter().enumerate() {
        let kept = !dropped_messages.contains(&position.entry)
            && !signed_drops.contains_key(&(position.entry, position.slot));
        if !kept {
            continue;
        }
        let generated = layout.generated_under(position.entry);
        let edited_before = changes
            .iter()
            .any(|(change, since)| *since > generated && *change < position);
        // Signed parts may be left out only at the ends, as an oldest run, a
        // newest run, or both (PR2): a kept part is invalid when a part left
        // out after it was generated lies between it and an earlier kept one.
        // Leaving out the newest parts makes no kept part invalid.
        let mut seen_kept = false;
        let mut gap = false;
        for earlier in &signed[..index] {
            match signed_drops.get(&(earlier.entry, earlier.slot)) {
                Some(since) if *since > generated => {
                    if seen_kept {
                        gap = true;
                        break;
                    }
                }
                Some(_) => {}
                None if dropped_messages.contains(&earlier.entry) => {}
                None => seen_kept = true,
            }
        }
        if edited_before || gap {
            let entry = &entries[position.entry];
            invalid.push((entry.mid.clone(), entry.parts[position.slot - 1].id.clone()));
        }
    }
    invalid
}

/// Resolve the view's explicit permission into temporary drops. These are
/// render instructions, never edits written back into the stored view: the
/// anchor must be checked again after every transcript change.
pub(crate) fn edits_for_view<'a>(
    entries: &[Entry],
    view: &'a View,
    rules: &ProviderRules,
    floor: Floor<'_>,
) -> Result<std::borrow::Cow<'a, [Edit]>, RuleBreak> {
    let anchor = view
        .built_through
        .as_ref()
        .and_then(|mid| entries.iter().position(|entry| &entry.mid == mid));
    let mut edits = std::borrow::Cow::Borrowed(view.edits.as_slice());
    loop {
        let invalid = invalid_signed_parts(entries, &edits, rules, floor);
        if invalid.is_empty() {
            return Ok(edits);
        }
        for (mid, part) in invalid {
            let refusal = || RuleBreak::SignedPartInvalid {
                mid: mid.clone(),
                part: part.clone(),
            };
            // The anchor is a message identity, not an ordinal: a new branch
            // can reuse an old ordinal without having been seen by the view.
            if view.new_signed.as_deref() != Some("strip")
                || !anchor.is_some_and(|anchor| {
                    entries.iter().position(|entry| entry.mid == mid).unwrap() > anchor
                })
            {
                return Err(refusal());
            }
            edits.to_mut().push(Edit::new(
                view.version,
                EditOp::Drop {
                    mid: mid.clone(),
                    part: Some(part.clone()),
                },
            ));
            // Permission to strip a signature does not waive the final-turn
            // rule. Refuse the invalid signature instead of producing a plan
            // that loses a protected tool call or the entry's last part.
            if crate::render::check_final_turn(entries, &edits).is_err() {
                return Err(refusal());
            }
        }
        // A newly omitted signature can create a gap under PR2. Recheck until
        // the retained signatures are valid, or a protected one must refuse.
    }
}

/// A place in the transcript: entry index, then slot within the entry. Slot
/// 0 is before the entry's first part, where message-level edits sit (an
/// insert before the message, or dropping the whole message); part `i` sits at
/// slot `i + 1`. So a message-level edit sorts strictly before every part of
/// its own message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Position {
    entry: usize,
    slot: usize,
}

impl Position {
    const BEFORE_ALL: Position = Position { entry: 0, slot: 0 };
}

struct Layout<'a> {
    entries: &'a [Entry],
    by_mid: std::collections::HashMap<&'a str, usize>,
    /// The last entry the signed floor covers, when the floor names one.
    floor: Option<usize>,
}

impl<'a> Layout<'a> {
    fn new(entries: &'a [Entry], floor: Floor<'_>) -> Self {
        let by_mid: std::collections::HashMap<&'a str, usize> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.mid.as_str(), index))
            .collect();
        let floor = floor.through.and_then(|mid| by_mid.get(mid).copied());
        Self {
            entries,
            by_mid,
            floor,
        }
    }

    fn message(&self, mid: &str) -> Option<Position> {
        self.by_mid
            .get(mid)
            .map(|&entry| Position { entry, slot: 0 })
    }

    fn part(&self, mid: &str, part: &str) -> Option<Position> {
        let &entry = self.by_mid.get(mid)?;
        let index = self.entries[entry]
            .parts
            .iter()
            .position(|candidate| candidate.id == part)?;
        Some(Position {
            entry,
            slot: index + 1,
        })
    }

    fn is_signed(&self, position: Position) -> bool {
        is_signed(&self.entries[position.entry].parts[position.slot - 1].seal)
    }

    /// Every signed part the request can still carry, in transcript order.
    /// Parts the signed floor covers are left out: the runner strips them. A
    /// drop of one still counts as leaving out a signed part, not as an edit
    /// that changes what later parts were signed after.
    fn signed(&self) -> Vec<Position> {
        let mut out = Vec::new();
        for (entry, message) in self.entries.iter().enumerate() {
            if self.floor.is_some_and(|floor| entry <= floor) {
                continue;
            }
            for (part, candidate) in message.parts.iter().enumerate() {
                if is_signed(&candidate.seal) {
                    out.push(Position {
                        entry,
                        slot: part + 1,
                    });
                }
            }
        }
        out
    }

    /// The view version a message was generated under; 0 when it was
    /// generated with no view applied.
    fn generated_under(&self, entry: usize) -> u64 {
        self.entries[entry]
            .generated_under
            .as_ref()
            .map_or(0, |view| view.version)
    }
}

/// Whether a seal binds the part to the history before it. Only `signed`
/// does; an unknown seal is only sealed (W1.5: dropped, never replaced).
fn is_signed(seal: &Seal) -> bool {
    *seal == Seal::Signed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_claude_families_are_bound_on_every_route() {
        for model in [
            "claude-opus-5-5",
            "claude-sonnet-5.5",
            "anthropic/claude-fable-5-1",
            "claude-fable-5-2",
            "anthropic/claude-fable-5.2",
            "us.anthropic.claude-fable-5-2-v1:0",
            "claude-mythos-5-1",
            "claude-mythos-5-2",
            "claude-haiku-5-5@20261001",
            "anthropic.claude-opus-5-5-v1:0",
            "CLAUDE-OPUS-5-5",
        ] {
            assert_eq!(defaults(model).signed_parts, SignedParts::Bound, "{model}");
        }
        for model in [
            "claude-sonnet-5-50",
            "claude-fable-5-20",
            "claude-fable-5",
            "notsonnet-5-5",
            "claude-opus-4-8",
            "gpt-6.1-sol",
            "gemini-3.8-flash",
        ] {
            assert_eq!(defaults(model).signed_parts, SignedParts::None, "{model}");
        }
        assert_eq!(
            defaults("claude-opus-5-5").history_edit_cost,
            EditCost::Suffix
        );
    }
}
