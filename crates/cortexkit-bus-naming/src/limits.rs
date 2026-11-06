use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::names::AccountNames;

pub const HOUR: Duration = Duration::from_secs(60 * 60);
pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;

/// Per-subject message cap on the module-event stream. A busy subject can carry
/// 50,000-70,000 events a day; at 100,000 messages that keeps about 1.5 days of
/// replay history (about 30 MB at 300 bytes an event), where 10,000 kept only
/// 3.5-5 hours. The 1 GiB stream byte cap and seven-day age stay the real
/// bounds; this cap keeps one runaway subject from evicting every other
/// subject's history.
const EVENT_MAX_MSGS_PER_SUBJECT: i64 = 100_000;

/// Dead-letter diagnostics suppress same-id republishes for 120 seconds.
/// Longer crash gaps may append another diagnostic; business receipts remain
/// authoritative. Command relays use the server default duplicate window and
/// must keep durable acceptance records keyed by command id and check them
/// before doing the work, rather than rely on broker duplicate history.
pub const DEAD_LETTER_DUPLICATE_WINDOW: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamKind {
    Room,
    Wake,
    Peer,
    Effect,
    EffectDead,
    Event,
    Inbox,
    CoreWork,
    DeliveryDead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardPolicy {
    Old,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StreamSpec {
    pub kind: StreamKind,
    pub name: String,
    pub subjects: Vec<String>,
    /// Zero disables broker age eviction; it is not a zero-length retention.
    pub max_age: Duration,
    pub max_bytes: u64,
    pub discard: DiscardPolicy,
    pub work_queue: bool,
    /// The per-subject message cap (`max_msgs_per_subject`), or `None` to set
    /// none. Only the event stream has one, so one noisy module or event cannot
    /// evict every other subject's history. Other streams leave it unset.
    pub max_msgs_per_subject: Option<i64>,
    /// Maximum serialized message size in bytes, or no stream-specific cap.
    pub max_message_size: Option<i32>,
    /// `None` means the server default, not zero: bootstrap must omit the wire
    /// property when unset. Explicit values bound broker same-id suppression.
    pub duplicate_window: Option<Duration>,
}

impl StreamSpec {
    /// Starts a custom limits-retained stream with no age, byte or message cap.
    /// Bootstrap must set its intended bounds before provisioning it.
    pub fn new(kind: StreamKind, name: String, subjects: Vec<String>) -> Self {
        Self {
            kind,
            name,
            subjects,
            max_age: Duration::ZERO,
            max_bytes: 0,
            discard: DiscardPolicy::Old,
            work_queue: false,
            max_msgs_per_subject: None,
            max_message_size: None,
            duplicate_window: None,
        }
    }
}

/// How a store protects bodies referenced by broker messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodyRetention {
    Finite(Duration),
    /// Bodies stay pinned until terminal business disposition, not merely ack.
    PinnedUntilTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerSpec {
    pub durable: String,
    pub stream: String,
    pub filter_subjects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitError {
    InvalidDurableOwner {
        stream: String,
        durable: String,
    },
    BodyMustBePinned {
        stream: String,
    },
    OverlappingConsumerFilters {
        stream: String,
        left_durable: String,
        right_durable: String,
    },
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
            Self::InvalidDurableOwner { stream, durable } => write!(f, "durable {durable} cannot own a consumer on stream {stream}"),
            Self::BodyMustBePinned { stream } => write!(f, "stream {stream} has no max age; bodies must be pinned until terminal business disposition"),
            Self::OverlappingConsumerFilters { stream, left_durable, right_durable } => write!(f, "overlapping consumer filters on {stream}: {left_durable} and {right_durable}"),
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

/// Returns the normative stream configurations for an account.
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
            max_msgs_per_subject: None,
            max_message_size: None,
            duplicate_window: None,
        },
        StreamSpec {
            kind: StreamKind::Wake,
            name: names.wake.clone(),
            subjects: vec![account.wake_binding()],
            max_age: HOUR * 24,
            max_bytes: GIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
            max_msgs_per_subject: None,
            max_message_size: None,
            duplicate_window: None,
        },
        StreamSpec {
            kind: StreamKind::Peer,
            name: names.peer.clone(),
            subjects: vec![account.peer_binding()],
            max_age: HOUR * 24,
            max_bytes: GIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
            max_msgs_per_subject: None,
            max_message_size: None,
            duplicate_window: None,
        },
        StreamSpec {
            kind: StreamKind::Effect,
            name: names.effect.clone(),
            subjects: vec![account.effect_binding()],
            max_age: Duration::ZERO,
            max_bytes: 256 * MIB,
            discard: DiscardPolicy::New,
            work_queue: true,
            max_msgs_per_subject: None,
            max_message_size: Some(16 * 1024),
            duplicate_window: None,
        },
        StreamSpec {
            kind: StreamKind::EffectDead,
            name: names.effect_dead.clone(),
            subjects: vec![account.effect_dead()],
            max_age: HOUR * 24 * 7,
            max_bytes: 64 * MIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
            max_msgs_per_subject: None,
            max_message_size: None,
            duplicate_window: Some(DEAD_LETTER_DUPLICATE_WINDOW),
        },
        StreamSpec {
            kind: StreamKind::Event,
            name: names.event.clone(),
            subjects: vec![account.event_binding()],
            max_age: HOUR * 24 * 7,
            max_bytes: GIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
            max_msgs_per_subject: Some(EVENT_MAX_MSGS_PER_SUBJECT),
            max_message_size: None,
            duplicate_window: None,
        },
        StreamSpec {
            kind: StreamKind::Inbox,
            name: names.inbox.clone(),
            subjects: account.inbox_bindings(),
            max_age: Duration::ZERO,
            max_bytes: 256 * MIB,
            discard: DiscardPolicy::New,
            work_queue: true,
            max_msgs_per_subject: None,
            max_message_size: Some(16 * 1024),
            duplicate_window: None,
        },
        StreamSpec {
            kind: StreamKind::CoreWork,
            name: names.core_work.clone(),
            subjects: account.core_work_bindings(),
            max_age: Duration::ZERO,
            max_bytes: 256 * MIB,
            discard: DiscardPolicy::New,
            work_queue: true,
            max_msgs_per_subject: None,
            max_message_size: Some(16 * 1024),
            duplicate_window: None,
        },
        StreamSpec {
            kind: StreamKind::DeliveryDead,
            name: names.delivery_dead.clone(),
            subjects: vec![account.delivery_dead()],
            max_age: HOUR * 24 * 7,
            max_bytes: 64 * MIB,
            discard: DiscardPolicy::Old,
            work_queue: false,
            max_msgs_per_subject: None,
            max_message_size: None,
            duplicate_window: Some(DEAD_LETTER_DUPLICATE_WINDOW),
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

    let valid_owner = match stream.kind {
        StreamKind::Wake | StreamKind::Peer | StreamKind::Effect => consumer
            .durable
            .strip_prefix("c_")
            .is_some_and(|id| crate::validate_token(crate::TokenKind::AgentId, id).is_ok()),
        StreamKind::Inbox | StreamKind::CoreWork => consumer.durable == "m_prefrontal-core",
        _ => true,
    };
    if !valid_owner {
        return Err(LimitError::InvalidDurableOwner {
            stream: stream.name.clone(),
            durable: consumer.durable.clone(),
        });
    }

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
    for (index, filter) in consumer.filter_subjects.iter().enumerate() {
        for previous in &consumer.filter_subjects[..index] {
            if subject_filters_overlap(filter, previous)? {
                return Err(LimitError::OverlappingConsumerFilters {
                    stream: consumer.stream.clone(),
                    left_durable: consumer.durable.clone(),
                    right_durable: consumer.durable.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Checks every binding and rejects overlapping readers on a WorkQueue stream.
/// The same durable name on different streams identifies distinct consumers.
pub fn validate_consumers(
    consumers: &[ConsumerSpec],
    streams: &[StreamSpec],
) -> Result<(), LimitError> {
    for (index, consumer) in consumers.iter().enumerate() {
        validate_consumer(consumer, streams)?;
        let stream = streams
            .iter()
            .find(|stream| stream.name == consumer.stream)
            .expect("validated stream");
        if !stream.work_queue {
            continue;
        }
        for previous in consumers[..index]
            .iter()
            .filter(|other| other.stream == consumer.stream)
        {
            for filter in &consumer.filter_subjects {
                for other in &previous.filter_subjects {
                    if subject_filters_overlap(filter, other)? {
                        return Err(LimitError::OverlappingConsumerFilters {
                            stream: consumer.stream.clone(),
                            left_durable: previous.durable.clone(),
                            right_durable: consumer.durable.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn subject_filters_overlap(left: &str, right: &str) -> Result<bool, LimitError> {
    Ok(SubjectPattern::parse(left)?.overlaps(&SubjectPattern::parse(right)?))
}

/// Enforces that the owning store keeps a body at least as long as its stream.
///
/// A module declaring a `resolved` event calls this against the event stream:
/// the event carries only an id, so the module must keep the record for at
/// least as long as the stream keeps the event.
/// No-age streams are refused here; use [`validate_body_retention`] with
/// [`BodyRetention::PinnedUntilTerminal`] for commands pinned by the business store.
pub fn validate_store_retention(
    stream: &StreamSpec,
    store_retention: Duration,
) -> Result<(), LimitError> {
    validate_body_retention(stream, BodyRetention::Finite(store_retention))
}

/// No-age queues require business-lifetime pins; a finite body TTL cannot cover them.
pub fn validate_body_retention(
    stream: &StreamSpec,
    retention: BodyRetention,
) -> Result<(), LimitError> {
    let BodyRetention::Finite(store_retention) = retention else {
        return Ok(());
    };
    if stream.max_age.is_zero() {
        return Err(LimitError::BodyMustBePinned {
            stream: stream.name.clone(),
        });
    }
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
