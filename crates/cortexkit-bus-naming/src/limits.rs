use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::names::AccountNames;

pub const HOUR: Duration = Duration::from_secs(60 * 60);
pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Room,
    Wake,
    Peer,
    Effect,
    EffectDead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardPolicy {
    Old,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSpec {
    pub kind: StreamKind,
    pub name: String,
    pub subjects: Vec<String>,
    pub max_age: Duration,
    pub max_bytes: u64,
    pub discard: DiscardPolicy,
    pub work_queue: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerSpec {
    pub durable: String,
    pub stream: String,
    pub filter_subjects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitError {
    InvalidSubject {
        subject: String,
        reason: &'static str,
    },
    DuplicateStream {
        stream: String,
    },
    OverlappingStreamFilters {
        left_stream: String,
        left_filter: String,
        right_stream: String,
        right_filter: String,
        involves_work_queue: bool,
    },
    MissingConsumerStream {
        durable: String,
        stream: String,
    },
    ConsumerFilterNotSubset {
        durable: String,
        filter: String,
        stream: String,
        bindings: Vec<String>,
    },
    StoreRetentionTooShort {
        stream: String,
        max_age: Duration,
        store_retention: Duration,
    },
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSubject { subject, reason } => {
                write!(f, "invalid subject filter {subject:?}: {reason}")
            }
            Self::DuplicateStream { stream } => write!(f, "duplicate stream {stream:?}"),
            Self::OverlappingStreamFilters {
                left_stream,
                left_filter,
                right_stream,
                right_filter,
                involves_work_queue,
            } => {
                let queue = if *involves_work_queue {
                    "work-queue "
                } else {
                    ""
                };
                write!(
                    f,
                    "{queue}stream filter overlap: {left_stream} {left_filter:?} overlaps {right_stream} {right_filter:?}"
                )
            }
            Self::MissingConsumerStream { durable, stream } => {
                write!(f, "consumer {durable:?} names unknown stream {stream:?}")
            }
            Self::ConsumerFilterNotSubset {
                durable,
                filter,
                stream,
                bindings,
            } => write!(
                f,
                "consumer {durable:?} filter {filter:?} is not a subset of stream {stream} bindings {bindings:?}"
            ),
            Self::StoreRetentionTooShort {
                stream,
                max_age,
                store_retention,
            } => write!(
                f,
                "stream {stream} max-age {}s exceeds declared store retention {}s",
                max_age.as_secs(),
                store_retention.as_secs()
            ),
        }
    }
}

impl Error for LimitError {}

/// Returns the five normative stream configurations for an account.
pub fn shipped_streams(account: &AccountNames) -> Vec<StreamSpec> {
    let names = account.streams();
    vec![
        StreamSpec {
            kind: StreamKind::Room,
            name: names.room.clone(),
            subjects: vec![account.room_binding()],
            max_age: HOUR * 24,
            max_bytes: GIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
        },
        StreamSpec {
            kind: StreamKind::Wake,
            name: names.wake.clone(),
            subjects: vec![account.wake_binding()],
            max_age: HOUR * 24,
            max_bytes: GIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
        },
        StreamSpec {
            kind: StreamKind::Peer,
            name: names.peer.clone(),
            subjects: vec![account.peer_binding()],
            max_age: HOUR * 24,
            max_bytes: GIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
        },
        StreamSpec {
            kind: StreamKind::Effect,
            name: names.effect.clone(),
            subjects: vec![account.effect_binding()],
            max_age: HOUR * 24,
            max_bytes: 256 * MIB,
            discard: DiscardPolicy::New,
            work_queue: true,
        },
        StreamSpec {
            kind: StreamKind::EffectDead,
            name: names.effect_dead.clone(),
            subjects: vec![account.effect_dead()],
            max_age: HOUR * 24 * 7,
            max_bytes: 64 * MIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
        },
    ]
}

/// Rejects duplicate streams, malformed filters, and every pair of overlapping filters.
pub fn validate_streams(streams: &[StreamSpec]) -> Result<(), LimitError> {
    for (index, stream) in streams.iter().enumerate() {
        if streams[..index]
            .iter()
            .any(|other| other.name == stream.name)
        {
            return Err(LimitError::DuplicateStream {
                stream: stream.name.clone(),
            });
        }
        for subject in &stream.subjects {
            SubjectPattern::parse(subject)?;
        }
    }

    for left_index in 0..streams.len() {
        for right_index in (left_index + 1)..streams.len() {
            let left = &streams[left_index];
            let right = &streams[right_index];
            for left_filter in &left.subjects {
                let left_pattern = SubjectPattern::parse(left_filter)?;
                for right_filter in &right.subjects {
                    let right_pattern = SubjectPattern::parse(right_filter)?;
                    if left_pattern.overlaps(&right_pattern) {
                        return Err(LimitError::OverlappingStreamFilters {
                            left_stream: left.name.clone(),
                            left_filter: left_filter.clone(),
                            right_stream: right.name.clone(),
                            right_filter: right_filter.clone(),
                            involves_work_queue: left.work_queue || right.work_queue,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Rejects a consumer filter unless every subject it can match is in its stream.
pub fn validate_consumer(
    consumer: &ConsumerSpec,
    streams: &[StreamSpec],
) -> Result<(), LimitError> {
    let stream = streams
        .iter()
        .find(|stream| stream.name == consumer.stream)
        .ok_or_else(|| LimitError::MissingConsumerStream {
            durable: consumer.durable.clone(),
            stream: consumer.stream.clone(),
        })?;

    let bindings = stream
        .subjects
        .iter()
        .map(|subject| SubjectPattern::parse(subject))
        .collect::<Result<Vec<_>, _>>()?;
    for filter in &consumer.filter_subjects {
        let filter_pattern = SubjectPattern::parse(filter)?;
        if !bindings
            .iter()
            .any(|binding| filter_pattern.is_subset_of(binding))
        {
            return Err(LimitError::ConsumerFilterNotSubset {
                durable: consumer.durable.clone(),
                filter: filter.clone(),
                stream: stream.name.clone(),
                bindings: stream.subjects.clone(),
            });
        }
    }
    Ok(())
}

/// Enforces that the owning store keeps a body at least as long as its stream.
pub fn validate_store_retention(
    stream: &StreamSpec,
    store_retention: Duration,
) -> Result<(), LimitError> {
    if stream.max_age > store_retention {
        Err(LimitError::StoreRetentionTooShort {
            stream: stream.name.clone(),
            max_age: stream.max_age,
            store_retention,
        })
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct SubjectPattern<'a> {
    tokens: Vec<&'a str>,
}

impl<'a> SubjectPattern<'a> {
    fn parse(raw: &'a str) -> Result<Self, LimitError> {
        if raw.is_empty() {
            return Err(invalid_subject(raw, "subject must not be empty"));
        }
        let tokens = raw.split('.').collect::<Vec<_>>();
        if tokens.iter().any(|token| token.is_empty()) {
            return Err(invalid_subject(raw, "subject contains an empty token"));
        }
        for (index, token) in tokens.iter().enumerate() {
            if token.bytes().any(|byte| byte.is_ascii_whitespace()) {
                return Err(invalid_subject(
                    raw,
                    "subject tokens cannot contain whitespace",
                ));
            }
            if (token.contains('*') && *token != "*") || (token.contains('>') && *token != ">") {
                return Err(invalid_subject(
                    raw,
                    "wildcards must occupy a whole subject token",
                ));
            }
            if *token == ">" && index + 1 != tokens.len() {
                return Err(invalid_subject(
                    raw,
                    "the > wildcard must be the final token",
                ));
            }
        }
        Ok(Self { tokens })
    }

    fn overlaps(&self, other: &Self) -> bool {
        patterns_overlap(&self.tokens, &other.tokens)
    }

    fn is_subset_of(&self, other: &Self) -> bool {
        pattern_subset(&self.tokens, &other.tokens)
    }
}

fn invalid_subject(subject: &str, reason: &'static str) -> LimitError {
    LimitError::InvalidSubject {
        subject: subject.to_owned(),
        reason,
    }
}

fn patterns_overlap(left: &[&str], right: &[&str]) -> bool {
    match (left.first(), right.first()) {
        (None, None) => true,
        (None, Some(_)) | (Some(_), None) => false,
        (Some(&">"), Some(_)) | (Some(_), Some(&">")) => true,
        (Some(left_head), Some(right_head))
            if left_head == right_head || *left_head == "*" || *right_head == "*" =>
        {
            patterns_overlap(&left[1..], &right[1..])
        }
        _ => false,
    }
}

fn pattern_subset(candidate: &[&str], binding: &[&str]) -> bool {
    match (candidate.first(), binding.first()) {
        (None, None) => true,
        (None, Some(_)) | (Some(_), None) => false,
        (Some(_), Some(&">")) => true,
        (Some(&">"), Some(_)) => false,
        (Some(candidate_head), Some(binding_head)) => {
            let head_is_subset = binding_head == candidate_head || *binding_head == "*";
            head_is_subset && pattern_subset(&candidate[1..], &binding[1..])
        }
    }
}
