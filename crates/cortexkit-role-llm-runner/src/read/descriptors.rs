//! Payload references for bounded reads. Runners identify payload locations
//! in their own schema; the shared code replaces only those bytes, leaving
//! call identities, error flags, signatures and provider metadata intact.

use crate::content_digest::{content_digest, is_sha256_hex, sha256_hex};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub const PART_INLINE_MAX: usize = 262_144;
pub const TEXT_HEAD_MAX: usize = 4_096;
/// Maximum encoded ReadMessage in descriptors mode: 3 MiB, leaving 1 MiB
/// of the daemon frame for the page envelope and transport header.
pub const DESCRIPTOR_MESSAGE_MAX: usize = 3_145_728;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaMode {
    #[default]
    Inline,
    Descriptors,
    /// Opt in to stored media blocks; default reads render text markers.
    Blocks,
}

pub(crate) fn deserialize_media<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<MediaMode>, D::Error> {
    MediaMode::deserialize(d).map(Some).map_err(|error| {
        serde::de::Error::custom(format!(
            "media must be inline, descriptors or blocks: {error}"
        ))
    })
}

fn digest<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if !is_sha256_hex(&value) {
        return Err(serde::de::Error::custom("expected lowercase SHA-256"));
    }
    Ok(value)
}

pub(crate) fn deserialize_digest<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    digest(d).map(Some)
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartDigest {
    pub part: String,
    #[serde(deserialize_with = "digest")]
    pub digest: String,
}

impl PartDigest {
    pub fn new(part: impl Into<String>, inline: &Value) -> Self {
        Self {
            part: part.into(),
            digest: content_digest(inline),
        }
    }
}

/// Strict reference wire shapes. `part` is present only when the ref itself
/// replaces a top-level part; nested payload refs have no independent key.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Reference {
    MediaRef {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        kind: String,
        mime: String,
        byte_size: u64,
        #[serde(deserialize_with = "digest")]
        sha256: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u64>,
    },
    TextRef {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        byte_size: u64,
        #[serde(deserialize_with = "digest")]
        sha256: String,
        char_count: u64,
        head: String,
    },
    ArgsRef {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        byte_size: u64,
        #[serde(deserialize_with = "digest")]
        sha256: String,
        name: String,
    },
    ReasoningRef {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        byte_size: u64,
        #[serde(deserialize_with = "digest")]
        sha256: String,
    },
}

impl Reference {
    pub fn text(part: Option<String>, text: &str) -> Self {
        let mut end = text.len().min(TEXT_HEAD_MAX);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Self::TextRef {
            part,
            byte_size: text.len() as u64,
            sha256: sha256_hex(text.as_bytes()),
            char_count: text.chars().count() as u64,
            head: text[..end].into(),
        }
    }

    pub fn media(
        part: Option<String>,
        kind: impl Into<String>,
        mime: impl Into<String>,
        bytes: &[u8],
        dimensions: Option<(u64, u64)>,
    ) -> Self {
        Self::MediaRef {
            part,
            kind: kind.into(),
            mime: mime.into(),
            byte_size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            width: dimensions.map(|d| d.0),
            height: dimensions.map(|d| d.1),
        }
    }

    pub fn arguments(part: Option<String>, name: impl Into<String>, bytes: &[u8]) -> Self {
        Self::ArgsRef {
            part,
            byte_size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            name: name.into(),
        }
    }

    pub fn reasoning(part: Option<String>, bytes: &[u8]) -> Self {
        Self::ReasoningRef {
            part,
            byte_size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        }
    }

    pub fn byte_size(&self) -> u64 {
        match self {
            Self::MediaRef { byte_size, .. }
            | Self::TextRef { byte_size, .. }
            | Self::ArgsRef { byte_size, .. }
            | Self::ReasoningRef { byte_size, .. } => *byte_size,
        }
    }
}

