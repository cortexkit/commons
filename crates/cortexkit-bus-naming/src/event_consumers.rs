use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::{validate_consumer, AccountNames, ConsumerSpec, LimitError, NamingError};

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
        ],
    },
    EventConsumer {
        module_id: "astrocyte",
        durable: "m_astrocyte",
        filter_subjects: &["event.insula.>"],
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
];

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
                    "event-consumer {id:?} needs nonempty filters under event."
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
            ]
        );
    }
}
