//! The privileged `message` display body. It carries metadata, not chat text.

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::{InvalidParams, Principal};

pub(crate) const MESSAGE_WRITER: &str = "prefrontal";
pub(crate) const MESSAGE_WRITER_ALIAS: &str = "prefrontal-core";

pub(crate) fn is_message_writer(writer: &Principal) -> bool {
    matches!(writer, Principal::Reserved { module_id }
        if matches!(module_id.as_str(), MESSAGE_WRITER | MESSAGE_WRITER_ALIAS))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MessageRecord {
    pub v: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub reply_to: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub quote: Option<String>,
    pub media: Vec<MessageMedia>,
    pub level: MessageLevel,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub key: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub done: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub deliver: Option<MessageDeliver>,
}

impl MessageRecord {
    pub fn new(media: Vec<MessageMedia>, level: MessageLevel) -> Self {
        Self {
            v: 1,
            reply_to: None,
            quote: None,
            media,
            level,
            key: None,
            done: None,
            deliver: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageLevel {
    Chat,
    Info,
    Attention,
    Urgent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageDeliver {
    Desktop,
    Phone,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[non_exhaustive]
pub enum MessageMedia {
    Image {
        artifact: String,
        mime: String,
        byte_count: u64,
        width: u64,
        height: u64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        title: Option<String>,
    },
    Video {
        artifact: String,
        mime: String,
        byte_count: u64,
        width: u64,
        height: u64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        duration_ms: Option<u64>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        poster: Option<MessagePoster>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        title: Option<String>,
    },
    File {
        artifact: String,
        mime: String,
        byte_count: u64,
        name: String,
    },
    Link {
        url: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        title: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        description: Option<String>,
    },
    Diff {
        artifact: String,
        repo: MessageRepo,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        commit: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        range: Option<MessageRange>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        pr: Option<MessagePr>,
        files: Vec<MessageDiffFile>,
        added: u64,
        removed: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MessagePoster {
    pub artifact: String,
    pub width: u64,
    pub height: u64,
}

impl MessagePoster {
    pub fn new(artifact: impl Into<String>, width: u64, height: u64) -> Self {
        Self {
            artifact: artifact.into(),
            width,
            height,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MessageRepo {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub host: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub owner: Option<String>,
    pub name: String,
}

impl MessageRepo {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            host: None,
            owner: None,
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MessageRange {
    pub base: String,
    pub head: String,
}

impl MessageRange {
    pub fn new(base: impl Into<String>, head: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            head: head.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MessagePr {
    pub number: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub head: Option<String>,
}

impl MessagePr {
    pub fn new(number: u64) -> Self {
        Self { number, head: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
#[non_exhaustive]
pub enum MessageDiffFile {
    Text {
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        from: Option<String>,
        added: u64,
        removed: u64,
    },
    Binary {
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        from: Option<String>,
        #[serde(deserialize_with = "true_only")]
        binary: bool,
    },
}

// Optional members are omitted, not null. Serde's usual Option decoder accepts both.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

fn true_only<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    if bool::deserialize(d)? {
        Ok(true)
    } else {
        Err(serde::de::Error::custom("binary must be true"))
    }
}

/// Decode a message body, naming the precise invalid member in an `invalid_params` refusal.
/// [`crate::validate_write`] also checks the canonical byte limit and envelope.
pub fn validate_message(body: &Map<String, Value>) -> Result<MessageRecord, InvalidParams> {
    let body = Object {
        members: body,
        path: "body".into(),
    };
    body.closed(&[
        "v", "reply_to", "quote", "media", "level", "key", "done", "deliver",
    ])?;
    let v = body.required("v")?;
    if v != 1 {
        return Err(body.invalid("v"));
    }
    let reply_to = body.optional("reply_to")?;
    let quote: Option<String> = body.optional("quote")?;
    if quote
        .as_ref()
        .is_some_and(|quote| reply_to.is_none() || quote.chars().count() > 280)
    {
        return Err(body.invalid("quote"));
    }
    let media = body.list("media", 8, decode_media)?;
    let level = body.required("level")?;
    let key: Option<String> = body.optional("key")?;
    if key.as_ref().is_some_and(|key| key.chars().count() > 64) {
        return Err(body.invalid("key"));
    }
    let done = body.optional("done")?;
    if done.is_some() && key.is_none() {
        return Err(body.invalid("done"));
    }
    let deliver = body.optional("deliver")?;
    if level != MessageLevel::Chat && deliver.is_none() {
        return Err(body.invalid("deliver"));
    }
    Ok(MessageRecord {
        v,
        reply_to,
        quote,
        media,
        level,
        key,
        done,
        deliver,
    })
}

fn decode_media(item: Object<'_>) -> Result<MessageMedia, InvalidParams> {
    match item.required::<String>("kind")?.as_str() {
        "image" => {
            item.closed(&[
                "kind",
                "artifact",
                "mime",
                "byteCount",
                "width",
                "height",
                "title",
            ])?;
            Ok(MessageMedia::Image {
                artifact: item.required("artifact")?,
                mime: item.required("mime")?,
                byte_count: item.required("byteCount")?,
                width: item.required("width")?,
                height: item.required("height")?,
                title: item.optional("title")?,
            })
        }
        "video" => {
            item.closed(&[
                "kind",
                "artifact",
                "mime",
                "byteCount",
                "width",
                "height",
                "durationMs",
                "poster",
                "title",
            ])?;
            let poster = item
                .nested("poster")?
                .map(|poster| {
                    poster.closed(&["artifact", "width", "height"])?;
                    Ok(MessagePoster::new(
                        poster.required::<String>("artifact")?,
                        poster.required("width")?,
                        poster.required("height")?,
                    ))
                })
                .transpose()?;
            Ok(MessageMedia::Video {
                artifact: item.required("artifact")?,
                mime: item.required("mime")?,
                byte_count: item.required("byteCount")?,
                width: item.required("width")?,
                height: item.required("height")?,
                duration_ms: item.optional("durationMs")?,
                poster,
                title: item.optional("title")?,
            })
        }
        "file" => {
            item.closed(&["kind", "artifact", "mime", "byteCount", "name"])?;
            Ok(MessageMedia::File {
                artifact: item.required("artifact")?,
                mime: item.required("mime")?,
                byte_count: item.required("byteCount")?,
                name: item.required("name")?,
            })
        }
        "link" => {
            item.closed(&["kind", "url", "title", "description"])?;
            let url: String = item.required("url")?;
            if !is_web_url(&url) {
                return Err(item.invalid("url"));
            }
            Ok(MessageMedia::Link {
                url,
                title: item.optional("title")?,
                description: item.optional("description")?,
            })
        }
        "diff" => decode_diff(item),
        _ => Err(item.invalid("kind")),
    }
}

fn decode_diff(item: Object<'_>) -> Result<MessageMedia, InvalidParams> {
    item.closed(&[
        "kind", "artifact", "repo", "commit", "range", "pr", "files", "added", "removed",
    ])?;
    let repo = item.nested("repo")?.ok_or_else(|| item.invalid("repo"))?;
    repo.closed(&["host", "owner", "name"])?;
    let repo = MessageRepo {
        host: repo.optional("host")?,
        owner: repo.optional("owner")?,
        name: repo.required("name")?,
    };
    if ["commit", "range", "pr"]
        .into_iter()
        .filter(|key| item.members.contains_key(*key))
        .count()
        != 1
    {
        return Err(InvalidParams::new(item.path));
    }
    let commit = item.sha("commit")?;
    let range = item
        .nested("range")?
        .map(|range| {
            range.closed(&["base", "head"])?;
            Ok(MessageRange {
                base: range.sha("base")?.ok_or_else(|| range.invalid("base"))?,
                head: range.sha("head")?.ok_or_else(|| range.invalid("head"))?,
            })
        })
        .transpose()?;
    let pr = item
        .nested("pr")?
        .map(|pr| {
            pr.closed(&["number", "head"])?;
            Ok(MessagePr {
                number: pr.required("number")?,
                head: pr.sha("head")?,
            })
        })
        .transpose()?;
    Ok(MessageMedia::Diff {
        artifact: item.required("artifact")?,
        repo,
        commit,
        range,
        pr,
        files: item.list("files", 200, decode_file)?,
        added: item.required("added")?,
        removed: item.required("removed")?,
    })
}

fn decode_file(file: Object<'_>) -> Result<MessageDiffFile, InvalidParams> {
    let path = file.required("path")?;
    let from = file.optional("from")?;
    if file.members.contains_key("binary") {
        file.closed(&["path", "from", "binary"])?;
        if !file.required::<bool>("binary")? {
            return Err(file.invalid("binary"));
        }
        Ok(MessageDiffFile::Binary {
            path,
            from,
            binary: true,
        })
    } else {
        file.closed(&["path", "from", "added", "removed"])?;
        Ok(MessageDiffFile::Text {
            path,
            from,
            added: file.required("added")?,
            removed: file.required("removed")?,
        })
    }
}

fn is_web_url(value: &str) -> bool {
    let Some((_, rest)) = value.split_once("://") else {
        return false;
    };
    // The URL parser recovers from missing authorities and strips some invalid characters.
    !rest.is_empty()
        && !rest.starts_with(['/', '?', '#'])
        && !value.contains('\\')
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
        && url::Url::parse(value)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host())
}

// Keep the member's path while decoding. A serde error for a tagged media variant
// on its own does not identify the item's index and nested member.
struct Object<'a> {
    members: &'a Map<String, Value>,
    path: String,
}

impl<'a> Object<'a> {
    fn at(value: &'a Value, path: String) -> Result<Self, InvalidParams> {
        match value.as_object() {
            Some(members) => Ok(Self { members, path }),
            None => Err(InvalidParams::new(path)),
        }
    }

    fn invalid(&self, field: &str) -> InvalidParams {
        InvalidParams::new(format!("{}.{field}", self.path))
    }

    fn closed(&self, allowed: &[&str]) -> Result<(), InvalidParams> {
        if let Some(field) = self
            .members
            .keys()
            .find(|key| !allowed.contains(&key.as_str()))
        {
            return Err(self.invalid(field));
        }
        Ok(())
    }

    fn required<T: DeserializeOwned>(&self, field: &str) -> Result<T, InvalidParams> {
        self.optional(field)?.ok_or_else(|| self.invalid(field))
    }

    fn optional<T: DeserializeOwned>(&self, field: &str) -> Result<Option<T>, InvalidParams> {
        self.members
            .get(field)
            .map(|value| T::deserialize(value).map_err(|_| self.invalid(field)))
            .transpose()
    }

    fn nested(&self, field: &str) -> Result<Option<Self>, InvalidParams> {
        self.members
            .get(field)
            .map(|value| Self::at(value, format!("{}.{field}", self.path)))
            .transpose()
    }

    fn sha(&self, field: &str) -> Result<Option<String>, InvalidParams> {
        let sha: Option<String> = self.optional(field)?;
        if sha.as_ref().is_some_and(|sha| {
            !(matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        }) {
            return Err(self.invalid(field));
        }
        Ok(sha)
    }

    fn list<T>(
        &self,
        field: &str,
        max: usize,
        decode: impl Fn(Self) -> Result<T, InvalidParams>,
    ) -> Result<Vec<T>, InvalidParams> {
        let items = self
            .members
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| self.invalid(field))?;
        if items.len() > max {
            return Err(self.invalid(field));
        }
        items
            .iter()
            .enumerate()
            .map(|(i, item)| decode(Self::at(item, format!("{}.{field}[{i}]", self.path))?))
            .collect()
    }
}