/// Validate refs wherever the runner embeds them, without interpreting its
/// opaque surrounding schema. Non-ref objects remain forward-compatible.
pub fn validate_references(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            if map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|s| s.ends_with("_ref"))
            {
                let reference: Reference =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                if let Reference::TextRef {
                    head,
                    byte_size,
                    char_count,
                    ..
                } = &reference
                {
                    if head.len() > TEXT_HEAD_MAX
                        || head.len() as u64 > *byte_size
                        || head.chars().count() as u64 > *char_count
                    {
                        return Err("invalid text_ref head".into());
                    }
                }
            } else {
                for child in map.values() {
                    validate_references(child)?;
                }
            }
        }
        Value::Array(array) => {
            for child in array {
                validate_references(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn deserialize_content<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
    let value = Value::deserialize(d)?;
    validate_references(&value).map_err(serde::de::Error::custom)?;
    Ok(value)
}

/// One payload location supplied by the runner's schema adapter. A JSON
/// pointer targets only the payload, or the whole block for a media block.
/// `required` is true for media with bytes: those are always described.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Payload {
    pub pointer: String,
    pub reference: Reference,
    pub required: bool,
}

/// Always replace media and payloads larger than PART_INLINE_MAX. If needed,
/// replace smaller payloads largest first (stable ties preserve part order).
/// `budget` is DESCRIPTOR_MESSAGE_MAX minus this message's envelope size,
/// never a page cap: the same message must always give the same refs.
pub fn describe(inline: &Value, payloads: &[Payload], budget: usize) -> Result<Value, String> {
    let mut answer = inline.clone();
    let mut size = serde_json::to_vec(&answer).expect("JSON").len();
    let mut optional = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        if payload.required || payload.reference.byte_size() > PART_INLINE_MAX as u64 {
            let (old_size, new_size) = replace(&mut answer, payload)?;
            size = size - old_size + new_size;
        } else {
            optional.push(index);
        }
    }
    optional.sort_by_key(|i| std::cmp::Reverse(payloads[*i].reference.byte_size()));
    for index in optional {
        if size <= budget {
            break;
        }
        let (old_size, new_size) = replace(&mut answer, &payloads[index])?;
        size = size - old_size + new_size;
    }
    let final_size = serde_json::to_vec(&answer).expect("JSON").len();
    debug_assert_eq!(size, final_size);
    if final_size > budget {
        return Err("non-payload structure exceeds descriptor message budget".into());
    }
    Ok(answer)
}

/// Compact JSON embeds each value unchanged, so only the replaced value's
/// encoded length changes; surrounding keys and delimiters stay the same.
fn replace(answer: &mut Value, payload: &Payload) -> Result<(usize, usize), String> {
    let target = answer
        .pointer_mut(&payload.pointer)
        .ok_or_else(|| format!("unknown payload pointer {}", payload.pointer))?;
    let old_size = serde_json::to_vec(target).expect("JSON").len();
    let replacement = serde_json::to_value(&payload.reference).expect("serializes");
    let new_size = serde_json::to_vec(&replacement).expect("JSON").len();
    *target = replacement;
    Ok((old_size, new_size))
}

impl super::ReadMessage {
    /// Provider-side descriptor validation without deriving opaque part keys.
    /// `profile` is the runner's serializer profile. Count the message's parts,
    /// validate digests and unique keys, and require exactly the keys served in
    /// this entry's complete `tags` rows, in the same order. Nonempty messages
    /// must carry tags rows even for parts with no tag number. Runners also use
    /// [`Self::check_descriptors`] with their independently known part keys.
    pub fn check_descriptor_rows(&self, profile: &str) -> Result<(), String> {
        self.check_descriptor_digests()?;
        let rows = self.part_digests.as_ref().expect("checked above");
        let mut keys = std::collections::BTreeSet::new();
        for row in rows {
            if !keys.insert(&row.part) {
                return Err(format!("duplicate part_digests key: {}", row.part));
            }
        }
        let count = crate::serializer_profile::top_level_part_count(profile, &self.message)?;
        if count > 0 && self.tags.is_empty() {
            return Err("descriptors read missing tags rows".into());
        }
        if rows.len() != count {
            return Err(format!(
                "part_digests row count {} does not match top-level part count {count}",
                rows.len()
            ));
        }
        if rows
            .iter()
            .map(|row| &row.part)
            .ne(self.tags.iter().map(|tag| &tag.part))
        {
            return Err("part_digests keys do not match tags rows in order".into());
        }
        Ok(())
    }

