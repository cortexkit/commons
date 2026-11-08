//! Views: what the provider wants the model to see, as edits over the
//! runner's transcript.
//!
//! A view is cumulative: each new version carries every edit of the previous
//! one, unchanged or changed in place (a changed edit takes the new version
//! as its `since`), plus new ones. An edit leaves only when a revert removes
//! the message it names, except when a provider replaces a foreign view whole
//! (CONTRACT.md W5.6). Parts not named by edits render unchanged unless the view
//! explicitly authorizes stripping invalidated newer signatures (CONTRACT.md W6.1).
//! While the transcript only grows and the view stays the same,
//! each request extends the previous one byte for byte; a revert, branch
//! switch or host edit can break that before a newer view arrives. Whether
//! the model provider's cache still holds the prefix depends on the cache's
//! lifetime and on the rest of the request staying the same.
//!
//! The provider computes every byte it puts in a view (placeholders such as
//! `[dropped §12§]`, cut-down edit arguments, the history head). The runner
//! adds nothing to them, not even a tag prefix; every other tagged text is
//! rendered with its prefix.
//!
//! No edit changes a tag number: the runner counts from the transcript, not
//! from what it renders. Hiding numbered parts can leave gaps, or lead the
//! model to write an older number; reply cleaning and rendering then restore
//! the number the runner assigned (W4.6, W6.4). Separately, a view must not
//! write a number into its own bytes, so a runner refuses an inserted message
//! that carries a tag prefix, and a replaced field that carries one the part
//! neither owns nor already contained.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::read::Role;

/// A published view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct View {
    /// At least 1, and greater than every view the provider built for the
    /// session before, whether or not the runner accepted it.
    #[serde(deserialize_with = "crate::integer::required")]
    pub version: u64,
    /// Checksum of `version`, `edits` and the optional policy members when
    /// present ([`View::seal_with_policy`]).
    pub checksum: String,
    pub edits: Vec<Edit>,
    /// Newest message the provider's index held when it built the view.
    /// Message ids, unlike ordinals, are never reused after a revert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_through: Option<String>,
    /// `strip` permits dropping invalidated signed parts strictly after the
    /// `built_through` message. Unknown strings are preserved but grant no permission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_signed: Option<String>,
}

impl View {
    /// Seal for the latest Setup. Without explicit anchor support, omit both
    /// policy members so older runners verify the original checksum shape.
    pub fn seal_for_setup(
        setup: &crate::setup::SetupRequest,
        version: u64,
        edits: Vec<Edit>,
        built_through: Option<String>,
        new_signed: Option<String>,
    ) -> Self {
        if setup.view_anchor == Some(true) {
            Self::seal_with_policy(version, edits, built_through, new_signed)
        } else {
            Self::seal(version, edits)
        }
    }

    /// Build a view, computing its checksum. Nothing else is checked: not
    /// the version, the edits, or what they name.
    pub fn seal(version: u64, edits: Vec<Edit>) -> Self {
        Self::seal_with_policy(version, edits, None, None)
    }

    /// Build a view whose checksum includes the policy members that are
    /// present. Absent members do not change checksums of older views.
    /// Providers may include either member only after Setup's `view_anchor`
    /// opt-in; [`Self::seal_for_setup`] applies that gate automatically.
    pub fn seal_with_policy(
        version: u64,
        edits: Vec<Edit>,
        built_through: Option<String>,
        new_signed: Option<String>,
    ) -> Self {
        let checksum = view_checksum(
            version,
            &edits,
            built_through.as_deref(),
            new_signed.as_deref(),
        );
        Self {
            version,
            checksum,
            edits,
            built_through,
            new_signed,
        }
    }

    /// Whether the checksum matches the view's content, including present
    /// policy members. The transcript and provider rules are not checked.
    pub fn verify(&self) -> bool {
        self.checksum
            == view_checksum(
                self.version,
                &self.edits,
                self.built_through.as_deref(),
                self.new_signed.as_deref(),
            )
    }

    pub fn applied(&self) -> AppliedView {
        AppliedView {
            version: self.version,
            checksum: self.checksum.clone(),
        }
    }
}

fn view_checksum(
    version: u64,
    edits: &[Edit],
    built_through: Option<&str>,
    new_signed: Option<&str>,
) -> String {
    #[derive(Serialize)]
    struct Sealed<'a> {
        version: u64,
        edits: &'a [Edit],
        #[serde(skip_serializing_if = "Option::is_none")]
        built_through: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        new_signed: Option<&'a str>,
    }
    crate::digest::checksum(&Sealed {
        version,
        edits,
        built_through,
        new_signed,
    })
}

/// Names a view by version and checksum. A version alone is not enough: a
/// provider restored from an older backup could publish different content
/// under a version number the runner already applied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AppliedView {
    #[serde(deserialize_with = "crate::integer::required")]
    pub version: u64,
    pub checksum: String,
}

