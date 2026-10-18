//! The `tags` group: runner-owned part tags, message origins and
//! `session.resolve`.
//!
//! A runner that declares `tags` ([`crate::capabilities::TAGS`]) gives every
//! message part a tag `§N§`, `N` a positive integer, per session. A tag is
//! assigned when the part is stored, or at the runner's first ingest of a
//! part its host stored itself, and is recorded in the runner's ledger keyed
//! by (message, part). It is durable and never reused: not across a fork, a
//! reset or a restart, and a fork continues numbering after its parent's
//! highest tag ([`next_after`]). Once assigned, a part's key, its tag and
//! its stored content never change.
//!
//! Every message also records where it came from ([`origins`]), at write or
//! first ingest, except the model's own output, which has no origin.
//! `session.read` entries carry both ([`crate::read::ReadMessage::tags`] and
//! [`crate::read::ReadMessage::origin`]); `session.resolve` answers a tag
//! with the part's exact stored content and its message's origin.
//!
//! Whether tags are rendered to the model is the session's fetch plan's
//! `render_tags` member ([`plan_render_tags`]); the numbers exist either way.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::capabilities;

/// The character that opens and closes a tag.
pub const SIGIL: char = '§';

/// The tag of number `n`: `§n§`, with `n` in decimal and no leading zero.
/// `None` for zero, which is never a tag.
pub fn format_tag(n: u64) -> Option<String> {
    (n > 0).then(|| format!("{SIGIL}{n}{SIGIL}"))
}

/// The number of a well-formed tag: `§`, a positive decimal integer without
/// a leading zero that fits a `u64`, then `§`, and nothing else. `None` for
/// anything else, which a runner refuses `invalid_params` naming `tag`.
pub fn parse_tag(tag: &str) -> Option<u64> {
    let digits = tag.strip_prefix(SIGIL)?.strip_suffix(SIGIL)?;
    if digits.is_empty()
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok()
}

/// The first number a session assigns after `highest`, the highest tag
/// number it (or, for a fork, its parent) ever assigned; `None` when it has
/// assigned none. Numbers are never reused, so a fork, a reset and a restart
/// all continue from here.
pub fn next_after(highest: Option<u64>) -> u64 {
    highest.map_or(1, |n| n.saturating_add(1))
}

/// Where a message came from, as `session.read` entries and
/// `session.resolve` answers name it. Decoded as an open string: a consumer
/// keeps a value it does not know as itself.
pub mod origins {
    /// Typed by the person in the harness's own input. Recorded only from
    /// that input path, never from a route: a `session.send` naming it is
    /// refused.
    pub const PERSON: &str = "person";
    /// A `session.send` from the session's owner. What an absent send
    /// `origin` means.
    pub const OWNER: &str = "owner";
    /// A `session.send` the owner makes to wake the session.
    pub const WAKE: &str = "wake";
    /// A `session.send` carrying a message from a peer agent.
    pub const PEER: &str = "peer";
    /// A reminder the runner placed in the transcript. Not a send origin.
    pub const REMINDER: &str = "reminder";
    /// A tool's result, on a runner whose schema stores it as a message of
    /// its own.
    pub const TOOL_RESULT: &str = "tool_result";
    /// Written by the runner or its host without an input behind it (a
    /// placeholder, a host-made notice).
    pub const SYNTHETIC: &str = "synthetic";
    /// Every origin this role names.
    pub const ALL: &[&str] = &[PERSON, OWNER, WAKE, PEER, REMINDER, TOOL_RESULT, SYNTHETIC];
}

/// One part's tag on a `session.read` entry: one row per tagged part of the
/// message, beside the message as `tool_calls` are.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`PartTag::new`], or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct PartTag {
    /// The runner's own key for the part within its message (an index, or a
    /// part id), opaque to consumers: the key its ledger records the tag
    /// under and `session.resolve` answers with. Never changes once
    /// assigned.
    pub part: String,
    /// The part's tag, `§N§`.
    pub tag: String,
}

