use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::{
    validate_consumer, validate_event_version, validate_token, AccountNames, ConsumerSpec,
    LimitError, NamingError, TokenKind,
};

/// One entry of the event-consumer table: a module allowed to read a filtered
/// slice of other modules' events through its own durable. Filters are relative
/// to `ck.{account}.`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventConsumer {
    pub module_id: &'static str,
    pub durable: &'static str,
    pub filter_subjects: &'static [&'static str],
}

impl EventConsumer {
    /// Uses the same module-durable namespace as the flow engine's `m_basal`.
    pub fn durable_name(&self) -> Result<String, NamingError> {
        AccountNames::module_consumer_name(self.module_id)
    }
}

/// Who may read another module's events is a security decision, reviewed in
/// code. Runtime configuration must not extend this closed table or its filters.
pub const EVENT_CONSUMERS: &[EventConsumer] = &[
    EventConsumer {
        module_id: "prefrontal-core",
        durable: "m_prefrontal-core",
        filter_subjects: &[
            "event.broca.>",
            "event.basal.flow_instance_created.v1",
            "event.basal.flow_instance_extended.v1",
            "event.basal.flow_instance_ended.v1",
            "event.basal.flow_instance_expired.v1",
            "event.basal.flow_instance_rescheduled.v1",
        ],
    },
    EventConsumer {
        module_id: "astrocyte",
        durable: "m_astrocyte",
        filter_subjects: &[
            "event.insula.>",
            "event.broca.step.finished.v1",
            "event.thalamus-harness.step.finished.v1",
            "event.thalamus.step.finished.v1",
        ],
    },
    EventConsumer {
        module_id: "prefrontal-routing",
        durable: "m_prefrontal-routing",
        filter_subjects: &["event.insula.>"],
    },
    EventConsumer {
        module_id: "magic-context",
        durable: "m_magic-context",
        filter_subjects: &[
            "event.thalamus-harness.compaction.>",
            "event.thalamus.compaction.>",
            "event.broca.compaction.>",
        ],
    },
    EventConsumer {
        module_id: "thalamus-harness",
        durable: "m_thalamus-harness",
        filter_subjects: &["event.magic-context.compaction.>"],
    },
    EventConsumer {
        module_id: "thalamus",
        durable: "m_thalamus",
        filter_subjects: &["event.magic-context.compaction.>"],
    },
    EventConsumer {
        module_id: "broca",
        durable: "m_broca",
        filter_subjects: &["event.magic-context.compaction.>"],
    },
    // Intended for the browser and computer-use module's cursor badge, which
    // it clears when a runner's turn ends; turn starts let it ignore a late end
    // from an older turn. This entry grants the durable EVENT reader only. The
    // bus module derives live-hint subscriptions from these filters, and when
    // to subscribe is the consumer's own behaviour.
    EventConsumer {
        module_id: "cerebellum",
        durable: "m_cerebellum",
        filter_subjects: &[
            "event.thalamus-harness.turn_started.v1",
            "event.thalamus-harness.turn_ended.v1",
        ],
    },
];

/// A module allowed to subscribe to exact live hints without gaining an EVENT
/// durable, EVENT-stream reads, or consumer create/info permissions. Subjects
/// use relative `event.{module}.{name}.v{version}` spelling to identify the hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LiveHintReader {
    pub module_id: &'static str,
    pub subjects: &'static [&'static str],
}