/// One edit, with the view version that first held it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Edit {
    /// The version of the first view that held this edit unchanged. A view
    /// repeats an earlier edit with its original `since`; a new or changed
    /// edit gets the view's own version. The rule check uses it to tell
    /// which edits came after a signed reasoning part was generated, and
    /// a runner verifies it against its previous view.
    #[serde(deserialize_with = "crate::integer::required")]
    pub since: u64,
    #[serde(flatten)]
    pub op: EditOp,
}

impl Edit {
    pub fn new(since: u64, op: EditOp) -> Self {
        Self { since, op }
    }
}

open_tagged! {
/// What an edit does.
#[derive(Clone, Debug, PartialEq)]
pub enum EditOp, "op" {
    /// Leave out one part, or the whole message when `part` is absent.
    Drop => "drop" {
        mid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
    },
    /// Render one field of a part with new bytes.
    Replace => "replace" {
        mid: String,
        part: String,
        /// `text` (a text part, or a file part, which then renders as text
        /// in the file's place), `output` (a tool result's text),
        /// `input.<key>` (one top-level string argument of a tool call), or
        /// `attachments` (a tool result's attachments), or `delivered` (its
        /// delivered text items).
        field: String,
        /// A string, for `attachments` the array of attachment ids to keep,
        /// or for `delivered` the array of item indexes to keep.
        value: Value,
        /// The part's checksum when the provider computed this edit.
        expect: String,
    },
    /// Render a message the transcript does not hold, such as the history
    /// summary at the start.
    Insert => "insert" {
        at: InsertAt,
        message: InsertedMessage,
    },
    /// Leave out every message up to and including `through`: the messages a
    /// summary at the start replaces. One per view. Once a view with a cut
    /// applies, the runner writes its host's own compaction marker there
    /// (M1), so the host stops loading those messages, and reports it in its
    /// next notice.
    Cut => "cut" { through: String },
}
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
#[non_exhaustive]
pub enum InsertAt {
    /// Before the first rendered message.
    Start,
    /// Immediately before this message, whether or not the message itself
    /// is dropped.
    Before(String),
    /// An unrecognised anchor name or object, kept without interpretation.
    /// A runner refuses the view `unsupported_edit` (CONTRACT.md W1.5).
    Unknown(Value),
}

impl TryFrom<Value> for InsertAt {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::String(name) if name == "start" => Ok(Self::Start),
            Value::String(name) => Ok(Self::Unknown(Value::String(name))),
            // W1.4: unknown fields beside `before` are ignored.
            Value::Object(fields) => match fields.get("before") {
                Some(Value::String(mid)) => Ok(Self::Before(mid.clone())),
                Some(_) => Err("expected a message id for `before`".to_owned()),
                None => Ok(Self::Unknown(Value::Object(fields))),
            },
            _ => Err("expected an anchor name or an anchor object".to_owned()),
        }
    }
}

impl From<InsertAt> for Value {
    fn from(value: InsertAt) -> Self {
        match value {
            InsertAt::Start => Value::String("start".to_owned()),
            InsertAt::Before(mid) => serde_json::json!({"before": mid}),
            InsertAt::Unknown(value) => value,
        }
    }
}

/// A message a view adds. Texts and images carry no tag numbers; the runner
/// renders images after the texts, in its host's own form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InsertedMessage {
    /// Chosen by the provider, unique within the view, and kept the same
    /// across views while the message is unchanged.
    pub id: String,
    pub role: Role,
    pub texts: Vec<String>,
    /// Only user-role inserts may carry images, and only when the runner
    /// reports `accepts_images: true`. Empty arrays hash as absent (W1.6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<InsertedImage>,
}

/// An image carried by an inserted message, included in the view's JSON size
/// and checksum. The runner checks its media type and base64 before applying
/// the view (W6.5); decoding the wire value alone does not validate either.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InsertedImage {
    /// Currently only `image/png` is supported.
    pub media_type: String,
    /// The bytes in standard padded base64.
    pub data: String,
}

