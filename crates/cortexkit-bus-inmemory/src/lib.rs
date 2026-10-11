//! Process-local implementation of the NATS-shaped bus traits.
//!
//! State intentionally disappears when the final [`InMemoryBus`] handle is
//! dropped. Server enforcement, leaves, and system events are not emulated.

mod backend;
mod conformance;
mod register;
mod stream;
mod work_queue;

pub use backend::{
    BackendEvent, InMemoryBus, InMemoryConfig, WorkQueueConfig, DEFAULT_DEDUPLICATION_WINDOW,
    DEFAULT_STREAM_ACK_WAIT,
};
pub use conformance::InMemoryConformance;
pub use register::InMemoryRegisterWatch;
pub use stream::InMemoryStreamCursor;
