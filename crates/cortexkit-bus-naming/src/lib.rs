//! Canonical names, topology limits, and allow-only grants for the NATS plane.

mod event_consumers;
mod grants;
mod limits;
mod names;
mod token;

pub use event_consumers::{
    delivery_consumer_specs, event_consumer_specs, flow_engine_consumer_spec,
    live_hint_subscriptions, AckPolicy, DeliverPolicy, DeliveryConsumerSpec, EventConsumer,
    EventConsumerError, LiveHintReader, ModuleEventConsumerSpec, EVENT_CONSUMERS,
    LIVE_HINT_READERS,
};
pub use grants::{
    bus_permissions, delivery_authority_permissions, flow_engine_permissions,
    generate_permission_golden, participant_permissions, system_permissions,
    validate_permission_file, AllowEntry, GoldenFixture, GrantError, Operation, PermissionDocument,
    Principal, RefusedEntry, PINNED_GOLDEN_FIXTURE,
};
pub use limits::{
    shipped_streams, validate_body_retention, validate_consumer, validate_consumers,
    validate_store_retention, validate_streams, BodyRetention, ConsumerSpec, DiscardPolicy,
    LimitError, StreamKind, StreamSpec, DEAD_LETTER_DUPLICATE_WINDOW, GIB, HOUR, MIB,
};
pub use names::{
    root_credential_id, validate_tenancy_name, AccountNames, BucketNames, NamingExemption,
    NamingRule, RootCredentialKind, StreamNames, TenancyNameError, CLOSED_NAMING_EXEMPTIONS,
};
pub use token::{
    validate_account_token, validate_event_version, validate_token, NamingError, TokenKind,
};
