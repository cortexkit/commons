//! `session.entries`, served by the runner: transcript entries in the neutral
//! part model, plus where the transcript and the applied view stand.
//!
//! An entry is every record of the host's transcript, on the active path,
//! that the model sees or that holds part of what it sees, in host order.
//! Marker records are never entries. A record whose content belongs to
//! another entry (a Pi `toolResult`, a tool-role message holding a batch's
//! results) is an entry with no parts: its result sits in the call's tool
//! part. So is a step that died before any content. Which storage records
//! that means on each host is the contract's table W4.2.

use serde::{Deserialize, Serialize};

use crate::part::Part;
use crate::view::AppliedView;

/// Names a session: the key each side keeps it under. It is built from the
/// route a call arrives on, never from a request body, which the caller could
/// forge. `runner` is the module the daemon attests for the route, and
/// `harness` and `session` are the identity the route is bound to. Every op of
/// this contract concerns the session its route is bound to, so no request
/// names a session. `harness` and `session` stay the same when a session moves
/// from a standalone host to a runner.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SessionRef {
    /// The runner module serving the session, for example `example-runner`.
    pub runner: String,
    /// The host the session lives in, for example `opencode` or `pi`.
    pub harness: String,
    /// The host's own session id.
    pub session: String,
}

/// One message of the transcript, as stored. Views never change entries;
/// they change what the runner renders from them. The parts come in full or
/// compact form ([`Detail`]); checksums use W7's full-detail projections,
/// minus the metadata W7 lists.
///
/// Kinds allowed by role: user entries (notices included) hold text and
/// files; assistant entries hold text, reasoning, tool, file and opaque
/// parts; system entries hold text only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Entry {
    /// Position on the session's active path, counting from 1 at its first
    /// entry. Every entry counts, including the ones a cut leaves out. After a
    /// revert or a branch switch, a new entry takes the ordinal of the removed
    /// one; the runner's notice says so.
    #[serde(deserialize_with = "crate::integer::required")]
    pub ordinal: u64,
    /// The host's message id. Stable for the life of the message, and never
    /// given to a different message within the session, even after a revert
    /// (P6). Nothing may derive it from the ordinal or the reverse.
    pub mid: String,
    pub role: Role,
    /// Set when a user-role entry was delivered by a module or the runner
    /// (a reminder, a wake, a notice) rather than typed or prompted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<NoticeOrigin>,
    /// When the host first stored the message, in milliseconds since the Unix
    /// epoch. It does not change when the entry is updated. For a message the
    /// runner did not store itself (history from before the runner, a moved
    /// session), the host's own write time.
    #[serde(deserialize_with = "crate::integer::required")]
    pub written_at: u64,
    /// When the host marked the message finished, in milliseconds since the
    /// Unix epoch, from the host's own stamp. Absent while the entry is
    /// unfinished or when the host records no stamp; time-marker readers then
    /// use `written_at`. Excluded from [`entry_digest`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub completed_at: Option<u64>,
    /// Checksum of W7's full-detail projection of role, notice origin and
    /// parts, minus the metadata W7 lists ([`entry_digest`]).
    pub digest: String,
    pub parts: Vec<Part>,
    /// For an assistant entry: what the request that produced it reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<StepUsage>,
    /// Assistant step reasoning in the page or notice's `token_family`.
    /// Absent means estimate. Once sent as measured, the value is frozen
    /// within its entry and family (W4.11). Excluded from [`entry_digest`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<crate::tokens::ReasoningTokens>,
    /// For an assistant entry: the view applied to the request that produced
    /// it. Signed reasoning is bound to the history as rendered under that
    /// view, so the view library's rule check compares against it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_under: Option<AppliedView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum Role {
    User,
    Assistant,
    /// A turn-scoped system entry (llm-runner's `turn_system` delivery).
    System,
    /// An unrecognised wire value, kept verbatim; handled as CONTRACT.md
    /// W1.5 says.
    Unknown(String),
}

