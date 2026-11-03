//! The declared exception to byte-stable reads of an archived session.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::serializer_profile::{ANTHROPIC_MESSAGES, OPENCODE_AISDK, OWNED_BROCA, PI};

pub mod kinds {
    pub const REASONING: &str = "reasoning";
    pub const OPAQUE_REASONING: &str = "opaque_reasoning";
    pub const SIGNATURE: &str = "signature";
    pub const ALL: &[&str] = &[REASONING, OPAQUE_REASONING, SIGNATURE];
}

/// Settings required when `archive_prunes_reasoning` is declared.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ArchivePrunesReasoning {
    pub kinds: Vec<String>,
}

impl ArchivePrunesReasoning {
    pub fn new(kinds: Vec<String>) -> Self {
        Self { kinds }
    }

    /// Declarations list at least one known kind, with no duplicates.
    pub fn valid(&self) -> bool {
        !self.kinds.is_empty()
            && self.kinds.iter().enumerate().all(|(index, kind)| {
                kinds::ALL.contains(&kind.as_str()) && !self.kinds[..index].contains(kind)
            })
    }
}

/// One pruned part served on a page, repeated on every read of that part.
/// Identity and the digest describe the original write, not the empty payload.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct PrunedReasoning {
    pub mid: String,
    pub part: String,
    pub kind: String,
    /// Length of the original part's RFC 8785 (JCS) encoding.
    pub bytes: u64,
    /// The original part's write-time `part_digests` digest.
    pub sha256: String,
}

/// A session's prune state, on head and read pages of a runner declaring
/// `archive_prunes_reasoning`. Absent until the session's first prune, so
/// "never pruned" needs no stored value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReasoningPruned {
    /// 1 after the first prune, raised by one in the same commit as every
    /// later prune. Never 0.
    pub generation: u64,
    /// The highest ordinal any prune of the session has touched.
    pub through_ordinal: u64,
    /// When the latest prune committed, in milliseconds since the Unix
    /// epoch. Display only: no party compares it with its own clock.
    pub at_ms: u64,
}

impl ReasoningPruned {
    pub fn new(generation: u64, through_ordinal: u64, at_ms: u64) -> Self {
        Self {
            generation,
            through_ordinal,
            at_ms,
        }
    }

    /// Generations and touched entry ordinals both count from 1.
    pub fn valid(&self) -> bool {
        self.generation >= 1 && self.through_ordinal >= 1
    }
}

fn non_empty_string(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

/// Whether `part`, a message part in the stored formats CONTRACT §9.14.1
/// names, carries content that a prune of `kind` removes, so sending it
/// would put that content back. This is the `owned-broca` reference; for a
/// session of another serializer profile use [`carries_pruned_content_in`],
/// which reads that profile's own part shapes:
///
/// - `reasoning`: a `reasoning` part with non-empty `text` or any
///   `signature`, or a `redacted_reasoning` part with non-empty `data`;
/// - `opaque_reasoning`: a `reasoning` part with any `signature` (on Claude
///   the encrypted thinking, on Gemini the thought signature), or a
///   `redacted_reasoning` part whose item JSON in `data` still has
///   `encrypted_content`; readable text and summaries may be sent;
/// - `signature`: a non-reasoning part with a non-empty `signature` or
///   `thought_signature`, or a `provider_state {format, data}` whose
///   `format` is one of [`provider_state_formats::SIGNED`] and whose `data`
///   is non-empty. Any other `format`, the Responses item-state formats
///   included, never carries signature content.
///
/// A kind this crate does not define is `false`: a runner applies only the
/// kinds it declares, and checks each with its own rule.
pub fn carries_pruned_content(kind: &str, part: &Value) -> bool {
    let kind_of = part.get("type").and_then(Value::as_str).unwrap_or_default();
    match kind {
        kinds::REASONING => match kind_of {
            "reasoning" => non_empty_string(part.get("text")) || part.get("signature").is_some(),
            "redacted_reasoning" => non_empty_string(part.get("data")),
            _ => false,
        },
        kinds::OPAQUE_REASONING => match kind_of {
            "reasoning" => part.get("signature").is_some(),
            "redacted_reasoning" => part
                .get("data")
                .and_then(Value::as_str)
                .and_then(|data| serde_json::from_str::<Value>(data).ok())
                .is_some_and(|item| item.get("encrypted_content").is_some()),
            _ => false,
        },
        kinds::SIGNATURE => {
            !matches!(kind_of, "reasoning" | "redacted_reasoning")
                && (non_empty_string(part.get("signature"))
                    || non_empty_string(part.get("thought_signature"))
                    || provider_state_signed(part.get("provider_state")))
        }
        _ => false,
    }
}

/// The `format` names an `owned-broca` `provider_state {format, data}`
/// carries (CONTRACT §9.14.1). Only [`provider_state_formats::SIGNED`] formats hold a signature in
/// `data`; a format not listed here is not a signature, and a runner
/// classifies a new one in the contract before it prunes it.
pub mod provider_state_formats {
    /// `data` is the JSON string `{"thoughtSignature": ...}`.
    pub const GEMINI: &str = "gemini";
    /// `data` is the JSON string `{"thoughtSignature": ...}`.
    pub const GOOGLE_CODE_ASSIST: &str = "google_code_assist";
    /// `data` is the Responses message item's `{id, phase}`: item state,
    /// not a signature, kept byte-identical by every prune.
    pub const OPEN_AI_RESPONSES: &str = "open_ai_responses";
    /// As [`OPEN_AI_RESPONSES`].
    pub const OPEN_AI_RESPONSES_CHATGPT: &str = "open_ai_responses_chatgpt";
    /// The formats whose `data` a `signature` prune empties to `""`.
    pub const SIGNED: &[&str] = &[GEMINI, GOOGLE_CODE_ASSIST];
}

/// Whether a `provider_state {format, data}` holds a signature: a
/// [`provider_state_formats::SIGNED`] format with non-empty `data`.
fn provider_state_signed(state: Option<&Value>) -> bool {
    state.is_some_and(|state| {
        state
            .get("format")
            .and_then(Value::as_str)
            .is_some_and(|format| provider_state_formats::SIGNED.contains(&format))
            && non_empty_string(state.get("data"))
    })
}

/// A serializer profile the per-profile prune rules do not name. The
/// `_in` functions answer it instead of "nothing pruned", so a session of an
/// unknown profile is never checked by the wrong rule.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnsupportedProfile {
    pub profile: String,
}

