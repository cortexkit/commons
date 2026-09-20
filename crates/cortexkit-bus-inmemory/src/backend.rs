use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cortexkit_bus_trait::{
    AsyncPermissionViolation, DeliveryToken, Message, RegisterUpdate, RegisterValue, Revision,
};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct WorkQueueConfig {
    pub subject: String,
    pub max_bytes: usize,
    pub max_deliveries: u32,
    pub max_nak_delay: Duration,
}

impl WorkQueueConfig {
    pub fn new(subject: impl Into<String>, max_bytes: usize) -> Self {
        Self {
            subject: subject.into(),
            max_bytes,
            max_deliveries: 5,
            max_nak_delay: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryConfig {
    pub(crate) durables: BTreeSet<(String, String)>,
    pub(crate) denied_subjects: BTreeSet<String>,
    pub(crate) work_queue: Option<WorkQueueConfig>,
}

impl InMemoryConfig {
    pub fn with_durable(mut self, durable: impl Into<String>, identity: impl Into<String>) -> Self {
        self.durables.insert((durable.into(), identity.into()));
        self
    }

    pub fn deny_subject(mut self, subject: impl Into<String>) -> Self {
        self.denied_subjects.insert(subject.into());
        self
    }

    pub fn with_work_queue(mut self, config: WorkQueueConfig) -> Self {
        self.work_queue = Some(config);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    Published { subject: String, id: String },
    Acknowledged { token: DeliveryToken },
    NegativelyAcknowledged { token: DeliveryToken },
    Terminated { token: DeliveryToken },
}

#[derive(Clone)]
pub struct InMemoryBus {
    pub(crate) inner: Arc<Mutex<State>>,
}

impl InMemoryBus {
    pub fn new(config: InMemoryConfig) -> Self {
        let durable_cursors = config
            .durables
            .iter()
            .cloned()
            .map(|key| (key, DurableState::default()))
            .collect();
        Self {
            inner: Arc::new(Mutex::new(State {
                config,
                next_stream_seq: 1,
                stream_messages: Vec::new(),
                durable_cursors,
                register_revision: 0,
                register_values: BTreeMap::new(),
                watchers: Vec::new(),
                queue: VecDeque::new(),
                next_delivery_token: 1,
                events: Vec::new(),
                permission_violations: VecDeque::new(),
            })),
        }
    }

    pub fn events(&self) -> Vec<BackendEvent> {
        self.inner
            .lock()
            .expect("in-memory bus poisoned")
            .events
            .clone()
    }

    pub fn queued_bytes(&self) -> usize {
        self.inner
            .lock()
            .expect("in-memory bus poisoned")
            .queue
            .iter()
            .map(|entry| entry.message.wire_size())
            .sum()
    }

    pub fn inject_permission_violation(&self, subject: impl Into<String>) {
        let subject = subject.into();
        self.inner
            .lock()
            .expect("in-memory bus poisoned")
            .permission_violations
            .push_back(AsyncPermissionViolation {
                subject,
                reason: "fixture-configured permission violation".into(),
            });
    }
}

pub(crate) struct State {
    pub config: InMemoryConfig,
    pub next_stream_seq: u64,
    pub stream_messages: Vec<StoredMessage>,
    pub durable_cursors: HashMap<(String, String), DurableState>,
    pub register_revision: u64,
    pub register_values: BTreeMap<String, RegisterValue>,
    pub watchers: Vec<Watcher>,
    pub queue: VecDeque<QueueEntry>,
    pub next_delivery_token: u64,
    pub events: Vec<BackendEvent>,
    pub permission_violations: VecDeque<AsyncPermissionViolation>,
}

#[derive(Clone)]
pub(crate) struct StoredMessage {
    pub stream_seq: u64,
    pub message: Message,
}

#[derive(Default)]
pub(crate) struct DurableState {
    pub next_index: usize,
    pub available_at: Option<Instant>,
}

pub(crate) struct Watcher {
    pub prefix: String,
    pub sender: mpsc::UnboundedSender<RegisterUpdate>,
}

pub(crate) struct QueueEntry {
    pub token: DeliveryToken,
    pub message: Message,
    pub delivery_count: u32,
    pub available_at: Instant,
    pub in_flight: bool,
}

impl State {
    pub fn next_revision(&mut self) -> Revision {
        self.register_revision += 1;
        Revision(self.register_revision)
    }

    pub fn notify_watchers(&mut self, update: RegisterUpdate) {
        let key = match &update {
            RegisterUpdate::Put(entry) => &entry.key,
            RegisterUpdate::Delete(tombstone) => &tombstone.key,
        };
        self.watchers.retain(|watcher| {
            !key.starts_with(&watcher.prefix) || watcher.sender.send(update.clone()).is_ok()
        });
    }
}