open_string! { Role {
    User => "user",
    Assistant => "assistant",
    System => "system",
} }

/// Who delivered a notice entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NoticeOrigin {
    /// The module that sent it, for example `example-notifier`.
    pub sender: String,
    /// The template it was rendered from, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// The session owner marked this notice as carrying the person's own words.
    /// Providers never drop it automatically; an agent's explicit drop still may.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
}

/// Token counts the model provider reported for one request, attached to the
/// assistant entry it produced. The runner fills these per provider so they
/// mean the same everywhere.
///
/// Every count is optional: absent means the provider did not report it,
/// never zero. A runner whose host stores 0 for "not reported" leaves the
/// count out unless it knows the 0 was reported, and always leaves out a
/// `prompt_tokens` of 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StepUsage {
    /// The whole prompt: uncached input plus cache reads plus cache writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub cache_write_tokens: Option<u64>,
    /// Reply tokens, reasoning included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub output_tokens: Option<u64>,
    /// Reasoning tokens within `output_tokens`. The reasoning budget depends
    /// on it: encrypted reasoning has no other measure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub reasoning_tokens: Option<u64>,
}

/// Where an entry sits: enough to tell whether a later read continues from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cursor {
    #[serde(deserialize_with = "crate::integer::required")]
    pub ordinal: u64,
    pub mid: String,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReadRequest {
    pub select: Select,
    pub detail: Detail,
    /// Stop adding entries once the page reaches this many bytes. The page
    /// always holds at least one entry when one matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub max_bytes: Option<u64>,
}

/// How much of each part a read returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    /// Parts in compact form ([`crate::part::Part::compact`]): enough to keep
    /// an index and plan edits. Used to catch up after a missed notice.
    Compact,
    /// Parts in full, for the summarizer and for showing a message again.
    Full,
}

open_tagged! {
/// Which entries to read.
#[derive(Clone, Debug, PartialEq)]
pub enum Select, "by" {
    /// Entries after an ordinal, oldest first; from the start when absent.
    After => "after" {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(deserialize_with = "crate::integer::optional")]
        after_ordinal: Option<u64>,
        /// At most this many entries; at least 1 when supplied (W5.4).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(deserialize_with = "crate::integer::positive")]
        limit: Option<u64>,
    },
    /// Exactly these messages, in transcript order. Missing ids are left out.
    Mids => "mids" { mids: Vec<String> },
    /// The messages holding these tag numbers, in transcript order.
    Tags => "tags" {
        #[serde(deserialize_with = "crate::integer::list")]
        tags: Vec<u64>
    },
}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReadPage {
    pub entries: Vec<Entry>,
    /// Opaque count family, required when this page carries W4.11 counts.
    /// Adopt each response's family in receipt order and discard held counts
    /// from other families. Compare the whole id by equality only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_family: Option<String>,
    /// Whether the family uses the model's own tokenizer (`true`) rather
    /// than a stand-in (`false`). Absent is unknown, treated as `false`.
    /// Sent alongside `token_family`; excluded from all W7 checksums.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_family_exact: Option<bool>,
    /// For [`Select::After`]: the ordinal to continue after when more entries
    /// match than fit the page. Absent when the page reaches the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub next_after: Option<u64>,
    pub head: Head,
}

/// Where the session stands at the moment of the read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Head {
    /// The newest entry, absent for an empty transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<Cursor>,
    /// The view the runner applies now, absent when it applies none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_view: Option<AppliedView>,
}