impl UnsupportedProfile {
    pub fn new(profile: impl Into<String>) -> Self {
        Self {
            profile: profile.into(),
        }
    }
}

impl std::fmt::Display for UnsupportedProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no archive-pruning rules for serializer profile: {}",
            self.profile
        )
    }
}

impl std::error::Error for UnsupportedProfile {}

/// The kinds whose removed content `profile`'s stored format can carry
/// (CONTRACT §9.14.1, per-profile table). A kind outside the list never
/// matches a part of that profile: `anthropic-messages` has no signature on
/// a non-reasoning block, so it carries no `signature` content.
pub fn kinds_carried(profile: &str) -> Result<&'static [&'static str], UnsupportedProfile> {
    match profile {
        OWNED_BROCA | OPENCODE_AISDK | PI => Ok(kinds::ALL),
        ANTHROPIC_MESSAGES => Ok(&[kinds::REASONING, kinds::OPAQUE_REASONING]),
        other => Err(UnsupportedProfile::new(other)),
    }
}

/// The OpenCode provider entries this crate classifies (CONTRACT §9.14.1),
/// as `(entry key in metadata, signature members, opaque members)`. OpenCode
/// copies the AI SDK's `providerMetadata` into a part's `metadata`, keyed by
/// provider. An entry not listed here carries nothing pruned: a runner adds
/// it to the contract's table before relying on a prune to remove it.
/// OpenRouter's `reasoning_details` are read by [`openrouter_details_carry`].
const OPENCODE_ENTRIES: &[OpencodeEntry] = &[
    ("anthropic", &["signature"], &["redactedData"]),
    ("bedrock", &["signature"], &["redactedData"]),
    ("google", &["thoughtSignature"], &[]),
    ("vertex", &["thoughtSignature"], &[]),
    ("openai", &[], &["reasoningEncryptedContent"]),
];

/// One row of [`OPENCODE_ENTRIES`].
type OpencodeEntry = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

/// The signature members of a listed entry.
fn opencode_signatures(entry: &OpencodeEntry) -> &'static [&'static str] {
    entry.1
}

/// The opaque members of a listed entry.
fn opencode_opaque(entry: &OpencodeEntry) -> &'static [&'static str] {
    entry.2
}

/// Whether a listed provider entry of an OpenCode part's `metadata` holds a
/// non-empty string in one of the members `members` picks for it.
fn opencode_metadata_has(
    part: &Value,
    members: fn(&OpencodeEntry) -> &'static [&'static str],
) -> bool {
    let Some(metadata) = part.get("metadata").and_then(Value::as_object) else {
        return false;
    };
    OPENCODE_ENTRIES.iter().any(|listed| {
        metadata.get(listed.0).is_some_and(|entry| {
            members(listed)
                .iter()
                .any(|member| non_empty_string(entry.get(*member)))
        })
    })
}