impl PartTag {
    pub fn new(part: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            part: part.into(),
            tag: tag.into(),
        }
    }
}

/// The `field` an `invalid_params` refusal names for a malformed tag.
pub const TAG_FIELD: &str = "tag";

/// The `field` an `invalid_params` refusal names for a range that is not
/// within the part's content, or is not a range.
pub const RANGE_FIELD: &str = "range";

/// A character range of a part's content: `[start, end)`, counted in
/// Unicode scalar values (not bytes, and not UTF-16 code units), so the
/// same range names the same text in every language.
///
/// Non-exhaustive: build one with [`CharRange::new`], or decode one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CharRange {
    pub start: u64,
    pub end: u64,
}

impl CharRange {
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }
}

/// The `session.resolve` request: `{tag, range?}`.
///
/// Strict: an unknown field is refused `invalid_params` naming it. It names
/// no session: the session is the one the route is bound to.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`ResolveRequest::new`] and [`ResolveRequest::with_range`], or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ResolveRequest {
    /// The tag to resolve, `§N§`.
    pub tag: String,
    /// Only this character range of the part's content. Absent means the
    /// whole content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<CharRange>,
}

impl ResolveRequest {
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            range: None,
        }
    }

    pub fn with_range(mut self, range: CharRange) -> Self {
        self.range = Some(range);
        self
    }

    /// The tag's number, or [`TAG_FIELD`] for the `invalid_params` refusal
    /// of a malformed tag. A well-formed tag the session never assigned is
    /// refused `unknown_tag` instead, which only the runner can tell.
    pub fn number(&self) -> Result<u64, &'static str> {
        parse_tag(&self.tag).ok_or(TAG_FIELD)
    }
}

/// The text of `range` within `content`, counted in Unicode scalar values,
/// or [`RANGE_FIELD`] for the `invalid_params` refusal of a range whose
/// start is after its end or whose end is past the content. `None` is the
/// whole content. An empty range within the content answers `""`.
pub fn slice(content: &str, range: Option<&CharRange>) -> Result<String, &'static str> {
    let Some(range) = range else {
        return Ok(content.to_owned());
    };
    let length = content.chars().count() as u64;
    if range.start > range.end || range.end > length {
        return Err(RANGE_FIELD);
    }
    Ok(content
        .chars()
        .skip(range.start as usize)
        .take((range.end - range.start) as usize)
        .collect())
}

/// The `session.resolve` answer. Decoded leniently.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`ResolveReply::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ResolveReply {
    /// The tag resolved, as the request named it.
    pub tag: String,
    /// The message holding the part.
    pub ordinal: u64,
    pub mid: String,
    /// The part's key, as the message's `session.read` entry names it.
    pub part: String,
    /// The part's exact stored content, or the requested range of it. A text
    /// part's text; any other part in the runner's own stored encoding of
    /// it. The same bytes on every resolve.
    pub content: String,
    /// The requested range, echoed; absent for the whole content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<CharRange>,
    /// The origin of the part's message. Absent for the model's own output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl ResolveReply {
    pub fn new(
        tag: impl Into<String>,
        ordinal: u64,
        mid: impl Into<String>,
        part: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            tag: tag.into(),
            ordinal,
            mid: mid.into(),
            part: part.into(),
            content: content.into(),
            range: None,
            origin: None,
        }
    }

    pub fn with_range(mut self, range: CharRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }
}

/// The fetch-plan member that turns tag rendering on: `render_tags: true`
/// renders each part's tag to the model. Absent (or `null`) means off.
pub const PLAN_RENDER_TAGS: &str = "render_tags";

/// The `field` of an `invalid_params` refusal of the plan's `render_tags`.
pub const PLAN_RENDER_TAGS_FIELD: &str = "plan.render_tags";

