//! The `tags` group: runner-rendered part numbers and `session.resolve`.
//!
//! A runner that declares `tags` ([`crate::capabilities::TAGS`]) shows the
//! model a number `§N§` on every part it renders with one, and reports each
//! number on `session.read` entries ([`crate::read::ReadMessage::tags`]). A
//! tag number is only a name the model can see, computed from the
//! transcript ([`numbering`]): the checkpoint before the cut plus the part's
//! position among the counted parts, in rendered order. It is never stored
//! per part and never keys anything durable; durable state is keyed by the
//! host's own identity (message id, part key, content digest). Any
//! renumbering of a part a read has exposed starts a new lineage, so within
//! one lineage a number names one part.
//!
//! The model's own text is cleaned of tag notation on every exposure
//! ([`clean`]), for text written by a run whose frozen plan renders tags.
//! `session.resolve` answers a number in the session's current lineage.
//!
//! Whether tags are rendered to the model is the session's fetch plan's
//! `render_tags` member ([`plan_render_tags`]); numbering, rendering and
//! cleaning apply only where it is `true`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::capabilities;

pub mod checkpoint;
pub mod clean;
pub mod numbering;

pub use checkpoint::{
    message_checkpoint_anchors, next_message_count, opencode1_message_counts,
    opencode1_message_counts_with_parts, CheckpointAnchors, CheckpointPosition, MessageAnchor,
    MessageCheckpointError,
};

pub use clean::{clean_model_text, strip_leading_tag_notation};
// Kept so 0.4.0 callers still build; it saturates, and its own deprecation
// note points them at `try_number_parts`.
#[allow(deprecated)]
pub use numbering::number_parts;
pub use numbering::{
    anchor, counts, try_number_parts, Anchor, Marker, MarkerCheckpoint, NumberingOverflow,
    PartKind, RenderedPart, SetupCheckpoint,
};

/// The name under which a run freezes these numbering and cleaning rules
/// with the rest of its configuration. A run frozen under it numbers by
/// [`numbering`] and cleans by [`clean`] for its whole life; a session's
/// earlier runs keep the rules they were frozen under. It succeeds the
/// stored-ledger rules some runners recorded as `runner-tags/v1`.
pub const RULES_FEATURE: &str = "runner-tags/v2";

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

/// One part of a `session.read` entry: one row per part the runner exposes,
/// in the message's part order, beside the message as `tool_calls` are.
/// Every exposed part has a row, numbered or not, so a reader learns every
/// part key from the rows alone.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`PartTag::tagged`] or [`PartTag::untagged`], or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct PartTag {
    /// The runner's own key for the part within its message (an index, or a
    /// part id), opaque to consumers: the host identity `session.resolve`
    /// answers with and a compaction provider addresses part-level view
    /// edits by. Stable: never changes once the part is exposed.
    pub part: String,
    /// The number the part shows in the current lineage, `§N§`. Absent for a
    /// part that does not count ([`counts`]): an image or file in a user
    /// message, reasoning, a tool call, the history head, a marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl PartTag {
    pub fn tagged(part: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            part: part.into(),
            tag: Some(tag.into()),
        }
    }

    pub fn untagged(part: impl Into<String>) -> Self {
        Self {
            part: part.into(),
            tag: None,
        }
    }
}

/// What separates a rendered tag from the text after it: one space.
pub const RENDER_SEPARATOR: &str = " ";

/// The text a counted text part renders as: its number, one space, then its
/// text with any leading tag notation stripped
/// ([`strip_leading_tag_notation`], never bare digits). The model's own text
/// is passed as exposed, so already cleaned when a tag-rendering run wrote
/// it.
///
/// Rendering what it rendered gives the same bytes again,
/// `render_counted_text(n, &render_counted_text(m, t)) ==
/// render_counted_text(n, t)`, for every text that does not begin with
/// whitespace (cleaned model text never does). A runner renders from the
/// exposed text each time, never from an earlier rendering.
pub fn render_counted_text(number: u64, text: &str) -> String {
    format!(
        "{SIGIL}{number}{SIGIL}{RENDER_SEPARATOR}{}",
        strip_leading_tag_notation(text)
    )
}