/// Whether an OpenCode part's `metadata.openrouter.reasoning_details`
/// carries content a prune of `kind` removes. Each detail is
/// `reasoning.text {text, signature}`, `reasoning.encrypted {data}` or
/// `reasoning.summary {summary}`: `reasoning` covers any non-empty `text`,
/// `summary`, `signature` or `data`; `opaque_reasoning` the `data` of a
/// `reasoning.encrypted` detail; `signature` the `signature` of a
/// `reasoning.text` detail.
fn openrouter_details_carry(kind: &str, part: &Value) -> bool {
    let details = part
        .pointer("/metadata/openrouter/reasoning_details")
        .and_then(Value::as_array);
    details.into_iter().flatten().any(|detail| {
        let detail_type = detail.get("type").and_then(Value::as_str);
        match kind {
            kinds::REASONING => ["text", "summary", "signature", "data"]
                .iter()
                .any(|member| non_empty_string(detail.get(*member))),
            kinds::OPAQUE_REASONING => {
                detail_type == Some("reasoning.encrypted") && non_empty_string(detail.get("data"))
            }
            kinds::SIGNATURE => {
                detail_type == Some("reasoning.text") && non_empty_string(detail.get("signature"))
            }
            _ => false,
        }
    })
}

/// Whether a `pi` `textSignature` is OpenAI Responses item state rather than
/// a signature: the `TextSignatureV1` JSON `{"v": 1, "id": ..., "phase"?}`
/// pi encodes from a Responses message item. A Google or Vertex
/// `textSignature` is a retained thought signature and does not parse so.
fn pi_text_signature_is_item_state(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|item| {
            item.get("v") == Some(&Value::from(1)) && item.get("id").is_some_and(Value::is_string)
        })
}

/// Whether a string holds JSON for an object with `encrypted_content`: an
/// OpenAI Responses reasoning item, serialized whole.
fn json_item_has_encrypted_content(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|item| item.get("encrypted_content").is_some())
}

/// [`carries_pruned_content`] for a part of a session whose stored bodies
/// follow serializer profile `profile` (CONTRACT §9.14.1, per-profile
/// table). `owned-broca` is exactly [`carries_pruned_content`]. Other
/// profiles read their own part types and members:
///
/// - `anthropic-messages`: `reasoning` is a `thinking` block with non-empty
///   `thinking` or `signature`, or a `redacted_thinking` block with
///   non-empty `data`; `opaque_reasoning` is a `thinking` block with a
///   non-empty `signature` (the encrypted full reasoning; its `thinking`
///   text is the readable summary) or a `redacted_thinking` block with
///   non-empty `data`; nothing carries `signature` content, a `thinking`
///   block's signature included.
/// - `opencode-aisdk`: `reasoning` is a `reasoning` part with non-empty
///   `text`, a listed provider entry of `metadata` holding a non-empty
///   signature or opaque member (the per-profile table), or OpenRouter
///   `reasoning_details` with any payload; `opaque_reasoning` the opaque
///   members and OpenRouter `reasoning.encrypted` data on a `reasoning`
///   part; `signature` a listed signature member on any other part, or an
///   OpenRouter `reasoning.text` detail's `signature`. An unlisted provider
///   entry carries nothing.
/// - `pi`: `reasoning` is a `thinking` part with non-empty `thinking` or
///   `thinkingSignature`; `opaque_reasoning` a `thinking` part whose
///   `thinkingSignature` is non-empty with `redacted: true`, or is JSON for
///   an item holding `encrypted_content`; `signature` a non-empty
///   `thoughtSignature` on any other part, or a non-empty `textSignature`
///   that is not OpenAI Responses item state (`TextSignatureV1` JSON).
///
/// A kind this crate does not define is `false`, as for
/// [`carries_pruned_content`]. A profile the rules do not name is
/// [`UnsupportedProfile`].
pub fn carries_pruned_content_in(
    profile: &str,
    kind: &str,
    part: &Value,
) -> Result<bool, UnsupportedProfile> {
    let kind_of = part.get("type").and_then(Value::as_str).unwrap_or_default();
    Ok(match profile {
        OWNED_BROCA => carries_pruned_content(kind, part),
        ANTHROPIC_MESSAGES => match (kind, kind_of) {
            (kinds::REASONING, "thinking") => {
                non_empty_string(part.get("thinking")) || non_empty_string(part.get("signature"))
            }
            (kinds::OPAQUE_REASONING, "thinking") => non_empty_string(part.get("signature")),
            (kinds::REASONING | kinds::OPAQUE_REASONING, "redacted_thinking") => {
                non_empty_string(part.get("data"))
            }
            _ => false,
        },
        OPENCODE_AISDK => match kind {
            kinds::REASONING => {
                kind_of == "reasoning"
                    && (non_empty_string(part.get("text"))
                        || opencode_metadata_has(part, opencode_signatures)
                        || opencode_metadata_has(part, opencode_opaque)
                        || openrouter_details_carry(kind, part))
            }
            kinds::OPAQUE_REASONING => {
                kind_of == "reasoning"
                    && (opencode_metadata_has(part, opencode_opaque)
                        || openrouter_details_carry(kind, part))
            }
            // A signature on a non-reasoning part, or on an OpenRouter
            // `reasoning.text` detail, which signs the detail rather than
            // being the reasoning itself.
            kinds::SIGNATURE => {
                (kind_of != "reasoning" && opencode_metadata_has(part, opencode_signatures))
                    || (kind_of == "reasoning" && openrouter_details_carry(kind, part))
            }
            _ => false,
        },
        PI => match kind {
            kinds::REASONING => {
                kind_of == "thinking"
                    && (non_empty_string(part.get("thinking"))
                        || non_empty_string(part.get("thinkingSignature")))
            }
            kinds::OPAQUE_REASONING => {
                let signature = part.get("thinkingSignature");
                kind_of == "thinking"
                    && ((part.get("redacted") == Some(&Value::Bool(true))
                        && non_empty_string(signature))
                        || json_item_has_encrypted_content(signature))
            }
            kinds::SIGNATURE => {
                let text_signature = part.get("textSignature");
                kind_of != "thinking"
                    && ((non_empty_string(text_signature)
                        && !pi_text_signature_is_item_state(text_signature))
                        || non_empty_string(part.get("thoughtSignature")))
            }
            _ => false,
        },
        other => return Err(UnsupportedProfile::new(other)),
    })
}