/// Checksum of an entry's role, notice origin (left out when absent) and
/// parts, each part with its `id`, `tag` and `digest`, as [`Entry::digest`]
/// carries it. The ordinal, `mid`, `written_at`, `completed_at`, usage and
/// `generated_under`, reasoning counts, part `tokens` and delivered-item
/// `tokens` are not hashed.
/// Pass full parts: compact parts hash to a
/// different value.
pub fn entry_digest(role: Role, notice: Option<&NoticeOrigin>, parts: &[Part]) -> String {
    #[derive(Serialize)]
    struct Digested<'a> {
        role: Role,
        #[serde(skip_serializing_if = "Option::is_none")]
        notice: Option<&'a NoticeOrigin>,
        parts: &'a [Part],
    }
    let mut value = serde_json::to_value(Digested {
        role,
        notice,
        parts,
    })
    .expect("contract types always serialize");
    // Strip only part metadata, never similarly named fields in tool input
    // or other opaque content.
    for part in value["parts"].as_array_mut().unwrap() {
        part.as_object_mut().unwrap().remove("tokens");
        crate::part::strip_delivered_tokens(part);
    }
    crate::digest::content_digest(&value)
}

impl Entry {
    /// Build an entry from full parts and compute its checksum. Pass full
    /// parts, never compact ones. Changing a hashed field afterwards doesn't
    /// refresh `digest`.
    pub fn new(
        ordinal: u64,
        mid: impl Into<String>,
        role: Role,
        written_at: u64,
        parts: Vec<Part>,
    ) -> Self {
        let digest = entry_digest(role.clone(), None, &parts);
        Self {
            ordinal,
            mid: mid.into(),
            role,
            notice: None,
            written_at,
            completed_at: None,
            digest,
            parts,
            usage: None,
            reasoning_tokens: None,
            generated_under: None,
        }
    }

    /// Set the notice origin and recompute the checksum, so the parts must
    /// be full.
    pub fn with_notice(mut self, notice: NoticeOrigin) -> Self {
        self.notice = Some(notice);
        self.digest = entry_digest(self.role.clone(), self.notice.as_ref(), &self.parts);
        self
    }

    /// Set the host's completion stamp without changing the content checksum.
    pub fn with_completed_at(mut self, completed_at: u64) -> Self {
        self.completed_at = Some(completed_at);
        self
    }

    pub fn with_usage(mut self, usage: StepUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_reasoning_tokens(mut self, tokens: crate::tokens::ReasoningTokens) -> Self {
        self.reasoning_tokens = Some(tokens);
        self
    }

    pub fn with_generated_under(mut self, generated_under: AppliedView) -> Self {
        self.generated_under = Some(generated_under);
        self
    }
}

impl SessionRef {
    /// Build the value with its required fields.
    pub fn new(
        runner: impl Into<String>,
        harness: impl Into<String>,
        session: impl Into<String>,
    ) -> Self {
        Self {
            runner: runner.into(),
            harness: harness.into(),
            session: session.into(),
        }
    }
}

impl NoticeOrigin {
    /// Build the value with its required fields.
    pub fn new(sender: impl Into<String>) -> Self {
        Self {
            sender: sender.into(),
            template: None,
            keep: false,
        }
    }

    pub fn with_template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }

    /// Protect this notice from automatic provider drops, not explicit agent drops.
    pub fn with_keep(mut self, keep: bool) -> Self {
        self.keep = keep;
        self
    }
}

impl StepUsage {
    /// Build the value with its required fields.
    pub fn new() -> Self {
        Self {
            prompt_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
        }
    }

    /// Set the prompt token count as given. Leave it unset when the model
    /// provider reported none; never pass 0 for an unreported count.
    pub fn with_prompt_tokens(mut self, prompt_tokens: u64) -> Self {
        self.prompt_tokens = Some(prompt_tokens);
        self
    }

    pub fn with_cache_read_tokens(mut self, cache_read_tokens: u64) -> Self {
        self.cache_read_tokens = Some(cache_read_tokens);
        self
    }

    pub fn with_cache_write_tokens(mut self, cache_write_tokens: u64) -> Self {
        self.cache_write_tokens = Some(cache_write_tokens);
        self
    }

    pub fn with_output_tokens(mut self, output_tokens: u64) -> Self {
        self.output_tokens = Some(output_tokens);
        self
    }

