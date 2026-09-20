//! Canonical names, topology limits, and allow-only grants for the NATS plane.

mod grants;
mod limits;
mod names;
mod token;

pub use grants::{
    bus_permissions, generate_permission_golden, participant_permissions, system_permissions,
    validate_permission_file, AllowEntry, GoldenFixture, GrantError, Operation, PermissionDocument,
    Principal, RefusedEntry, PINNED_GOLDEN_FIXTURE,
};
pub use limits::{
    shipped_streams, validate_consumer, validate_store_retention, validate_streams, ConsumerSpec,
    DiscardPolicy, LimitError, StreamKind, StreamSpec, GIB, HOUR, MIB,
};
pub use names::{
    validate_tenancy_name, AccountNames, BucketNames, NamingExemption, NamingRule, StreamNames,
    TenancyNameError, CLOSED_NAMING_EXEMPTIONS,
};
pub use token::{validate_account_token, validate_token, NamingError, TokenKind};