/// Why a native part cannot be pruned under the declared profile rule.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PruneRefused {
    /// The per-profile rules do not name the profile.
    UnsupportedProfile(UnsupportedProfile),
    /// A kind this crate does not define, so there is no rule to apply.
    UnknownKind(String),
    /// The part carries content the kind removes, in a form the profile's
    /// rule neither empties nor removes: applying the rule would leave that
    /// content in place. A runner refuses to prune a message whose applied
    /// part carries content the declared native rule cannot remove.
    Inexpressible { profile: String, kind: String },
}

impl From<UnsupportedProfile> for PruneRefused {
    fn from(unsupported: UnsupportedProfile) -> Self {
        Self::UnsupportedProfile(unsupported)
    }
}

impl std::fmt::Display for PruneRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedProfile(unsupported) => unsupported.fmt(f),
            Self::UnknownKind(kind) => write!(f, "no archive-pruning rule for kind: {kind}"),
            Self::Inexpressible { profile, kind } => write!(
                f,
                "the {profile} rule for {kind} cannot remove what this part carries"
            ),
        }
    }
}

impl std::error::Error for PruneRefused {}

/// Set a string member that is present to `""`; an absent member stays
/// absent, because a prune never adds a member.
fn empty_member(part: &mut serde_json::Map<String, Value>, member: &str) {
    if part.get(member).is_some_and(Value::is_string) {
        part.insert(member.into(), Value::from(""));
    }
}

/// Rewrite a string member holding JSON for an object with
/// `encrypted_content` as the same object without it: an OpenAI Responses
/// reasoning item keeps its summary and every other member.
fn strip_encrypted_item(part: &mut serde_json::Map<String, Value>, member: &str) {
    let Some(mut item) = part
        .get(member)
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
    else {
        return;
    };
    if let Some(object) = item.as_object_mut() {
        if object.remove("encrypted_content").is_some() {
            part.insert(member.into(), Value::from(item.to_string()));
        }
    }
}

/// Remove the members `members` picks from every listed provider entry of
/// an OpenCode part's `metadata`. The entry itself stays, possibly empty.
fn opencode_metadata_remove(
    part: &mut serde_json::Map<String, Value>,
    members: fn(&OpencodeEntry) -> &'static [&'static str],
) {
    let Some(metadata) = part.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    for listed in OPENCODE_ENTRIES {
        if let Some(entry) = metadata.get_mut(listed.0).and_then(Value::as_object_mut) {
            for member in members(listed) {
                entry.remove(*member);
            }
        }
    }
}