/// What follows the number in an image-only tool result's label.
pub const IMAGE_LABEL_TEXT: &str = "[image]";

/// What follows the number in a non-image media-only tool result's label.
pub const MEDIA_LABEL_TEXT: &str = "[media]";

/// The short text label an image-only tool result renders, carrying its
/// number: `§N§ [image]`. The runner places it where the provider accepts
/// text beside the image (on Anthropic's API, a text block before the image
/// inside the `tool_result`).
pub fn image_label(number: u64) -> String {
    format!("{SIGIL}{number}{SIGIL}{RENDER_SEPARATOR}{IMAGE_LABEL_TEXT}")
}

/// The short text label for a tool result with non-image media but no non-empty
/// text: `§N§ [media]`. Place it where the provider accepts text beside media.
pub fn media_label(number: u64) -> String {
    format!("{SIGIL}{number}{SIGIL}{RENDER_SEPARATOR}{MEDIA_LABEL_TEXT}")
}

/// A tool-result block as numbering sees it, independent of provider storage.
/// Map text to its exposed bytes, images to `Image`, and files or other media
/// to `Media`. A structured value with no text block contributes no blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolResultBlock<'a> {
    Text(&'a str),
    Image,
    Media,
}

/// The block to prefix when rendering a whole tool result's number. Empty
/// exposed text is ignored. Later text and all media render as exposed without
/// a number. `None` means use the label selected by [`tool_result_kind`], if any.
pub fn numbered_block_index(blocks: &[ToolResultBlock<'_>]) -> Option<usize> {
    blocks
        .iter()
        .position(|block| matches!(block, ToolResultBlock::Text(text) if !text.is_empty()))
}

/// Classify one whole tool result, never individual blocks. Non-empty exposed
/// text takes precedence over media; otherwise any non-image media selects the
/// media label, images select the image label, and no media means uncounted.
/// A text block containing JSON is text; a structured value alone is not.
pub fn tool_result_kind(blocks: &[ToolResultBlock<'_>]) -> PartKind {
    if numbered_block_index(blocks).is_some() {
        PartKind::ToolResultText
    } else if blocks.contains(&ToolResultBlock::Media) {
        PartKind::ToolResultMediaOnly
    } else if blocks.contains(&ToolResultBlock::Image) {
        PartKind::ToolResultImageOnly
    } else {
        PartKind::EmptyText
    }
}

/// The whole rendered text of a counted part a compaction provider dropped:
/// `[dropped §N§]`, no prefix before it. The part keeps its number.
pub fn dropped_placeholder(number: u64) -> String {
    format!("[dropped {SIGIL}{number}{SIGIL}]")
}

/// The text a runner renders for a part of `kind` numbered `number`, when
/// the run renders tags: [`render_counted_text`] for counted text,
/// [`image_label`] or [`media_label`] for a tool result without non-empty text
/// (its `text` is unused), and
/// `text` unchanged for a part that does not count or has no number.
pub fn render_part(kind: PartKind, number: Option<u64>, text: &str) -> String {
    match (kind, number) {
        (PartKind::ToolResultImageOnly, Some(number)) => image_label(number),
        (PartKind::ToolResultMediaOnly, Some(number)) => media_label(number),
        (kind, Some(number)) if counts(kind) => render_counted_text(number, text),
        _ => text.to_owned(),
    }
}

/// Who wrote a piece of the model's text, which decides how it is exposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TextWriter {
    /// A run whose frozen plan renders tags. Cleaned on every exposure.
    TagRenderingRun,
    /// A run whose frozen plan renders no tags. Exposed byte for byte,
    /// whatever later runs of the session render.
    UntaggedRun,
    /// The final text of a structured-output run, under any plan. Never
    /// cleaned: a tag-like string inside the output is the output.
    StructuredOutputFinal,
}

/// The model's text as every exposure shows it (`session.read`,
/// `session.resolve`, `run.result`'s final text, a render, the summarizer's
/// feed): [`clean_model_text`] for text a tag-rendering run wrote, and the
/// stored text otherwise. Applies to the model's text parts only: signed
/// thinking, encrypted reasoning and tool-call arguments are never cleaned.
/// Storage may keep the raw text.
pub fn exposed_model_text(writer: TextWriter, text: &str) -> std::borrow::Cow<'_, str> {
    match writer {
        TextWriter::TagRenderingRun => std::borrow::Cow::Owned(clean_model_text(text)),
        TextWriter::UntaggedRun | TextWriter::StructuredOutputFinal => {
            std::borrow::Cow::Borrowed(text)
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

/// The `session.resolve` request: `{tag, range?, media?}`.
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::read::descriptors::deserialize_media"
    )]
    pub media: Option<crate::read::MediaMode>,
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
            media: None,
            range: None,
        }
    }

    pub fn with_range(mut self, range: CharRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn with_media(mut self, media: crate::read::MediaMode) -> Self {
        self.media = Some(media);
        self
    }

    /// The tag's number, or [`TAG_FIELD`] for the `invalid_params` refusal
    /// of a malformed tag. A well-formed tag no part of the current lineage
    /// carries is refused `unknown_tag` instead, which only the runner can
    /// tell.
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
    /// The part's exposed content, or the requested range of it: a text
    /// part's text, cleaned when it is the model's text and a tag-rendering
    /// run wrote it ([`exposed_model_text`]); an image-only tool result in
    /// the runner's own stored encoding of it. The same bytes on every
    /// resolve within a lineage for the same media mode. Inline is a string;
    /// descriptors mode may instead carry a ref or a structured part with
    /// payload refs, preserving its call pairing fields.
    #[serde(deserialize_with = "crate::read::descriptors::deserialize_content")]
    pub content: serde_json::Value,
    /// The requested range, echoed; absent for the whole content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<CharRange>,
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
            content: serde_json::Value::String(content.into()),
            range: None,
        }
    }

    pub fn with_range(mut self, range: CharRange) -> Self {
        self.range = Some(range);
        self
    }
}

