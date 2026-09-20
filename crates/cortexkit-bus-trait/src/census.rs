use std::collections::BTreeSet;

use crate::{BusError, BusResult, RegisterUpdate, WatchEvent};

/// Tracks presence and deletion observations made on this connection as a local safeguard;
/// the server-side credential revocation remains authoritative.
#[derive(Debug, Default, Clone)]
pub struct CensusWatchGuard {
    present: BTreeSet<String>,
    observed_deleted: BTreeSet<String>,
    has_snapshot: bool,
    connection_dropped: bool,
}

impl CensusWatchGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&mut self, event: &WatchEvent) {
        if self.connection_dropped && !matches!(event, WatchEvent::Snapshot { .. }) {
            return;
        }
        match event {
            WatchEvent::Snapshot { entries, .. } => {
                self.connection_dropped = false;
                self.present = entries.iter().map(|entry| entry.key.clone()).collect();
                self.observed_deleted.clear();
                self.has_snapshot = true;
            }
            WatchEvent::Update(RegisterUpdate::Put(entry)) => {
                self.present.insert(entry.key.clone());
                self.observed_deleted.remove(&entry.key);
            }
            WatchEvent::Update(RegisterUpdate::Delete(tombstone)) => {
                self.present.remove(&tombstone.key);
                self.observed_deleted.insert(tombstone.key.clone());
            }
        }
    }

    /// Clears observations after disconnect so missing events are not treated as proof of absence.
    pub fn connection_dropped(&mut self) {
        self.present.clear();
        self.observed_deleted.clear();
        self.has_snapshot = false;
        self.connection_dropped = true;
    }

    pub fn has_snapshot(&self) -> bool {
        self.has_snapshot
    }

    pub fn check_publish(&self, census_key: &str, subject: &str) -> BusResult<()> {
        if self.connection_dropped {
            return Ok(());
        }

        let confirmed_absent = self.observed_deleted.contains(census_key)
            || (self.has_snapshot && !self.present.contains(census_key));
        if confirmed_absent {
            return Err(BusError::denied(
                subject,
                format!("confirmed absent census key {census_key}"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RegisterEntry, Revision, Tombstone};

    #[test]
    fn no_snapshot_and_dropped_connection_are_both_neutral() {
        let mut guard = CensusWatchGuard::new();
        assert_eq!(
            guard.check_publish("module-a", "ck.box.room.r.post"),
            Ok(())
        );

        guard.observe(&WatchEvent::Snapshot {
            entries: Vec::new(),
            revision: Revision(0),
        });
        assert!(matches!(
            guard.check_publish("module-a", "ck.box.room.r.post"),
            Err(BusError::Denied { .. })
        ));

        guard.connection_dropped();
        guard.observe(&WatchEvent::Update(RegisterUpdate::Delete(Tombstone {
            key: "module-a".into(),
            revision: Revision(1),
        })));
        assert_eq!(
            guard.check_publish("module-a", "ck.box.room.r.post"),
            Ok(())
        );
    }

    #[test]
    fn observed_delete_confirms_absence_even_without_a_snapshot() {
        let mut guard = CensusWatchGuard::new();
        guard.observe(&WatchEvent::Update(RegisterUpdate::Delete(Tombstone {
            key: "module-a".into(),
            revision: Revision(2),
        })));

        assert!(matches!(
            guard.check_publish("module-a", "ck.box.wake.agent.fire"),
            Err(BusError::Denied { subject, .. }) if subject == "ck.box.wake.agent.fire"
        ));

        guard.observe(&WatchEvent::Update(RegisterUpdate::Put(RegisterEntry {
            key: "module-a".into(),
            value: Vec::new(),
            revision: Revision(3),
        })));
        assert_eq!(
            guard.check_publish("module-a", "ck.box.wake.agent.fire"),
            Ok(())
        );
    }
}