/// Apply `kind` to each OpenRouter `reasoning_details` entry of an
/// OpenCode part, as [`openrouter_details_carry`] reads them: `reasoning`
/// empties `text` and `summary` and removes `signature` and `data`;
/// `opaque_reasoning` removes a `reasoning.encrypted` detail's `data`;
/// `signature` removes a `reasoning.text` detail's `signature`.
fn openrouter_details_prune(part: &mut serde_json::Map<String, Value>, kind: &str) {
    let Some(details) = part
        .get_mut("metadata")
        .and_then(|metadata| metadata.pointer_mut("/openrouter/reasoning_details"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for detail in details.iter_mut().filter_map(Value::as_object_mut) {
        let detail_type = detail
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match kind {
            kinds::REASONING => {
                empty_member(detail, "text");
                empty_member(detail, "summary");
                detail.remove("signature");
                detail.remove("data");
            }
            kinds::OPAQUE_REASONING if detail_type.as_deref() == Some("reasoning.encrypted") => {
                detail.remove("data");
            }
            kinds::SIGNATURE if detail_type.as_deref() == Some("reasoning.text") => {
                detail.remove("signature");
            }
            _ => {}
        }
    }
}

/// `part` as a prune of `kind` leaves it under serializer profile `profile`
/// (CONTRACT §9.14.1, per-profile table): the bytes a runner stores in
/// place of a pruned part. Required
/// members are emptied to `""`, optional members removed; no member is
/// added and the part keeps its type. A part that carries nothing the kind
/// removes ([`carries_pruned_content_in`]) comes back unchanged, so a kind a
/// profile does not carry changes nothing.
///
/// [`PruneRefused::Inexpressible`] when the rule would leave content of the
/// kind in the part: an `owned-broca` non-reasoning part with a `signature`
/// member, which the profile's table does not name, is one. A profile the
/// rules do not name is [`PruneRefused::UnsupportedProfile`], a kind this
/// crate does not define [`PruneRefused::UnknownKind`].
pub fn prune_part_in(profile: &str, kind: &str, part: &Value) -> Result<Value, PruneRefused> {
    kinds_carried(profile)?;
    if !kinds::ALL.contains(&kind) {
        return Err(PruneRefused::UnknownKind(kind.into()));
    }
    if !carries_pruned_content_in(profile, kind, part)? {
        return Ok(part.clone());
    }
    let mut pruned = part.clone();
    let kind_of = part.get("type").and_then(Value::as_str).unwrap_or_default();
    if let Some(object) = pruned.as_object_mut() {
        match (profile, kind, kind_of) {
            (OWNED_BROCA, kinds::REASONING, "reasoning") => {
                empty_member(object, "text");
                object.remove("signature");
            }
            (OWNED_BROCA, kinds::REASONING, "redacted_reasoning") => empty_member(object, "data"),
            (OWNED_BROCA, kinds::OPAQUE_REASONING, "reasoning") => {
                object.remove("signature");
            }
            (OWNED_BROCA, kinds::OPAQUE_REASONING, "redacted_reasoning") => {
                strip_encrypted_item(object, "data")
            }
            (OWNED_BROCA, kinds::SIGNATURE, _) => {
                object.remove("thought_signature");
                if provider_state_signed(object.get("provider_state")) {
                    if let Some(state) = object
                        .get_mut("provider_state")
                        .and_then(Value::as_object_mut)
                    {
                        empty_member(state, "data");
                    }
                }
            }
            (ANTHROPIC_MESSAGES, kinds::REASONING, "thinking") => {
                empty_member(object, "thinking");
                empty_member(object, "signature");
            }
            (ANTHROPIC_MESSAGES, kinds::OPAQUE_REASONING, "thinking") => {
                empty_member(object, "signature")
            }
            (ANTHROPIC_MESSAGES, _, "redacted_thinking") => empty_member(object, "data"),
            (OPENCODE_AISDK, kinds::REASONING, _) => {
                empty_member(object, "text");
                opencode_metadata_remove(object, opencode_signatures);
                opencode_metadata_remove(object, opencode_opaque);
                openrouter_details_prune(object, kind);
            }
            (OPENCODE_AISDK, kinds::OPAQUE_REASONING, _) => {
                opencode_metadata_remove(object, opencode_opaque);
                openrouter_details_prune(object, kind);
            }
            (OPENCODE_AISDK, kinds::SIGNATURE, "reasoning") => {
                openrouter_details_prune(object, kind)
            }
            (OPENCODE_AISDK, kinds::SIGNATURE, _) => {
                opencode_metadata_remove(object, opencode_signatures)
            }
            (PI, kinds::REASONING, _) => {
                empty_member(object, "thinking");
                object.remove("thinkingSignature");
            }
            (PI, kinds::OPAQUE_REASONING, _) => {
                if object.get("redacted") == Some(&Value::Bool(true)) {
                    object.remove("thinkingSignature");
                } else {
                    strip_encrypted_item(object, "thinkingSignature");
                }
            }
            (PI, kinds::SIGNATURE, _) => {
                object.remove("thoughtSignature");
                let text_signature = object.get("textSignature");
                if non_empty_string(text_signature)
                    && !pi_text_signature_is_item_state(text_signature)
                {
                    object.remove("textSignature");
                }
            }
            _ => {}
        }
    }
    if carries_pruned_content_in(profile, kind, &pruned)? {
        return Err(PruneRefused::Inexpressible {
            profile: profile.into(),
            kind: kind.into(),
        });
    }
    Ok(pruned)
}

impl PrunedReasoning {
    pub fn new(
        mid: impl Into<String>,
        part: impl Into<String>,
        kind: impl Into<String>,
        bytes: u64,
        sha256: impl Into<String>,
    ) -> Self {
        Self {
            mid: mid.into(),
            part: part.into(),
            kind: kind.into(),
            bytes,
            sha256: sha256.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content_digest::content_digest,
        describe::{check_describe, RoleDescribe},
        read::ReadPage,
    };
    use serde_json::{json, Value};

    #[test]
    fn archive_pruning_vectors_preserve_required_fields_and_write_time_digests() {
        let file = crate::vectors::load("archive-prunes-reasoning.json");
        for case in file["cases"].as_array().unwrap() {
            let page: ReadPage = serde_json::from_value(case["page"].clone()).unwrap();
            assert_eq!(serde_json::to_value(&page).unwrap(), case["page"]);
            let entry = &page.messages[0];
            let pruning = &page.pruned_reasoning[0];
            assert_eq!(pruning.sha256, content_digest(&case["original"]));
            assert_eq!(
                pruning.bytes,
                serde_jcs::to_vec(&case["original"]).unwrap().len() as u64
            );
            assert_eq!(
                entry.digest.as_ref().unwrap(),
                &content_digest(&case["original_message"])
            );
            assert_eq!(
                entry.part_digests.as_ref().unwrap()[0].digest,
                pruning.sha256
            );
            assert_eq!(entry.message["content"][0], case["served"]);
            assert_eq!(case["served"]["type"], case["original"]["type"]);
            for key in case["served"].as_object().unwrap().keys() {
                assert!(
                    case["original"].get(key).is_some(),
                    "no block member is added"
                );
            }
            match case["original"]["type"].as_str().unwrap() {
                "reasoning" => {
                    assert_eq!(case["served"]["text"], "");
                    assert!(case["served"].get("signature").is_none());
                }
                "redacted_reasoning" if case["kind"] == "reasoning" => {
                    assert_eq!(case["served"]["data"], "")
                }
                "redacted_reasoning" => {
                    let original: Value =
                        serde_json::from_str(case["original"]["data"].as_str().unwrap()).unwrap();
                    let served: Value =
                        serde_json::from_str(case["served"]["data"].as_str().unwrap()).unwrap();
                    assert_eq!(original["summary"], served["summary"]);
                    assert!(served.get("encrypted_content").is_none());
                }
                "tool_call" => {
                    assert_eq!(case["served"]["arguments"], case["original"]["arguments"]);
                    assert!(case["served"].get("thought_signature").is_none());
                }
                "text_with_state" => {
                    assert_eq!(case["served"]["text"], case["original"]["text"]);
                    assert_eq!(
                        case["served"]["provider_state"]["format"],
                        case["original"]["provider_state"]["format"]
                    );
                    assert_eq!(case["served"]["provider_state"]["data"], "");
                }
                other => panic!("unexpected vector {other}"),
            }
        }
    }

    /// No member is added: every member of `served` exists in `original`,
    /// checked recursively through nested objects. Array contents are not
    /// inspected; the vectors' parts nest no objects inside arrays that a
    /// prune changes.
    fn adds_nothing(original: &Value, served: &Value) -> bool {
        match (original, served) {
            (Value::Object(original), Value::Object(served)) => {
                served.iter().all(|(key, value)| {
                    original
                        .get(key)
                        .is_some_and(|before| adds_nothing(before, value))
                })
            }
            _ => true,
        }
    }

    #[test]
    fn per_profile_prunes_follow_each_profiles_own_part_shapes() {
        let file = crate::vectors::load("reasoning-pruned-profiles.json");
        let mut profiles_seen = Vec::new();
        for entry in file["profiles"].as_array().unwrap() {
            let profile = entry["profile"].as_str().unwrap();
            profiles_seen.push(profile.to_owned());
            let carried = kinds_carried(profile).unwrap();
            assert_eq!(
                serde_json::to_value(carried).unwrap(),
                entry["kinds_carried"],
                "{profile}"
            );
            for case in entry["prunes"].as_array().unwrap() {
                let name = format!("{profile}: {}", case["name"].as_str().unwrap());
                let kind = case["kind"].as_str().unwrap();
                let (original, served) = (&case["original"], &case["served"]);
                assert!(carried.contains(&kind), "{name}");
                assert_eq!(
                    carries_pruned_content_in(profile, kind, original),
                    Ok(true),
                    "{name}"
                );
                assert_eq!(
                    carries_pruned_content_in(profile, kind, served),
                    Ok(false),
                    "{name}"
                );
                assert_eq!(original["type"], served["type"], "{name}: same type");
                assert!(adds_nothing(original, served), "{name}: a member was added");
            }
            for case in entry["not_carried"].as_array().unwrap() {
                let name = format!("{profile}: {}", case["name"].as_str().unwrap());
                let kind = case["kind"].as_str().unwrap();
                assert_eq!(
                    carries_pruned_content_in(profile, kind, &case["part"]),
                    Ok(false),
                    "{name}"
                );
            }
        }
        profiles_seen.sort();
        let mut named = vec![
            crate::serializer_profile::ANTHROPIC_MESSAGES,
            crate::serializer_profile::OPENCODE_AISDK,
            crate::serializer_profile::OWNED_BROCA,
            crate::serializer_profile::PI,
        ];
        named.sort();
        assert_eq!(profiles_seen, named, "one vector entry per named profile");
    }

    #[test]
    fn anthropic_thinking_signature_is_not_a_signature_prune() {
        let thinking = json!({"type":"thinking","thinking":"Consider the blue door.","signature":"invented-signature"});
        let profile = crate::serializer_profile::ANTHROPIC_MESSAGES;
        assert_eq!(
            carries_pruned_content_in(profile, kinds::SIGNATURE, &thinking),
            Ok(false)
        );
        assert_eq!(
            carries_pruned_content_in(profile, kinds::REASONING, &thinking),
            Ok(true)
        );
    }

    #[test]
    fn prune_part_in_gives_each_profiles_served_form() {
        let file = crate::vectors::load("reasoning-pruned-profiles.json");
        for entry in file["profiles"].as_array().unwrap() {
            let profile = entry["profile"].as_str().unwrap();
            for case in entry["prunes"].as_array().unwrap() {
                let name = format!("{profile}: {}", case["name"].as_str().unwrap());
                let kind = case["kind"].as_str().unwrap();
                let (original, served) = (&case["original"], &case["served"]);
                assert_eq!(
                    prune_part_in(profile, kind, original).as_ref(),
                    Ok(served),
                    "{name}"
                );
                assert_eq!(
                    prune_part_in(profile, kind, served).as_ref(),
                    Ok(served),
                    "{name}: a pruned part prunes to itself"
                );
            }
            for case in entry["not_carried"].as_array().unwrap() {
                let name = format!("{profile}: {}", case["name"].as_str().unwrap());
                let kind = case["kind"].as_str().unwrap();
                assert_eq!(
                    prune_part_in(profile, kind, &case["part"]).as_ref(),
                    Ok(&case["part"]),
                    "{name}: a part carrying nothing stays as it is"
                );
            }
        }
        let pruning = crate::vectors::load("archive-prunes-reasoning.json");
        for case in pruning["cases"].as_array().unwrap() {
            let kind = case["kind"].as_str().unwrap();
            assert_eq!(
                prune_part_in(
                    crate::serializer_profile::OWNED_BROCA,
                    kind,
                    &case["original"]
                )
                .as_ref(),
                Ok(&case["served"]),
                "{kind}"
            );
        }
    }

    #[test]
    fn prune_part_in_refuses_what_its_rule_cannot_remove() {
        let profile = crate::serializer_profile::OWNED_BROCA;
        // The owned-broca table names no `signature` member on a
        // non-reasoning part, yet such a member is signature content.
        let text =
            json!({"type":"text","text":"Open the blue door.","signature":"invented-signature"});
        let inexpressible = PruneRefused::Inexpressible {
            profile: profile.into(),
            kind: kinds::SIGNATURE.into(),
        };
        assert_eq!(
            prune_part_in(profile, kinds::SIGNATURE, &text),
            Err(inexpressible.clone())
        );
        assert_eq!(
            inexpressible.to_string(),
            "the owned-broca rule for signature cannot remove what this part carries"
        );
        assert_eq!(
            prune_part_in(profile, "future-kind", &text),
            Err(PruneRefused::UnknownKind("future-kind".into()))
        );
        let file = crate::vectors::load("reasoning-pruned-profiles.json");
        for unsupported in file["unsupported"].as_array().unwrap() {
            let unsupported = unsupported.as_str().unwrap();
            let refused = PruneRefused::UnsupportedProfile(UnsupportedProfile::new(unsupported));
            assert_eq!(
                prune_part_in(unsupported, kinds::SIGNATURE, &text),
                Err(refused.clone())
            );
        }
    }

    #[test]
    fn an_unnamed_profile_is_unsupported_never_nothing_pruned() {
        let file = crate::vectors::load("reasoning-pruned-profiles.json");
        let part = json!({"type":"reasoning","text":"Consider the blue door."});
        for profile in file["unsupported"].as_array().unwrap() {
            let profile = profile.as_str().unwrap();
            let unsupported = UnsupportedProfile::new(profile);
            assert_eq!(kinds_carried(profile), Err(unsupported.clone()));
            assert_eq!(
                carries_pruned_content_in(profile, kinds::REASONING, &part),
                Err(unsupported.clone())
            );
            assert_eq!(
                unsupported.to_string(),
                format!("no archive-pruning rules for serializer profile: {profile}")
            );
        }
    }

    #[test]
    fn archive_pruning_describe_is_paired_and_strict() {
        let base = RoleDescribe::new(
            vec![crate::describe::Major::new(
                crate::PROVIDES,
                vec![
                    "role.describe".into(),
                    "session.baseline".into(),
                    "session.read".into(),
                ],
                "alpha",
            )],
            "example",
        )
        .with_session_capabilities_from("baseline");
        let raw = serde_json::to_value(base).unwrap();
        assert!(check_describe(&raw).is_ok());
        for kinds in [
            json!([]),
            json!(["unknown"]),
            json!(["reasoning", "reasoning"]),
        ] {
            let mut bad = raw.clone();
            bad["capabilities"] = json!(["archive_prunes_reasoning"]);
            bad["archive_prunes_reasoning"] = json!({"kinds": kinds});
            assert!(check_describe(&bad).is_err());
        }
        let mut paired = raw.clone();
        paired["archive_prunes_reasoning"] = json!({"kinds": kinds::ALL});
        assert!(check_describe(&paired).is_err());
        paired["capabilities"] = json!(["archive_prunes_reasoning"]);
        assert!(check_describe(&paired).is_ok());
        paired
            .as_object_mut()
            .unwrap()
            .remove("archive_prunes_reasoning");
        assert!(check_describe(&paired).is_err());
    }

    #[test]
    fn pruning_state_and_served_part_keys_round_trip_on_pages() {
        let vectors = crate::vectors::load("reasoning-pruned.json");
        for case in vectors["heads"].as_array().unwrap() {
            crate::vectors::round_trip::<crate::read::HeadMeta>(
                case["name"].as_str().unwrap(),
                &case["head"],
            );
        }
        for case in vectors["pages"].as_array().unwrap() {
            crate::vectors::round_trip::<crate::read::ReadPage>(
                case["name"].as_str().unwrap(),
                &case["page"],
            );
        }
        let part = PrunedReasoning::new(
            "host-id",
            "opaque-key",
            kinds::REASONING,
            42,
            "a".repeat(64),
        );
        let message = crate::read::ReadMessage::new(
            1,
            "host-id",
            json!({"content":[{"type":"reasoning","text":""}]}),
        )
        .with_written_at(1000)
        .with_part_keys(vec!["opaque-key".into()]);
        let page = crate::read::ReadPage::new(7, "record-identity", vec![message.clone()])
            .with_pruned_reasoning(vec![part.clone()])
            .with_reasoning_pruned(ReasoningPruned::new(1, 1, 1000));
        let value = serde_json::to_value(&page).unwrap();
        assert_eq!(
            serde_json::from_value::<crate::read::ReadPage>(value).unwrap(),
            page
        );
        assert_eq!(
            page.messages[0].served_part_keys(),
            Some(vec!["opaque-key"])
        );
        let history = crate::history::HistoryPage::new(
            7,
            "record-identity",
            vec![crate::history::HistoryEntry::from_read(message).unwrap()],
        )
        .with_pruned_reasoning(vec![part]);
        assert_eq!(history.check(), Ok(()));
        assert_eq!(
            history.messages[0].part_keys.as_ref().unwrap(),
            &["opaque-key"]
        );
        assert!(!ReasoningPruned::new(1, 0, 1000).valid());
    }
}