/// The fetch-plan member that turns tag rendering on: `render_tags: true`
/// numbers, renders and cleans by [`RULES_FEATURE`]. Absent (or `null`)
/// means off: no part carries a number and the model's text is exposed as
/// written.
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
    }

    /// Within one lineage the numbers run consecutively over the counted
    /// parts, each used once; a part that does not count has a row and no
    /// number.
    #[test]
    fn read_entries_carry_consecutive_numbers_on_counted_parts() {
        let file = vectors::load("tags.json");
        let mut numbers = Vec::new();
        let mut untagged = 0;
        for case in vectors::cases(&file, "read_entries") {
            let name = case["name"].as_str().unwrap();
            let entry: ReadMessage = vectors::round_trip(name, &case["entry"]);
            let mut parts = std::collections::BTreeSet::new();
            for row in &entry.tags {
                assert!(parts.insert(row.part.clone()), "{name}: part twice");
                match &row.tag {
                    Some(tag) => numbers.push(parse_tag(tag).expect(name)),
                    None => untagged += 1,
                }
            }
        }
        let expected: Vec<u64> = (1..=numbers.len() as u64).collect();
        assert_eq!(numbers, expected, "numbers run consecutively");
        assert!(
            untagged >= 4,
            "reasoning, a tool call, an image and turn_system"
        );
        // A runner without the group sends no tags, and a consumer still
        // decodes the entry.
        let plain: ReadMessage =
            serde_json::from_value(json!({"ordinal": 0, "mid": "m0", "message": {}, "x": 1}))
                .unwrap();
        assert!(plain.tags.is_empty());
    }

    #[test]
    fn resolve_answers_the_current_lineage_or_refuses_by_name() {
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
        let parts = vectors::cases(&file, "lineage");
        let mut kinds = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "resolve_cases") {
            let name = case["name"].as_str().unwrap();
            let request: ResolveRequest = vectors::round_trip(name, &case["request"]);
            let decided = request
                .number()
                .map_err(|field| (errors::INVALID_PARAMS, Some(json!({ "field": field }))));
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
                Ok(reply)
            });
            match (decided, case.get("refusal")) {
                (Ok(reply), None) => {
                    let expected: ResolveReply = vectors::round_trip(name, &case["reply"]);
                    assert_eq!(reply, expected, "{name}");
                    kinds.insert(if reply.range.is_some() {
                        "range"
                    } else {
                        "whole"
                    });
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
        for kind in [
            "whole",
            "range",
            "unknown_tag",
            "range_refused",
            "tag_refused",
        ] {
            assert!(kinds.contains(kind), "no resolve vector for {kind}");
        }
        let lenient: ResolveReply = serde_json::from_value(json!({
            "tag": "§1§", "ordinal": 0, "mid": "m", "part": "0", "content": "", "future": true
        }))
        .unwrap();
        assert_eq!(lenient, ResolveReply::new("§1§", 0, "m", "0", ""));
    }

    #[test]
    fn rendered_parts_follow_the_counting_predicate() {
        let file = vectors::load("tags.json");
        let mut kinds = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "rendered_parts") {
            let name = case["name"].as_str().unwrap();
            let kind: PartKind = serde_json::from_value(case["kind"].clone()).unwrap();
            let number = case["number"].as_u64();
            assert_eq!(number.is_some(), counts(kind), "{name}");
            let rendered = render_part(kind, number, case["text"].as_str().unwrap());
            assert_eq!(rendered, case["rendered"].as_str().unwrap(), "{name}");
            kinds.insert(format!("{kind:?}"));
        }
        assert_eq!(kinds.len(), 12, "every part kind has a vector");
        for case in vectors::cases(&file, "dropped") {
            let number = case["number"].as_u64().unwrap();
            assert_eq!(
                dropped_placeholder(number),
                case["rendered"].as_str().unwrap()
            );
        }
        assert_eq!(image_label(3), "§3§ [image]");
        assert_eq!(media_label(3), "§3§ [media]");
    }

    fn tool_result_vector(name: &str) {
        let file = vectors::load("tags.json");
        let case = vectors::cases(&file, "tool_results")
            .iter()
            .find(|case| case["name"] == name)
            .expect("tool-result vector");
        let content = case["result"]["content"].as_array();
        let blocks: Vec<_> = content
            .into_iter()
            .flatten()
            .map(|block| match block["type"].as_str().unwrap() {
                "text" => ToolResultBlock::Text(block["text"].as_str().unwrap()),
                "image" => ToolResultBlock::Image,
                _ => ToolResultBlock::Media,
            })
            .collect();
        let kind = tool_result_kind(&blocks);
        assert_eq!(serde_json::to_value(kind).unwrap(), case["kind"]);
        let index = numbered_block_index(&blocks);
        assert_eq!(json!(index), case["numbered_block"]);
        let numbers = try_number_parts(
            40,
            &[
                RenderedPart::new(kind, 0),
                RenderedPart::new(PartKind::AssistantText, 1),
            ],
        )
        .unwrap();
        assert_eq!(json!(numbers), case["numbers"]);
        let mut rendered = content.cloned().unwrap_or_default();
        if let Some(index) = index {
            rendered[index]["text"] = json!(render_part(
                kind,
                numbers[0],
                rendered[index]["text"].as_str().unwrap()
            ));
        } else if counts(kind) {
            rendered.insert(
                0,
                json!({"type":"text", "text":render_part(kind, numbers[0], "")}),
            );
        }
        assert_eq!(json!(rendered), case["rendered_blocks"]);
    }

    #[test]
    fn tool_result_text_image_text() {
        tool_result_vector("text_image_text");
    }

    #[test]
    fn tool_result_image_text() {
        tool_result_vector("image_text");
    }

    #[test]
    fn tool_result_empty_text_then_text() {
        tool_result_vector("empty_text_then_text");
    }

    #[test]
    fn tool_result_image_empty_text() {
        tool_result_vector("image_empty_text");
    }

    #[test]
    fn tool_result_pdf() {
        tool_result_vector("pdf");
    }

    #[test]
    fn tool_result_image_pdf() {
        tool_result_vector("image_pdf");
    }

    #[test]
    fn tool_result_empty_text() {
        tool_result_vector("empty_text");
    }

    #[test]
    fn tool_result_structured_only() {
        tool_result_vector("structured_only");
    }

    #[test]
    fn tool_result_json_text() {
        tool_result_vector("json_text");
    }

    #[test]
    fn ranges_count_unicode_scalar_values() {
        assert_eq!(slice("héllo", Some(&CharRange::new(1, 3))), Ok("él".into()));
        assert_eq!(
            slice("héllo", Some(&CharRange::new(5, 5))),
            Ok(String::new())
        );
        assert_eq!(
            slice("héllo", Some(&CharRange::new(0, 6))),
            Err(RANGE_FIELD)
        );
        assert_eq!(
            slice("héllo", Some(&CharRange::new(3, 2))),
            Err(RANGE_FIELD)
        );
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

/// Cleaning and rendering vectors from the tag cleaning in standalone Magic
/// Context, the plugin that manages context inside OpenCode and Pi
/// (`tag-content-primitives.ts`, `text-complete.ts`). They preserve section
/// references: `§` followed by digits, then `.` and a digit, is never a dangling
/// tag. The source pattern backtracks one digit and cleans `§12.3` to `2.3`;
/// the vectors record that output separately from the corrected result.
#[cfg(test)]
mod cleaning_tests {
    use super::*;
    use crate::vectors;

    fn cases(section: &str) -> Vec<serde_json::Value> {
        vectors::cases(&vectors::load("tag-cleaning.json"), section).to_vec()
    }

    /// Whether `text` contains a sigil followed by two or more digits, then
    /// a decimal point and a digit.
    fn multi_digit_decimal(text: &str) -> bool {
        text.split(SIGIL).skip(1).any(|after| {
            let digits = after.bytes().take_while(u8::is_ascii_digit).count();
            let mut rest = after[digits..].chars();
            digits >= 2
                && rest.next() == Some('.')
                && rest.next().is_some_and(|c| c.is_ascii_digit())
        })
    }

    /// Rows with a recorded source-pattern difference contain the multi-digit
    /// section references affected by regex backtracking.
    fn standalone_differs_only_on_multi_digit_decimals(cases: &[serde_json::Value]) {
        for case in cases {
            if case.get("standalone_before_fix").is_some() {
                let name = case["name"].as_str().unwrap();
                assert!(
                    multi_digit_decimal(case["text"].as_str().unwrap()),
                    "{name}: standalone differs on a text the decimal fix does not change"
                );
            }
        }
    }

    #[test]
    fn model_text_is_cleaned_by_the_shared_rules() {
        let cases = cases("clean");
        assert!(cases.iter().any(|case| case["source"] == "commons"));
        standalone_differs_only_on_multi_digit_decimals(&cases);
        for case in &cases {
            let name = case["name"].as_str().unwrap();
            let cleaned = clean_model_text(case["text"].as_str().unwrap());
            assert_eq!(cleaned, case["cleaned"].as_str().unwrap(), "{name}");
            assert_eq!(clean_model_text(&cleaned), cleaned, "{name}: idempotent");
            assert!(!cleaned.contains(SIGIL), "{name}: no sigil survives");
        }
    }

    #[test]
    fn leading_tag_notation_is_stripped_and_bare_digits_kept() {
        let cases = cases("strip_leading");
        standalone_differs_only_on_multi_digit_decimals(&cases);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let stripped = strip_leading_tag_notation(case["text"].as_str().unwrap());
            assert_eq!(stripped, case["stripped"].as_str().unwrap(), "{name}");
            assert_eq!(
                strip_leading_tag_notation(stripped),
                stripped,
                "{name}: idempotent"
            );
        }
    }

    #[test]
    fn counted_text_renders_its_number_and_rendering_again_changes_nothing() {
        for case in cases("render") {
            let name = case["name"].as_str().unwrap();
            let number = case["number"].as_u64().unwrap();
            let text = case["text"].as_str().unwrap();
            let rendered = render_counted_text(number, text);
            assert_eq!(rendered, case["rendered"].as_str().unwrap(), "{name}");
            assert_eq!(render_counted_text(number, &rendered), rendered, "{name}");
            assert_eq!(
                render_counted_text(number + 1, &rendered),
                render_counted_text(number + 1, text),
                "{name}: a new number replaces the old one"
            );
        }
        // The model's correct self-tag survives cleaning and rendering byte
        // for byte, as standalone's round-trip test pins.
        let reply = "§12§ Reading both files in parallel.\n\nThe next step follows.";
        assert_eq!(render_counted_text(12, &clean_model_text(reply)), reply);
    }

    #[test]
    fn structured_output_final_text_is_never_cleaned() {
        let mut writers = std::collections::BTreeSet::new();
        for case in cases("exposure") {
            let name = case["name"].as_str().unwrap();
            let writer: TextWriter = serde_json::from_value(case["writer"].clone()).unwrap();
            let text = case["text"].as_str().unwrap();
            assert_eq!(
                exposed_model_text(writer, text),
                case["exposed"].as_str().unwrap(),
                "{name}"
            );
            writers.insert(format!("{writer:?}"));
        }
        assert_eq!(writers.len(), 3, "every writer has a vector");
    }
}

