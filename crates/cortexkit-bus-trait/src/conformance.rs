use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    InMemory,
    Nats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertySet {
    Parity,
    NatsOnly,
}

impl PropertySet {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Parity => "parity",
            Self::NatsOnly => "nats-only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertySpec {
    pub name: &'static str,
    pub set: PropertySet,
}

pub const CONFORMANCE_PROPERTIES: &[PropertySpec] = &[
    PropertySpec {
        name: "publish_ack_monotonic_sequence",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "consumer_replay_within_process_lifetime",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "register_revision",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "register_absent_vs_empty",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "watch_snapshot_then_updates_with_connection_flag",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "work_redelivery_count",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "work_cap_exhaustion_and_dead_letter",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "full_work_queue_names_exhausted_limit",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "error_taxonomy_and_boundary_mapping",
        set: PropertySet::Parity,
    },
    PropertySpec {
        name: "cursor_survives_process_restart",
        set: PropertySet::NatsOnly,
    },
    PropertySpec {
        name: "cursor_survives_credential_rotation",
        set: PropertySet::NatsOnly,
    },
    PropertySpec {
        name: "cursor_survives_device_rekey",
        set: PropertySet::NatsOnly,
    },
    PropertySpec {
        name: "server_enforced_grant_denial",
        set: PropertySet::NatsOnly,
    },
    PropertySpec {
        name: "reconnect_refusal_after_revocation",
        set: PropertySet::NatsOnly,
    },
    PropertySpec {
        name: "asynchronous_permission_violation_and_sentinel_attribution",
        set: PropertySet::NatsOnly,
    },
    PropertySpec {
        name: "leaf_behaviour",
        set: PropertySet::NatsOnly,
    },
    PropertySpec {
        name: "system_events",
        set: PropertySet::NatsOnly,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyApplicability {
    Applicable,
    NotApplicable { reason: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyClassification {
    pub property: PropertySpec,
    pub applicability: PropertyApplicability,
}

pub fn classify_properties(backend: BackendKind) -> Vec<PropertyClassification> {
    CONFORMANCE_PROPERTIES
        .iter()
        .copied()
        .map(|property| {
            let applicability = match (backend, property.set) {
                (BackendKind::InMemory, PropertySet::NatsOnly) => {
                    PropertyApplicability::NotApplicable {
                        reason:
                            "in-memory has no persisted process state, leaf, or server enforcement",
                    }
                }
                _ => PropertyApplicability::Applicable,
            };
            PropertyClassification {
                property,
                applicability,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyOutcome {
    Passed,
    Failed { reason: String },
    NotApplicable { reason: String },
}

#[async_trait]
pub trait ConformanceBackend: Send + Sync {
    fn backend_kind(&self) -> BackendKind;
    async fn probe(&self, property: PropertySpec) -> PropertyOutcome;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceReport {
    pub backend: BackendKind,
    pub property_set: PropertySet,
    pub property_set_name: &'static str,
    pub outcomes: Vec<(PropertySpec, PropertyOutcome)>,
}

impl ConformanceReport {
    pub fn passed(&self) -> bool {
        self.outcomes
            .iter()
            .all(|(_, outcome)| matches!(outcome, PropertyOutcome::Passed))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiteBug {
    NatsOnlyAssertedAgainstInMemory,
    ApplicablePropertyReportedNotApplicable { property: &'static str },
}

pub async fn run_conformance<B: ConformanceBackend>(
    backend: &B,
    property_set: PropertySet,
) -> Result<ConformanceReport, SuiteBug> {
    if backend.backend_kind() == BackendKind::InMemory && property_set == PropertySet::NatsOnly {
        return Err(SuiteBug::NatsOnlyAssertedAgainstInMemory);
    }

    let mut outcomes = Vec::new();
    for property in CONFORMANCE_PROPERTIES
        .iter()
        .copied()
        .filter(|property| property.set == property_set)
    {
        let outcome = backend.probe(property).await;
        if matches!(outcome, PropertyOutcome::NotApplicable { .. }) {
            return Err(SuiteBug::ApplicablePropertyReportedNotApplicable {
                property: property.name,
            });
        }
        outcomes.push((property, outcome));
    }

    Ok(ConformanceReport {
        backend: backend.backend_kind(),
        property_set,
        property_set_name: property_set.name(),
        outcomes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct WeakenedNats;

    #[async_trait]
    impl ConformanceBackend for WeakenedNats {
        fn backend_kind(&self) -> BackendKind {
            BackendKind::Nats
        }

        async fn probe(&self, property: PropertySpec) -> PropertyOutcome {
            match property.set {
                PropertySet::Parity => PropertyOutcome::Passed,
                PropertySet::NatsOnly => PropertyOutcome::Failed {
                    reason: "fixture intentionally has only parity semantics".into(),
                },
            }
        }
    }

    #[tokio::test]
    async fn weakened_nats_fixture_passes_parity_but_fails_nats_only() {
        let parity = run_conformance(&WeakenedNats, PropertySet::Parity)
            .await
            .expect("parity run should be valid");
        let nats_only = run_conformance(&WeakenedNats, PropertySet::NatsOnly)
            .await
            .expect("NATS-only run should be valid for a NATS fixture");

        assert_eq!(parity.property_set_name, "parity");
        assert!(parity.passed());
        assert_eq!(nats_only.property_set_name, "nats-only");
        assert!(!nats_only.passed());
    }

    #[test]
    fn in_memory_classifies_every_nats_only_property_as_not_applicable() {
        let classifications = classify_properties(BackendKind::InMemory);
        for row in classifications
            .iter()
            .filter(|row| row.property.set == PropertySet::NatsOnly)
        {
            assert!(matches!(
                row.applicability,
                PropertyApplicability::NotApplicable { .. }
            ));
        }
    }
}
