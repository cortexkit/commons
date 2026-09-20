//! NATS-shaped primitives shared by the production and in-memory bus backends.
//!
//! The broker transports identifiers and content digests, never record bodies.
//! The traits intentionally model the selected NATS primitives rather than a
//! least-common-denominator queue API.

mod census;
mod conformance;
mod error;
mod message;
mod register;
mod stream;
mod work_queue;

pub use census::CensusWatchGuard;
pub use conformance::{
    classify_properties, run_conformance, BackendKind, ConformanceBackend, ConformanceReport,
    PropertyApplicability, PropertyClassification, PropertyOutcome, PropertySet, PropertySpec,
    SuiteBug, CONFORMANCE_PROPERTIES,
};
pub use error::{
    attribute_sentinel_failure, map_connection_failure, map_credential_sign_failure,
    AsyncPermissionViolation, BusError, BusResult, ConnectionFailure, CredentialSignFailure,
    PermissionViolationSource,
};
pub use message::{ContentDigest, Headers, Message, MessageId};
pub use register::{
    Register, RegisterEntry, RegisterUpdate, RegisterValue, RegisterWatch, Revision, Tombstone,
    WatchEvent,
};
pub use stream::{PublishAck, Stream, StreamCursor, StreamDelivery};
pub use work_queue::{
    effect_dead_subject, terminally_dispose, ClaimOutcome, DeadLetterRecord, DeliveryToken,
    MaxDeliveriesExceeded, WorkItem, WorkQueue, DEAD_LETTER_REASON_MAX_DELIVERIES,
};