impl LiveHintReader {
    pub const fn new(module_id: &'static str, subjects: &'static [&'static str]) -> Self {
        Self {
            module_id,
            subjects,
        }
    }
}

/// Who may observe another module's hints is a security decision reviewed in
/// code. Runtime configuration cannot extend this closed table. Entries grant
/// live-hint subscribe permission only, independently of event durables.
pub const LIVE_HINT_READERS: &[LiveHintReader] = &[LiveHintReader::new(
    "prefrontal-routing",
    &[
        "event.broca.step.finished.v1",
        "event.thalamus-harness.step.finished.v1",
    ],
)];

/// Sorted, deduplicated live subscriptions from both reviewed reader tables.
/// EVENT filters retain their wildcard scope under `ck.{account}.live.`; the
/// live-only table adds exact hints without changing any durable or EVENT grant.
/// A valid module absent from both tables has no live subscriptions.
pub fn live_hint_subscriptions(
    account: &AccountNames,
    module_id: &str,
) -> Result<Vec<String>, NamingError> {
    validate_token(TokenKind::ModuleId, module_id)?;
    validate_live_hint_readers(LIVE_HINT_READERS, EVENT_CONSUMERS)?;
    let subjects = EVENT_CONSUMERS
        .iter()
        .filter(|entry| entry.module_id == module_id)
        .flat_map(|entry| entry.filter_subjects)
        .chain(
            LIVE_HINT_READERS
                .iter()
                .filter(|entry| entry.module_id == module_id)
                .flat_map(|entry| entry.subjects),
        )
        .map(|subject| {
            let suffix = subject.strip_prefix("event.").ok_or_else(|| {
                NamingError::new(
                    TokenKind::EventName,
                    subject,
                    "expected relative event subject",
                )
            })?;
            Ok(format!("ck.{}.live.{suffix}", account.account()))
        })
        .collect::<Result<BTreeSet<_>, NamingError>>()?;
    Ok(subjects.into_iter().collect())
}

fn validate_live_hint_readers(
    entries: &[LiveHintReader],
    event_consumers: &[EventConsumer],
) -> Result<(), NamingError> {
    let mut modules = BTreeSet::new();
    for entry in entries {
        validate_token(TokenKind::ModuleId, entry.module_id)?;
        if !modules.insert(entry.module_id) {
            return Err(NamingError::new(
                TokenKind::ModuleId,
                entry.module_id,
                "duplicate live-hint reader module",
            ));
        }
        if entry.subjects.is_empty() {
            return Err(NamingError::new(
                TokenKind::EventName,
                entry.module_id,
                "live-hint reader needs at least one subject",
            ));
        }
        let mut subjects = BTreeSet::new();
        for subject in entry.subjects {
            validate_live_hint_subject(subject)?;
            if !subjects.insert(subject) {
                return Err(NamingError::new(
                    TokenKind::EventName,
                    subject,
                    "duplicate live-hint reader subject",
                ));
            }
            for filter in event_consumers
                .iter()
                .filter(|consumer| consumer.module_id == entry.module_id)
                .flat_map(|consumer| consumer.filter_subjects)
            {
                let overlaps =
                    crate::limits::subject_filters_overlap(subject, filter).map_err(|_| {
                        NamingError::new(
                            TokenKind::EventName,
                            filter,
                            "invalid event-consumer filter",
                        )
                    })?;
                if overlaps {
                    return Err(NamingError::new(
                        TokenKind::EventName,
                        subject,
                        "live-hint subject is already covered by the module's EVENT filters",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_live_hint_subject(subject: &str) -> Result<(), NamingError> {
    let refuse = || {
        NamingError::new(
            TokenKind::EventName,
            subject,
            "expected exact event.{module}.{name}.v{version} subject",
        )
    };
    let relative = subject.strip_prefix("event.").ok_or_else(refuse)?;
    let (module, event_version) = relative.split_once('.').ok_or_else(refuse)?;
    let (event, version) = event_version.rsplit_once('.').ok_or_else(refuse)?;
    validate_token(TokenKind::ModuleId, module)?;
    validate_token(TokenKind::EventName, event)?;
    validate_event_version(version)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckPolicy {
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliverPolicy {
    All,
}

/// Configuration ck-bus bootstrap uses to create a durable pull reader of module
/// events. Every reader, including the flow engine's `m_basal`, gets the same
/// settings: pull, explicit ack, deliver-all, 30 s ack wait, 1,000 max pending.
///
/// The topology-only `ConsumerSpec` stays unchanged for existing callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleEventConsumerSpec {
    pub consumer: ConsumerSpec,
    pub pull: bool,
    pub ack_policy: AckPolicy,
    pub deliver_policy: DeliverPolicy,
    pub ack_wait: Duration,
    pub max_ack_pending: i64,
}

/// Bootstrap policy for reliable command readers and bounded dead-letter diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeliveryConsumerSpec {
    pub consumer: ConsumerSpec,
    pub pull: bool,
    pub ack_policy: AckPolicy,
    pub deliver_policy: DeliverPolicy,
    pub ack_wait: Duration,
    /// -1 is unlimited: the application owns terminal disposition.
    pub max_deliver: i64,
    pub max_ack_pending: i64,
}

/// Core owns distinct module durables on INBOX and CORE_WORK. Diagnostics use
/// `c_ckbus_dead` on each dead-letter stream, never an agent stream.
pub fn delivery_consumer_specs(
    account: &AccountNames,
) -> Result<Vec<DeliveryConsumerSpec>, crate::LimitError> {
    let names = account.streams();
    let specs = [
        (
            names.inbox.clone(),
            "m_prefrontal-core",
            account.inbox_bindings(),
            1,
        ),
        (
            names.core_work.clone(),
            "m_prefrontal-core",
            account.core_work_bindings(),
            1,
        ),
        (
            names.effect_dead.clone(),
            "c_ckbus_dead",
            vec![account.effect_dead()],
            1_000,
        ),
        (
            names.delivery_dead.clone(),
            "c_ckbus_dead",
            vec![account.delivery_dead()],
            1_000,
        ),
    ]
    .into_iter()
    .map(
        |(stream, durable, filter_subjects, max_ack_pending)| DeliveryConsumerSpec {
            consumer: ConsumerSpec {
                stream,
                durable: durable.into(),
                filter_subjects,
            },
            pull: true,
            ack_policy: AckPolicy::Explicit,
            deliver_policy: DeliverPolicy::All,
            ack_wait: Duration::from_secs(30),
            max_deliver: -1,
            max_ack_pending,
        },
    )
    .collect::<Vec<_>>();
    crate::validate_consumers(
        &specs
            .iter()
            .map(|spec| spec.consumer.clone())
            .collect::<Vec<_>>(),
        &crate::shipped_streams(account),
    )?;
    Ok(specs)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventConsumerError {
    Naming(NamingError),
    Limits(LimitError),
    DuplicateModule(&'static str),
    DuplicateDurable(&'static str),
    DurableMismatch(&'static str),
    InvalidFilters(String),
}

impl fmt::Display for EventConsumerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Naming(error) => error.fmt(f),
            Self::Limits(error) => error.fmt(f),
            Self::DuplicateModule(id) => write!(f, "duplicate event-consumer module {id:?}"),
            Self::DuplicateDurable(name) => write!(f, "duplicate event-consumer durable {name:?}"),
            Self::DurableMismatch(id) => write!(f, "event-consumer durable must be m_{id}"),
            Self::InvalidFilters(id) => {
                write!(
                    f,
                    "event-consumer {id:?} needs nonempty, non-overlapping filters under event."
                )
            }
        }
    }
}

impl Error for EventConsumerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Naming(error) => Some(error),
            Self::Limits(error) => Some(error),
            _ => None,
        }
    }
}

/// The flow engine's broad event reader, using the same pull settings as the
/// filtered readers. Bootstrap owns creation; grants only permit durable reads.
pub fn flow_engine_consumer_spec(
    account: &AccountNames,
    module_id: &str,
) -> Result<ModuleEventConsumerSpec, EventConsumerError> {
    module_event_spec(account, module_id, &["event.>"])
}

/// One bootstrap spec for every reviewed reader, scoped to this account.
pub fn event_consumer_specs(
    account: &AccountNames,
) -> Result<Vec<ModuleEventConsumerSpec>, EventConsumerError> {
    validate_table(EVENT_CONSUMERS)?;
    EVENT_CONSUMERS
        .iter()
        .map(|entry| module_event_spec(account, entry.module_id, entry.filter_subjects))
        .collect()
}

fn validate_table(entries: &[EventConsumer]) -> Result<(), EventConsumerError> {
    let mut modules = BTreeSet::new();
    let mut durables = BTreeSet::new();
    for entry in entries {
        if !modules.insert(entry.module_id) {
            return Err(EventConsumerError::DuplicateModule(entry.module_id));
        }
        if !durables.insert(entry.durable) {
            return Err(EventConsumerError::DuplicateDurable(entry.durable));
        }
        if entry.durable_name().map_err(EventConsumerError::Naming)? != entry.durable {
            return Err(EventConsumerError::DurableMismatch(entry.module_id));
        }
        validate_filters(entry.module_id, entry.filter_subjects)?;
    }
    Ok(())
}

fn validate_filters(module_id: &str, filters: &[&str]) -> Result<(), EventConsumerError> {
    if filters.is_empty()
        || filters
            .iter()
            .any(|filter| !filter.starts_with("event.") || filter.len() == "event.".len())
    {
        return Err(EventConsumerError::InvalidFilters(module_id.to_owned()));
    }
    // NATS rejects overlapping filters on one durable, including duplicates.
    // Check each module's static list without comparing different consumers.
    for (index, filter) in filters.iter().enumerate() {
        for previous in filters.iter().take(index) {
            if crate::limits::subject_filters_overlap(previous, filter)
                .map_err(EventConsumerError::Limits)?
            {
                return Err(EventConsumerError::InvalidFilters(module_id.to_owned()));
            }
        }
    }
    Ok(())
}

fn module_event_spec(
    account: &AccountNames,
    module_id: &str,
    filters: &[&str],
) -> Result<ModuleEventConsumerSpec, EventConsumerError> {
    validate_filters(module_id, filters)?;
    let consumer = ConsumerSpec {
        durable: AccountNames::module_consumer_name(module_id)
            .map_err(EventConsumerError::Naming)?,
        stream: account.streams().event.clone(),
        filter_subjects: filters
            .iter()
            .map(|filter| format!("ck.{}.{filter}", account.account()))
            .collect(),
    };
    validate_consumer(&consumer, &crate::shipped_streams(account))
        .map_err(EventConsumerError::Limits)?;
    Ok(ModuleEventConsumerSpec {
        consumer,
        pull: true,
        ack_policy: AckPolicy::Explicit,
        deliver_policy: DeliverPolicy::All,
        ack_wait: Duration::from_secs(30),
        max_ack_pending: 1_000,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_hint_reader_table_rejects_duplicate_modules() {
        let reader = LIVE_HINT_READERS[0];
        let error = validate_live_hint_readers(&[reader, reader], EVENT_CONSUMERS).unwrap_err();
        assert_eq!(error.kind(), TokenKind::ModuleId);
        assert_eq!(error.token(), "prefrontal-routing");
        assert_eq!(error.reason(), "duplicate live-hint reader module");
    }

    #[test]
    fn live_hint_reader_table_rejects_cross_module_wildcards() {
        for subjects in [&["event.*.step.finished.v1"][..], &["event.>"][..]] {
            let reader = LiveHintReader::new("reader", subjects);
            assert!(validate_live_hint_readers(&[reader], EVENT_CONSUMERS).is_err());
        }
    }

    #[test]
    fn live_hint_reader_table_rejects_overlap_with_own_event_filters() {
        for reader in [
            LiveHintReader::new("astrocyte", &["event.broca.step.finished.v1"]),
            LiveHintReader::new("prefrontal-routing", &["event.insula.quota_sampled.v1"]),
        ] {
            let error = validate_live_hint_readers(&[reader], EVENT_CONSUMERS).unwrap_err();
            assert_eq!(error.token(), reader.subjects[0]);
            assert_eq!(
                error.reason(),
                "live-hint subject is already covered by the module's EVENT filters"
            );
        }
    }

    #[test]
    fn live_hint_reader_table_requires_nonempty_unique_exact_subjects() {
        for subjects in [
            &[][..],
            &["live.broca.step.finished.v1"][..],
            &["event.broca.>"][..],
            &["event.broca.step.*.v1"][..],
            &["event.broca.step.finished.v0"][..],
            &["event.broca.step.finished.v01"][..],
            &["event.broca.step.finished"][..],
            &["event.broca.step.v1.finished.v1"][..],
            &[
                "event.broca.step.finished.v1",
                "event.broca.step.finished.v1",
            ][..],
        ] {
            let reader = LiveHintReader::new("reader", subjects);
            assert!(
                validate_live_hint_readers(&[reader], EVENT_CONSUMERS).is_err(),
                "{subjects:?}"
            );
        }
        let reader = LiveHintReader::new("reader", &["event.broca.step.finished.v1"]);
        validate_live_hint_readers(&[reader], EVENT_CONSUMERS).unwrap();
        let invalid_module = LiveHintReader::new("*", reader.subjects);
        assert_eq!(
            validate_live_hint_readers(&[invalid_module], EVENT_CONSUMERS)
                .unwrap_err()
                .kind(),
            TokenKind::ModuleId
        );
    }

    #[test]
    fn event_consumer_table_rejects_duplicate_modules() {
        let entry = EVENT_CONSUMERS[0];
        assert_eq!(
            validate_table(&[
                entry,
                EventConsumer {
                    durable: "m_other",
                    ..entry
                }
            ]),
            Err(EventConsumerError::DuplicateModule("prefrontal-core"))
        );
    }

    #[test]
    fn event_consumer_table_rejects_duplicate_durables() {
        let entry = EVENT_CONSUMERS[0];
        assert_eq!(
            validate_table(&[
                entry,
                EventConsumer {
                    module_id: "other",
                    ..entry
                }
            ]),
            Err(EventConsumerError::DuplicateDurable("m_prefrontal-core"))
        );
    }

    #[test]
    fn event_consumer_table_requires_canonical_durable_names() {
        let entry = EventConsumer {
            durable: "m_other",
            ..EVENT_CONSUMERS[0]
        };
        assert_eq!(
            validate_table(&[entry]),
            Err(EventConsumerError::DurableMismatch("prefrontal-core"))
        );
    }

    #[test]
    fn event_consumer_filters_must_be_nonempty_and_under_event() {
        for filters in [
            &[][..],
            &["peer.>"][..],
            &["event"][..],
            &["event."][..],
            &["> "][..],
        ] {
            let entry = EventConsumer {
                filter_subjects: filters,
                ..EVENT_CONSUMERS[0]
            };
            assert!(matches!(
                validate_table(&[entry]),
                Err(EventConsumerError::InvalidFilters(_))
            ));
        }
        let names = AccountNames::derive("box_readers").unwrap();
        assert!(module_event_spec(&names, "reader", &["event.broca.>.run"]).is_err());
    }

    #[test]
    fn event_consumer_filters_refuse_overlap_within_one_module() {
        let names = AccountNames::derive("box_readers").unwrap();
        for filters in [
            &["event.insula.>", "event.*.step.finished.v1"][..],
            &["event.*.step.finished.v1", "event.insula.>"][..],
            &["event.runner_a.>", "event.runner_a.step.finished.v1"][..],
            &[
                "event.runner_a.step.finished.v1",
                "event.runner_a.step.finished.v1",
            ][..],
        ] {
            assert_eq!(
                module_event_spec(&names, "reader", filters),
                Err(EventConsumerError::InvalidFilters("reader".into())),
                "{filters:?}"
            );
        }
        assert!(module_event_spec(
            &names,
            "reader",
            &["event.insula.>", "event.runner_a.step.finished.v1"]
        )
        .is_ok());
    }

    #[test]
    fn prefrontal_core_filter_is_exact_for_basal_lifecycle_events() {
        let names = AccountNames::derive("box_readers").unwrap();
        let reader = event_consumer_specs(&names)
            .unwrap()
            .into_iter()
            .find(|spec| spec.consumer.durable == "m_prefrontal-core")
            .expect("prefrontal-core reader is in the reviewed table");

        assert_eq!(
            reader.consumer.filter_subjects,
            [
                "ck.box_readers.event.broca.>",
                "ck.box_readers.event.basal.flow_instance_created.v1",
                "ck.box_readers.event.basal.flow_instance_extended.v1",
                "ck.box_readers.event.basal.flow_instance_ended.v1",
                "ck.box_readers.event.basal.flow_instance_expired.v1",
                "ck.box_readers.event.basal.flow_instance_rescheduled.v1",
            ]
        );
    }
}