    /// Runner-side check against the adapter's independently known top-level
    /// part keys, in message order. Keys are runner-defined, not a profile
    /// convention, and must not be copied from the digest rows being checked.
    /// Providers use [`Self::check_descriptor_rows`] instead.
    pub fn check_descriptors(&self, keys: &[String]) -> Result<(), String> {
        self.check_descriptor_digests()?;
        let rows = self.part_digests.as_ref().expect("checked above");
        if rows.iter().map(|p| &p.part).ne(keys.iter())
            || keys.iter().collect::<std::collections::BTreeSet<_>>().len() != keys.len()
        {
            return Err("part_digests keys do not match message parts in order".into());
        }
        Ok(())
    }

    fn check_descriptor_digests(&self) -> Result<(), String> {
        validate_references(&self.message)?;
        if self.digest.as_deref().is_none_or(|d| !is_sha256_hex(d)) {
            return Err("descriptors read missing message digest".into());
        }
        let rows = self
            .part_digests
            .as_ref()
            .ok_or("descriptors read missing part_digests")?;
        for row in rows {
            if !is_sha256_hex(&row.digest) {
                return Err(format!("invalid part_digests digest for key: {}", row.part));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod row_tests {
    use super::*;
    use crate::{read::ReadMessage, serializer_profile::ANTHROPIC_MESSAGES, tags::PartTag};
    use serde_json::json;

    fn message() -> ReadMessage {
        let body = json!({"role":"assistant","content":[
            {"type":"text","text":"before"},
            {"type":"tool_use","id":"call-1","name":"echo","input":{}},
            {"type":"text","text":"after"}
        ]});
        let mut read = ReadMessage::new(0, "m0", body.clone()).with_tags(vec![
            PartTag::tagged("token-z", "§1§"),
            PartTag::untagged("token-q"),
            PartTag::tagged("token-a", "§2§"),
        ]);
        read.digest = Some(content_digest(&body));
        read.part_digests = Some(vec![
            PartDigest::new("token-z", &body["content"][0]),
            PartDigest::new("token-q", &body["content"][1]),
            PartDigest::new("token-a", &body["content"][2]),
        ]);
        read
    }

    #[test]
    fn descriptor_rows_accept_opaque_keys_and_untagged_tool_call() {
        let read = message();
        assert_eq!(read.check_descriptor_rows(ANTHROPIC_MESSAGES), Ok(()));
    }

    #[test]
    fn descriptor_rows_require_tags_rows() {
        assert_eq!(
            message()
                .with_tags(vec![])
                .check_descriptor_rows(ANTHROPIC_MESSAGES)
                .unwrap_err(),
            "descriptors read missing tags rows"
        );
    }

    #[test]
    fn descriptor_rows_reject_omitted_untagged_tool_call_row() {
        let mut read = message();
        read.tags.remove(1);
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "part_digests keys do not match tags rows in order"
        );
    }

    #[test]
    fn descriptor_rows_accept_empty_part_containers() {
        let body = json!({"role":"assistant","content":[]});
        let mut read = ReadMessage::new(0, "empty", body.clone());
        read.digest = Some(content_digest(&body));
        read.part_digests = Some(vec![]);
        assert_eq!(read.check_descriptor_rows(ANTHROPIC_MESSAGES), Ok(()));
    }

    #[test]
    fn descriptor_rows_count_opencode_parts_without_using_part_ids() {
        let body = json!({"info":{"role":"assistant"},"parts":[
            {"id":"native-id","type":"tool","state":{"attachments":[{},{}]}},
            {"id":"bookkeeping-id","type":"step-start"}
        ]});
        let mut read = ReadMessage::new(0, "m0", body.clone()).with_tags(vec![
            PartTag::untagged("opaque-tool"),
            PartTag::untagged("opaque-step"),
        ]);
        read.digest = Some(content_digest(&body));
        read.part_digests = Some(vec![
            PartDigest::new("opaque-tool", &body["parts"][0]),
            PartDigest::new("opaque-step", &body["parts"][1]),
        ]);
        assert_eq!(
            read.check_descriptor_rows(crate::serializer_profile::OPENCODE_AISDK),
            Ok(())
        );
    }

    #[test]
    fn descriptor_rows_require_message_digest() {
        let mut read = message();
        read.digest = None;
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "descriptors read missing message digest"
        );
    }

    #[test]
    fn descriptor_rows_reject_malformed_message_digest() {
        for digest in [
            "a".repeat(63),
            "A".repeat(64),
            "g".repeat(64),
            "a".repeat(65),
        ] {
            let mut read = message();
            read.digest = Some(digest);
            assert_eq!(
                read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
                "descriptors read missing message digest"
            );
        }
    }

    #[test]
    fn descriptor_rows_require_part_digests() {
        let mut read = message();
        read.part_digests = None;
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "descriptors read missing part_digests"
        );
    }

    #[test]
    fn descriptor_rows_reject_malformed_part_digest() {
        for digest in [
            "a".repeat(63),
            "A".repeat(64),
            "g".repeat(64),
            "a".repeat(65),
        ] {
            let mut read = message();
            read.part_digests.as_mut().unwrap()[1].digest = digest;
            assert_eq!(
                read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
                "invalid part_digests digest for key: token-q"
            );
        }
    }

    #[test]
    fn descriptor_rows_reject_duplicate_key() {
        let mut read = message();
        read.part_digests.as_mut().unwrap()[1].part = "token-z".into();
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "duplicate part_digests key: token-z"
        );
    }

