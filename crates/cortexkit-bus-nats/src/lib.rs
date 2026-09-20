//! Production message-plane backend built on `async-nats` and JetStream.
//!
//! Workload consumers are deliberately bind-only. Stream and consumer creation
//! belong to the separately privileged plane provisioner.

mod codec;
mod connection;
mod error;
mod register;
mod stream;
mod work_queue;

pub use connection::{ConnectConfig, NatsConnection, NatsPermissionViolations};
pub use register::{NatsRegister, NatsRegisterWatch};
pub use stream::{NatsStream, NatsStreamCursor};
pub use work_queue::NatsWorkQueue;

/// Lowest `async-nats` release accepted by this backend's manifest.
pub const ASYNC_NATS_VERSION_FLOOR: &str = "0.50";
pub const INBOX_ROOT: &str = "_INBOX";

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;