    pub fn with_reasoning_tokens(mut self, reasoning_tokens: u64) -> Self {
        self.reasoning_tokens = Some(reasoning_tokens);
        self
    }
}

impl Cursor {
    /// Build the value with its required fields.
    pub fn new(ordinal: u64, mid: impl Into<String>, digest: impl Into<String>) -> Self {
        Self {
            ordinal,
            mid: mid.into(),
            digest: digest.into(),
        }
    }
}

impl ReadRequest {
    /// Build the value with its required fields.
    pub fn new(select: Select, detail: Detail) -> Self {
        Self {
            select,
            detail,
            max_bytes: None,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }
}

impl ReadPage {
    /// Build the value with its required fields.
    pub fn new(entries: Vec<Entry>, head: Head) -> Self {
        Self {
            entries,
            head,
            next_after: None,
            token_family: None,
            token_family_exact: None,
        }
    }

    pub fn with_next_after(mut self, next_after: u64) -> Self {
        self.next_after = Some(next_after);
        self
    }

    pub fn with_token_family(mut self, family: impl Into<String>) -> Self {
        self.token_family = Some(family.into());
        self
    }

    pub fn with_token_family_exact(mut self, exact: bool) -> Self {
        self.token_family_exact = Some(exact);
        self
    }
}

impl Head {
    /// Build the value with its required fields.
    pub fn new() -> Self {
        Self {
            newest: None,
            applied_view: None,
        }
    }

    pub fn with_newest(mut self, newest: Cursor) -> Self {
        self.newest = Some(newest);
        self
    }

    pub fn with_applied_view(mut self, applied_view: AppliedView) -> Self {
        self.applied_view = Some(applied_view);
        self
    }
}

impl Default for Head {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn absent_notice_is_omitted_from_the_entry_checksum() {
        let expected = crate::digest::content_digest(&json!({"role": "user", "parts": []}));
        assert_eq!(entry_digest(Role::User, None, &[]), expected);
        assert_ne!(
            expected,
            crate::digest::content_digest(&json!({"role": "user", "notice": null, "parts": []}))
        );
    }

    #[test]
    fn completed_at_is_optional_and_round_trips() {
        let entry = Entry::new(1, "m1", Role::Assistant, 1000, vec![]);
        let wire = serde_json::to_value(&entry).unwrap();
        assert!(wire.get("completed_at").is_none());
        assert_eq!(serde_json::from_value::<Entry>(wire).unwrap(), entry);
        assert_eq!(entry.completed_at, None);

        for completed_at in [0, 2000, 9_007_199_254_740_991] {
            let finished = entry.clone().with_completed_at(completed_at);
            let wire = serde_json::to_value(&finished).unwrap();
            assert_eq!(wire["completed_at"], completed_at);
            assert_eq!(serde_json::from_value::<Entry>(wire).unwrap(), finished);
        }
    }

    #[test]
    fn completed_at_obeys_timestamp_integer_rules() {
        let entry = Entry::new(1, "m1", Role::Assistant, 1000, vec![]);
        for invalid in [
            json!(9007199254740992_u64),
            json!(-1),
            json!(1.5),
            json!("2000"),
        ] {
            let mut wire = serde_json::to_value(&entry).unwrap();
            wire["completed_at"] = invalid;
            assert!(serde_json::from_value::<Entry>(wire).is_err());
        }
    }

    #[test]
    fn completed_at_does_not_change_the_entry_checksum() {
        let entry = Entry::new(1, "m1", Role::Assistant, 1000, vec![]);
        let expected = crate::digest::content_digest(&json!({"role": "assistant", "parts": []}));
        assert_eq!(entry.digest, expected);
        let finished = entry.with_completed_at(2000);
        assert_eq!(finished.digest, expected);
        assert_eq!(
            entry_digest(finished.role, finished.notice.as_ref(), &finished.parts),
            expected
        );
    }
}