    #[test]
    fn descriptor_rows_reject_missing_row() {
        let mut read = message();
        read.part_digests.as_mut().unwrap().remove(1);
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "part_digests row count 2 does not match top-level part count 3"
        );
    }

    #[test]
    fn descriptor_rows_reject_extra_row() {
        let mut read = message();
        read.part_digests
            .as_mut()
            .unwrap()
            .push(PartDigest::new("extra", &json!(null)));
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "part_digests row count 4 does not match top-level part count 3"
        );
    }

    #[test]
    fn descriptor_rows_reject_tag_key_absent_from_rows() {
        let mut read = message();
        read.tags[0].part = "absent".into();
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "part_digests keys do not match tags rows in order"
        );
    }

    #[test]
    fn descriptor_rows_reject_reordered_row() {
        let mut read = message();
        read.part_digests.as_mut().unwrap().swap(0, 2);
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "part_digests keys do not match tags rows in order"
        );
    }

    #[test]
    fn descriptor_rows_reject_reordered_or_repeated_tag() {
        let mut read = message();
        read.tags.swap(0, 1);
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "part_digests keys do not match tags rows in order"
        );
        read.tags[1] = read.tags[0].clone();
        assert_eq!(
            read.check_descriptor_rows(ANTHROPIC_MESSAGES).unwrap_err(),
            "part_digests keys do not match tags rows in order"
        );
    }

    #[test]
    fn descriptor_rows_refuse_unknown_profile_by_name() {
        assert_eq!(
            message()
                .check_descriptor_rows("future-profile")
                .unwrap_err(),
            "unknown serializer profile: future-profile"
        );
    }

    #[test]
    fn runner_check_detects_reorder_when_tags_and_rows_both_reordered() {
        let mut read = message();
        let keys = vec!["token-z".into(), "token-q".into(), "token-a".into()];
        assert_eq!(read.check_descriptors(&keys), Ok(()));
        read.part_digests.as_mut().unwrap().swap(0, 1);
        read.tags.swap(0, 1);
        assert_eq!(read.check_descriptor_rows(ANTHROPIC_MESSAGES), Ok(()));
        assert_eq!(
            read.check_descriptors(&keys).unwrap_err(),
            "part_digests keys do not match message parts in order"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anthropic_descriptor_vectors_preserve_structure_and_inline_identity() {
        let file = crate::vectors::load("read-descriptors.json");
        for row in crate::vectors::cases(&file, "cases") {
            let name = row["name"].as_str().unwrap();
            let mut inline = row["inline"].clone();
            for expand in row["expand"].as_array().unwrap() {
                let text = format!(
                    "{}{}{}",
                    expand["before"].as_str().unwrap(),
                    expand["unit"]
                        .as_str()
                        .unwrap()
                        .repeat(expand["count"].as_u64().unwrap() as usize),
                    expand["after"].as_str().unwrap()
                );
                *inline
                    .pointer_mut(expand["pointer"].as_str().unwrap())
                    .unwrap() = json!(text);
            }
            let payloads: Vec<Payload> = serde_json::from_value(row["payloads"].clone()).unwrap();
            let described = describe(&inline, &payloads, DESCRIPTOR_MESSAGE_MAX).unwrap();
            assert_eq!(described, row["expected"], "{name}");
            validate_references(&described).expect(name);
            assert_eq!(content_digest(&inline), row["digest"], "{name}");
            let parts = inline["content"].as_array().unwrap();
            for (index, part) in parts.iter().enumerate() {
                assert_eq!(
                    content_digest(part),
                    row["part_digests"][index],
                    "{name}: part {index}"
                );
            }
            let mut read = super::super::ReadMessage::new(0, "m0", described);
            read.digest = Some(content_digest(&inline));
            read.part_digests = Some(
                parts
                    .iter()
                    .enumerate()
                    .map(|(i, p)| PartDigest::new(i.to_string(), p))
                    .collect(),
            );
            let keys = (0..parts.len()).map(|i| i.to_string()).collect::<Vec<_>>();
            let decoded: super::super::ReadMessage =
                serde_json::from_value(serde_json::to_value(&read).unwrap()).expect(name);
            decoded.check_descriptors(&keys).expect(name);
        }
    }

    #[test]
    fn fallback_describes_eight_equal_payloads_in_part_order_and_is_stable() {
        let file = crate::vectors::load("read-descriptors.json");
        let vector = &file["fallback"];
        assert_eq!(vector["parts"], 20);
        assert_eq!(vector["bytes_per_part"], 250 * 1024);
        let inline =
            json!({"content": vec![json!({"type":"text","text":"x".repeat(250 * 1024)});20]});
        let payloads: Vec<_> = (0..20)
            .map(|i| Payload {
                pointer: format!("/content/{i}"),
                reference: Reference::text(Some(i.to_string()), &"x".repeat(250 * 1024)),
                required: false,
            })
            .collect();
        let first = describe(&inline, &payloads, DESCRIPTOR_MESSAGE_MAX).unwrap();
        assert_eq!(
            first,
            describe(&inline, &payloads, DESCRIPTOR_MESSAGE_MAX).unwrap()
        );
        for (i, part) in first["content"].as_array().unwrap().iter().enumerate() {
            assert_eq!(
                part["type"],
                if vector["described_parts"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(i))
                {
                    "text_ref"
                } else {
                    "text"
                },
                "part {i}"
            );
        }
    }

    #[test]
    fn mixed_size_fallback_is_largest_first_and_breaks_ties_by_part_order() {
        let file = crate::vectors::load("read-descriptors.json");
        let vector = &file["mixed_fallback"];
        let texts: Vec<_> = vector["byte_sizes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, size)| {
                let prefix = format!("{}{:02}:", vector["prefix"].as_str().unwrap(), i);
                format!(
                    "{prefix}{}",
                    "x".repeat(size.as_u64().unwrap() as usize - prefix.len())
                )
            })
            .collect();
        let inline = json!({"content":texts.iter().map(|text| json!({"type":"text","text":text})).collect::<Vec<_>>()});
        let payloads: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(i, text)| Payload {
                pointer: format!("/content/{i}"),
                reference: Reference::text(Some(i.to_string()), text),
                required: false,
            })
            .collect();
        let described = describe(&inline, &payloads, DESCRIPTOR_MESSAGE_MAX).unwrap();
        let selected: Vec<_> = described["content"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter_map(|(i, part)| (part["type"] == "text_ref").then_some(i))
            .collect();
        assert_eq!(
            json!(selected),
            vector["described_parts"],
            "largest-first fallback must describe exactly the recorded part set"
        );
        assert_eq!(
            described["content"][5]["type"], "text",
            "the later tied part stays inline"
        );
    }

    #[test]
    fn replacement_sizes_account_for_json_escaping_and_utf8_exactly() {
        let text = "\"\\\n🦀".repeat(12_000);
        let inline =
            json!({"content":[{"type":"text","text":text},{"type":"text","text":"still inline"}]});
        let payload = Payload {
            pointer: "/content/0".into(),
            reference: Reference::text(Some("0".into()), &text),
            required: false,
        };
        let mut expected = inline.clone();
        expected["content"][0] = serde_json::to_value(&payload.reference).unwrap();
        let budget = serde_json::to_vec(&expected).unwrap().len();
        assert!(serde_json::to_vec(&inline).unwrap().len() > budget);
        assert_eq!(describe(&inline, &[payload], budget).unwrap(), expected);
    }

    #[test]
    fn non_payload_structure_over_the_budget_is_the_only_residual_refusal() {
        let inline = json!({"metadata":"x".repeat(DESCRIPTOR_MESSAGE_MAX)});
        assert_eq!(
            describe(&inline, &[], DESCRIPTOR_MESSAGE_MAX).unwrap_err(),
            "non-payload structure exceeds descriptor message budget"
        );
    }

    #[test]
    fn resolve_vector_counts_the_resolved_part() {
        let file = crate::vectors::load("read-descriptors.json");
        let part = &file["resolve"]["reply"]["content"];
        let kind = match part["type"].as_str() {
            Some("tool_result") => {
                let content = part["content"].as_array().expect("tool-result content");
                let blocks: Vec<_> = content
                    .iter()
                    .filter_map(|block| match block["type"].as_str() {
                        Some("text") => block["text"]
                            .as_str()
                            .map(crate::tags::ToolResultBlock::Text),
                        Some("media_ref") if block["kind"] == "image" => {
                            Some(crate::tags::ToolResultBlock::Image)
                        }
                        Some("media_ref") => Some(crate::tags::ToolResultBlock::Media),
                        _ => None,
                    })
                    .collect();
                crate::tags::tool_result_kind(&blocks)
            }
            Some("media_ref") if part["kind"] == "image" => crate::tags::PartKind::UserAttachment,
            _ => panic!("resolve vector must describe a recognizable transcript part"),
        };
        assert!(
            crate::tags::counts(kind),
            "resolve vector part kind {kind:?} is not counted by §9.6"
        );
    }

    #[test]
    fn reference_decoding_is_strict_and_digest_keys_must_match_the_parts() {
        let file = crate::vectors::load("read-descriptors.json");
        let request: crate::tags::ResolveRequest =
            serde_json::from_value(file["resolve"]["request"].clone()).unwrap();
        assert_eq!(request.media, Some(MediaMode::Descriptors));
        let reply: crate::tags::ResolveReply =
            serde_json::from_value(file["resolve"]["reply"].clone()).unwrap();
        assert_eq!(reply.content["type"], "tool_result");
        assert_eq!(reply.content["tool_use_id"], "c1");
        assert_eq!(reply.content["is_error"], true);
        assert_eq!(reply.content["content"][0]["type"], "media_ref");
        assert!(reply.content["content"][0].get("part").is_none());
        assert_eq!(reply.content, file["cases"][1]["expected"]["content"][0]);
        let refused: super::super::ReadRequest =
            serde_json::from_value(file["refused"]["request"].clone()).unwrap();
        assert_eq!(
            refused.mode().unwrap_err(),
            file["refused"]["field"].as_str().unwrap()
        );
        let valid = serde_json::to_value(Reference::text(Some("p0".into()), "hello")).unwrap();
        for (key, value) in [
            ("sha256", json!("bad")),
            ("extra", json!(true)),
            ("byte_size", json!(-1)),
            ("char_count", json!("five")),
        ] {
            let mut bad = valid.clone();
            bad[key] = value;
            assert!(serde_json::from_value::<Reference>(bad).is_err(), "{key}");
        }
        let mut read = super::super::ReadMessage::new(0, "m", json!({"content":[valid]}));
        read.digest = Some("0".repeat(64));
        read.part_digests = Some(vec![PartDigest {
            part: "not-p0".into(),
            digest: "0".repeat(64),
        }]);
        assert!(read
            .check_descriptors(&["p0".into()])
            .unwrap_err()
            .contains("keys"));
        for media in [json!("bogus"), json!(null), json!(3)] {
            let value = json!({"media":media});
            assert!(serde_json::from_value::<super::super::ReadRequest>(value)
                .unwrap_err()
                .to_string()
                .contains("media"));
            assert!(serde_json::from_value::<crate::tags::ResolveRequest>(
                json!({"tag":"§1§","media":media})
            )
            .unwrap_err()
            .to_string()
            .contains("media"));
        }
    }
}
