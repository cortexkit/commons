use async_trait::async_trait;

use crate::BusResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterValue {
    pub value: Vec<u8>,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterEntry {
    pub key: String,
    pub value: Vec<u8>,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub key: String,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterUpdate {
    Put(RegisterEntry),
    Delete(Tombstone),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// One event contains the complete, possibly empty, connection snapshot.
    Snapshot {
        entries: Vec<RegisterEntry>,
        revision: Revision,
    },
    Update(RegisterUpdate),
}

#[async_trait]
pub trait RegisterWatch: Send {
    async fn next(&mut self) -> BusResult<WatchEvent>;

    /// True only after this watch delivered its snapshot on this connection.
    fn has_snapshot(&self) -> bool;
}

#[async_trait]
pub trait Register: Send + Sync {
    type Watch: RegisterWatch;

    async fn put(&self, key: &str, value: Vec<u8>) -> BusResult<Revision>;
    async fn get(&self, key: &str) -> BusResult<RegisterValue>;
    async fn delete(&self, key: &str) -> BusResult<Tombstone>;
    async fn watch(&self, prefix: &str) -> BusResult<Self::Watch>;
}