/// Counting, numbering and anchoring against `tag-numbering.json`.
#[cfg(test)]
mod numbering_tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn one_predicate_decides_what_counts() {
        let file = vectors::load("tag-numbering.json");
        for case in vectors::cases(&file, "counts") {
            let kind: PartKind = serde_json::from_value(case["kind"].clone()).unwrap();
            assert_eq!(counts(kind), case["counts"].as_bool().unwrap(), "{kind:?}");
        }
    }

    #[test]
    fn numbers_follow_rendered_order_from_the_checkpoint() {
        let file = vectors::load("tag-numbering.json");
        let mut reordered = false;
        for case in vectors::cases(&file, "numbering") {
            let name = case["name"].as_str().unwrap();
            let parts: Vec<RenderedPart> = serde_json::from_value(case["parts"].clone()).unwrap();
            let expected: Vec<Option<u64>> =
                serde_json::from_value(case["numbers"].clone()).unwrap();
            assert_eq!(
                try_number_parts(case["checkpoint"].as_u64().unwrap(), &parts),
                Ok(expected),
                "{name}"
            );
            reordered |= parts
                .windows(2)
                .any(|w| w[0].rendered_at > w[1].rendered_at);
        }
        assert!(reordered, "no vector stores parts out of rendered order");
    }

    /// Numbering that would pass `u64::MAX` is refused, naming the
    /// checkpoint and the counted parts, and one ending exactly at
    /// `u64::MAX` is numbered. The saturating `number_parts` would give the
    /// refused case's two parts the same number.
    #[test]
    fn numbering_past_u64_max_is_refused_never_repeated() {
        let file = vectors::load("tag-numbering.json");
        let cases = vectors::cases(&file, "overflow");
        assert!(cases.iter().any(|case| case.get("refused").is_some()));
        assert!(cases.iter().any(|case| case.get("numbers").is_some()));
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let checkpoint = case["checkpoint"].as_u64().unwrap();
            let parts: Vec<RenderedPart> = serde_json::from_value(case["parts"].clone()).unwrap();
            let numbered = try_number_parts(checkpoint, &parts);
            match case.get("refused") {
                Some(refused) => {
                    let overflow = numbered.expect_err(name);
                    assert_eq!(
                        overflow.checkpoint,
                        refused["checkpoint"].as_u64().unwrap(),
                        "{name}"
                    );
                    assert_eq!(
                        overflow.counted,
                        refused["counted"].as_u64().unwrap(),
                        "{name}"
                    );
                    #[allow(deprecated)]
                    let saturated = number_parts(checkpoint, &parts);
                    let shown: Vec<u64> = saturated.into_iter().flatten().collect();
                    assert!(shown.windows(2).any(|w| w[0] == w[1]), "{name}: {shown:?}");
                }
                None => {
                    let expected: Vec<Option<u64>> =
                        serde_json::from_value(case["numbers"].clone()).unwrap();
                    assert!(expected.contains(&Some(u64::MAX)), "{name}");
                    assert_eq!(numbered, Ok(expected), "{name}");
                }
            }
        }
    }

    /// Number a session from the anchor `markers` and `setup` give: every
    /// part from the anchor's cut, in storage order (as OpenCode renders,
    /// marker pairs never counting), fed to [`try_number_parts`]. Answers the
    /// numbers by `message:part` and the anchor.
    fn number_from_anchor(
        case: &serde_json::Value,
        markers: &serde_json::Value,
    ) -> (std::collections::BTreeMap<String, u64>, Anchor) {
        let name = case["name"].as_str().unwrap();
        let markers: Vec<Marker> = serde_json::from_value(markers.clone()).unwrap();
        let setup: Option<SetupCheckpoint> = case
            .get("setup")
            .map(|setup| serde_json::from_value(setup.clone()).unwrap());
        let messages = case["messages"].as_array().unwrap();
        let ids: Vec<&str> = messages.iter().map(|m| m["id"].as_str().unwrap()).collect();
        let found = anchor("s", &markers, setup.as_ref(), |id| ids.contains(&id)).expect(name);
        let (checkpoint, from) = match &found {
            Anchor::Marker { checkpoint, cut } | Anchor::Setup { checkpoint, cut } => (
                *checkpoint,
                ids.iter().position(|id| id == cut).expect(name),
            ),
            Anchor::SessionStart { checkpoint, .. } => (*checkpoint, 0),
        };
        let mut keys = Vec::new();
        let mut parts = Vec::new();
        for (at, message) in messages.iter().enumerate().skip(from) {
            for (index, kind) in message["parts"].as_array().unwrap().iter().enumerate() {
                keys.push(format!("{}:{index}", ids[at]));
                parts.push(RenderedPart::new(
                    serde_json::from_value(kind.clone()).unwrap(),
                    at as u64,
                ));
            }
        }
        let numbers = keys
            .into_iter()
            .zip(try_number_parts(checkpoint, &parts).expect(name))
            .filter_map(|(key, number)| Some((key, number?)))
            .collect();
        (numbers, found)
    }

    /// A lost key renumbers nothing: falling back to an older keyed marker,
    /// the Setup checkpoint or the session start, and counting every part
    /// from that cut, gives the numbers computed with every key present.
    #[test]
    fn a_lost_marker_key_falls_back_without_renumbering() {
        let file = vectors::load("tag-numbering.json");
        let cases = vectors::cases(&file, "lost_keys");
        assert_eq!(cases.len(), 4);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let expected: std::collections::BTreeMap<String, u64> =
                serde_json::from_value(case["numbers"].clone()).unwrap();
            let (with_keys, newest) = number_from_anchor(case, &case["markers_with_keys"]);
            let (without, fallback) = number_from_anchor(case, &case["markers"]);
            assert_ne!(newest, fallback, "{name}: no fallback happened");
            for (key, number) in &expected {
                assert_eq!(
                    with_keys.get(key),
                    Some(number),
                    "{name}: {key} with every key"
                );
                assert_eq!(
                    without.get(key),
                    Some(number),
                    "{name}: {key} after the key was lost"
                );
            }
        }
    }

    #[test]
    fn the_anchor_falls_back_from_markers_to_the_setup_checkpoint() {
        let file = vectors::load("tag-numbering.json");
        for case in vectors::cases(&file, "anchors") {
            let name = case["name"].as_str().unwrap();
            let markers: Vec<Marker> = serde_json::from_value(case["markers"].clone()).unwrap();
            let setup: Option<SetupCheckpoint> = case
                .get("setup")
                .map(|setup| vectors::round_trip(name, setup));
            let existing: Vec<&str> = case["existing"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap())
                .collect();
            let outcome = anchor(
                case["session"].as_str().unwrap(),
                &markers,
                setup.as_ref(),
                |id| existing.contains(&id),
            );
            match (outcome, case.get("fork_parent")) {
                (Err(parent), Some(expected)) => assert_eq!(parent, expected.as_str().unwrap()),
                (Ok(found), None) => {
                    let expected: Anchor = vectors::round_trip(name, &case["anchor"]);
                    assert_eq!(found, expected, "{name}");
                }
                (outcome, expected) => panic!("{name}: {outcome:?} against {expected:?}"),
            }
        }
    }
}