open_tagged! {
/// Why a runner refuses a view. The runner keeps applying its previous view.
/// A refusal of a handed-over view is that call's answer; a refusal of a
/// ride's view, or of an accepted view at a step boundary, goes in the next
/// notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewRefusal, "code" {
    BadChecksum => "bad_checksum",
    /// The view's version is not newer than the runner's latest view.
    StaleVersion => "stale_version" {
        #[serde(deserialize_with = "crate::integer::required")]
        version: u64,
        #[serde(deserialize_with = "crate::integer::required")]
        latest: u64,
    },
    /// An edit's `since` is not consistent with the previous view.
    BadSince => "bad_since",
    UnknownMessage => "unknown_message" {
        mid: String,
    },
    UnknownPart => "unknown_part" {
        mid: String,
        part: String,
    },
    /// The part changed since the provider computed the edit.
    StalePart => "stale_part" {
        mid: String,
        part: String,
    },
    /// The field does not exist on that part, or the value has the wrong shape.
    BadField => "bad_field" {
        mid: String,
        part: String,
        field: String,
    },
    /// Two edits target the same part in ways that cannot both hold, such as
    /// a drop and a replace, or two replaces of one field.
    Conflict => "conflict" {
        mid: String,
        part: String,
    },
    /// A tool call still running cannot be dropped or edited.
    RunningTool => "running_tool" {
        mid: String,
        part: String,
    },
    UnknownAnchor => "unknown_anchor" {
        mid: String,
    },
    DuplicateInsert => "duplicate_insert" {
        id: String,
    },
    /// More than one cut.
    DuplicateCut => "duplicate_cut",
    /// An inserted message's text contains a tag prefix.
    TaggedInsert => "tagged_insert" {
        id: String,
    },
    /// A replace value contains a tag prefix for a number other than the
    /// part's own that the field's original text does not already contain.
    ForeignTag => "foreign_tag" {
        mid: String,
        part: String,
        field: String,
        #[serde(deserialize_with = "crate::integer::required")]
        tag: u64,
    },
    /// The view breaks a provider rule the runner declared.
    Rule => "rule"(payload: crate::rules::RuleBreak),
    /// The edit at this index of `edits` has an `op`, an insert `at` or an
    /// inserted message `role` the runner does not know (W1.5), or images on
    /// a non-user insert, a non-PNG media type, or invalid standard padded
    /// base64 (W6.5). Each case names the zero-based edit index.
    UnsupportedEdit => "unsupported_edit" {
        #[serde(deserialize_with = "crate::integer::required")]
        index: u64,
    },
}
}

/// The largest view, as serialized JSON, a provider may hand over (W6.7).
/// It leaves room under a module call's 4 MiB frame for the request around it.
/// A provider refuses by name to send a larger view: one would be a bug in
/// the provider, not something to page.
pub const MAX_VIEW_BYTES: usize = 3 << 20;

/// `session.apply_view`, served by the runner: the provider hands over a new
/// view. The runner checks it with the view library (its shape, its `since`
/// values and the provider rules), stores it, and applies it from the next step
/// boundary, never in the middle of a step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApplyViewRequest {
    pub view: View,
}

open_tagged! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyViewAnswer, "answer" {
    /// Stored; it applies from the next step boundary.
    Accepted => "accepted",
    /// Not stored; the runner keeps its current view.
    Refused => "refused" { refusal: ViewRefusal },
}
}

/// `session.applied_view`, served by the runner: the view it applies now, for
/// a provider whose own copy is missing or older (for example after its store
/// was restored from a backup).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AppliedViewRequest {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AppliedViewAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
    /// Applying module id, attested by the daemon for the caller's route and
    /// recorded when the runner accepts the view, never from request data.
    /// Absent when the author was not recorded, including first views imported
    /// from standalone sessions. Providers replace a foreign module's whole
    /// view instead of adopting its edits, including the cut that hides earlier
    /// history (CONTRACT.md W5.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_by: Option<String>,
}

impl AppliedView {
    /// Build the value with its required fields.
    pub fn new(version: u64, checksum: impl Into<String>) -> Self {
        Self {
            version,
            checksum: checksum.into(),
        }
    }
}

impl InsertedMessage {
    /// Build the value with its required fields.
    pub fn new(id: impl Into<String>, role: Role, texts: Vec<String>) -> Self {
        Self {
            id: id.into(),
            role,
            texts,
            images: Vec::new(),
        }
    }

    pub fn with_images(mut self, images: Vec<InsertedImage>) -> Self {
        self.images = images;
        self
    }
}

impl InsertedImage {
    /// Build the wire value; the runner validates it when checking the view.
    pub fn new(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            media_type: media_type.into(),
            data: data.into(),
        }
    }
}

impl ApplyViewRequest {
    /// Build the value with its required fields.
    pub fn new(view: View) -> Self {
        Self { view }
    }
}

impl AppliedViewRequest {
    /// Build the value with its required fields.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for AppliedViewRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl AppliedViewAnswer {
    /// Build the value with its required fields.
    pub fn new() -> Self {
        Self {
            view: None,
            applied_by: None,
        }
    }

    pub fn with_view(mut self, view: View) -> Self {
        self.view = Some(view);
        self
    }

    pub fn with_applied_by(mut self, module: impl Into<String>) -> Self {
        self.applied_by = Some(module.into());
        self
    }
}

impl Default for AppliedViewAnswer {
    fn default() -> Self {
        Self::new()
    }
}