/// Whether the plan asks for tags to be rendered to the model: `false` when
/// the member is absent or `null`, or [`PLAN_RENDER_TAGS_FIELD`] for a value
/// that is not a boolean.
pub fn plan_render_tags(plan: &Map<String, Value>) -> Result<bool, &'static str> {
    match plan.get(PLAN_RENDER_TAGS) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(render)) => Ok(*render),
        Some(_) => Err(PLAN_RENDER_TAGS_FIELD),
    }
}

/// Admission's check of a plan's `render_tags` against `declared` (the
/// runner's module-level capabilities): a malformed value, or `true` on a
/// runner that does not declare `tags` and so has no tags to render, is
/// refused `invalid_params` naming [`PLAN_RENDER_TAGS_FIELD`], writing
/// nothing. `false` passes everywhere.
pub fn check_plan<S: AsRef<str>>(
    plan: &Map<String, Value>,
    declared: &[S],
) -> Result<(), &'static str> {
    let render = plan_render_tags(plan)?;
    if render
        && !declared
            .iter()
            .any(|name| name.as_ref() == capabilities::TAGS)
    {
        return Err(PLAN_RENDER_TAGS_FIELD);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, read::ReadMessage, vectors};
    use serde_json::json;

    #[test]
    fn tags_parse_only_in_their_one_spelling() {
        let file = vectors::load("tags.json");
        for case in vectors::cases(&file["spellings"], "valid") {
            let tag = case["tag"].as_str().unwrap();
            let number = case["number"].as_u64().unwrap();
            assert_eq!(parse_tag(tag), Some(number), "{tag}");
            assert_eq!(format_tag(number).as_deref(), Some(tag));
        }
        for case in vectors::cases(&file["spellings"], "invalid") {
            let tag = case.as_str().unwrap();
            assert_eq!(parse_tag(tag), None, "{tag}");
            assert_eq!(ResolveRequest::new(tag).number(), Err(TAG_FIELD), "{tag}");
        }
        assert_eq!(format_tag(0), None);
        let fork = &file["fork"];
        assert_eq!(
            format_tag(next_after(fork["parent_highest"].as_u64())),
            fork["first_new"].as_str().map(str::to_owned)
        );
        assert_eq!(next_after(None), 1);
    }

    #[test]
    fn read_entries_carry_part_tags_and_origin() {
        let file = vectors::load("tags.json");
        let mut seen = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "read_entries") {
            let name = case["name"].as_str().unwrap();
            let entry: ReadMessage = vectors::round_trip(name, &case["entry"]);
            let mut parts = std::collections::BTreeSet::new();
            for row in &entry.tags {
                assert!(parse_tag(&row.tag).is_some(), "{name}: {}", row.tag);
                assert!(parts.insert(row.part.clone()), "{name}: part twice");
                assert!(seen.insert(row.tag.clone()), "{name}: {} reused", row.tag);
            }
            if let Some(origin) = &entry.origin {
                assert!(origins::ALL.contains(&origin.as_str()), "{name}: {origin}");
            }
        }
        for origin in origins::ALL {
            assert!(
                vectors::cases(&file, "read_entries")
                    .iter()
                    .any(|case| case["entry"]["origin"] == *origin),
                "no read entry has origin {origin}"
            );
        }
        // A runner without the group sends neither member, and a consumer
        // still decodes the entry; an origin this role does not name decodes
        // as itself.
        let plain: ReadMessage =
            serde_json::from_value(json!({"ordinal": 0, "mid": "m0", "message": {}})).unwrap();
        assert!(plain.tags.is_empty() && plain.origin.is_none());
        let future: ReadMessage = serde_json::from_value(
            json!({"ordinal": 0, "mid": "m0", "message": {}, "origin": "future", "x": 1}),
        )
        .unwrap();
        assert_eq!(future.origin.as_deref(), Some("future"));
    }

    #[test]
    fn resolve_answers_stored_content_or_refuses_by_name() {
        let file = vectors::load("tags.json");
        for case in vectors::cases(&file, "resolve_requests") {
            vectors::round_trip::<ResolveRequest>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            let error =
                serde_json::from_value::<ResolveRequest>(case["request"].clone()).expect_err(name);
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
        let parts = vectors::cases(&file, "ledger");
        let mut kinds = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "resolve_cases") {
            let name = case["name"].as_str().unwrap();
            let request: ResolveRequest = vectors::round_trip(name, &case["request"]);
            let decided = request.number().map_err(|field| {
                (errors::INVALID_PARAMS, Some(json!({ "field": field })))
            });
            let decided = decided.and_then(|_| {
                let part = parts
                    .iter()
                    .find(|part| part["tag"] == request.tag.as_str())
                    .ok_or((errors::UNKNOWN_TAG, None))?;
                let content = slice(part["content"].as_str().unwrap(), request.range.as_ref())
                    .map_err(|field| (errors::INVALID_PARAMS, Some(json!({ "field": field }))))?;
                let mut reply = ResolveReply::new(
                    &request.tag,
                    part["ordinal"].as_u64().unwrap(),
                    part["mid"].as_str().unwrap(),
                    part["part"].as_str().unwrap(),
                    content,
                );
                if let Some(range) = request.range {
                    reply = reply.with_range(range);
                }
                if let Some(origin) = part["origin"].as_str() {
                    reply = reply.with_origin(origin);
                }
                Ok(reply)
            });
            match (decided, case.get("refusal")) {
                (Ok(reply), None) => {
                    let expected: ResolveReply = vectors::round_trip(name, &case["reply"]);
                    assert_eq!(reply, expected, "{name}");
                    kinds.insert(if reply.range.is_some() { "range" } else { "whole" });
                }
                (Err((code, detail)), Some(refusal)) => {
                    assert_eq!(refusal["code"], code, "{name}");
                    assert_eq!(refusal.get("detail"), detail.as_ref(), "{name}");
                    assert!(errors::CODES.contains(&code), "{name}");
                    assert!(!errors::is_retryable(code), "{name}");
                    kinds.insert(match (code, detail) {
                        (errors::UNKNOWN_TAG, _) => "unknown_tag",
                        (_, Some(detail)) if detail["field"] == RANGE_FIELD => "range_refused",
                        _ => "tag_refused",
                    });
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
        for kind in ["whole", "range", "unknown_tag", "range_refused", "tag_refused"] {
            assert!(kinds.contains(kind), "no resolve vector for {kind}");
        }
        let lenient: ResolveReply = serde_json::from_value(json!({
            "tag": "§1§", "ordinal": 0, "mid": "m", "part": "0", "content": "", "future": true
        }))
        .unwrap();
        assert_eq!(lenient, ResolveReply::new("§1§", 0, "m", "0", ""));
    }

    #[test]
    fn ranges_count_unicode_scalar_values() {
        assert_eq!(slice("héllo", Some(&CharRange::new(1, 3))), Ok("él".into()));
        assert_eq!(slice("héllo", Some(&CharRange::new(5, 5))), Ok(String::new()));
        assert_eq!(slice("héllo", Some(&CharRange::new(0, 6))), Err(RANGE_FIELD));
        assert_eq!(slice("héllo", Some(&CharRange::new(3, 2))), Err(RANGE_FIELD));
        assert_eq!(slice("héllo", None), Ok("héllo".into()));
    }

    #[test]
    fn render_tags_is_a_plan_member_off_when_absent() {
        let file = vectors::load("tags.json");
        for case in vectors::cases(&file, "plans") {
            let name = case["name"].as_str().unwrap();
            let plan = case["plan"].as_object().unwrap();
            let declared: Vec<&str> = case["declares"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect();
            match (check_plan(plan, &declared), case.get("refusal")) {
                (Ok(()), None) => assert_eq!(
                    plan_render_tags(plan),
                    Ok(case["render_tags"].as_bool().unwrap()),
                    "{name}"
                ),
                (Err(field), Some(refusal)) => {
                    assert_eq!(refusal["code"], errors::INVALID_PARAMS, "{name}");
                    assert_eq!(refusal["detail"]["field"], field, "{name}");
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
    }
}
