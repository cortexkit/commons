use std::collections::HashMap;

use crate::{Cursor, ReadReply, StepRow};

/// What [`LatestRows::apply`] did with a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Applied {
    /// The first row seen for its identity.
    Added,
    /// A row with the same identity was already kept and is now replaced.
    Replaced,
}

/// The last row per identity (`harness` plus `step_ref`), in the order reads
/// returned them.
///
/// Apply rows in the order the reads return them, which is cursor order:
/// pages oldest first, one read at a time, each from the previous read's
/// [`ReadReply::resume_after`]. A later row for an identity replaces the kept
/// one in place; it is never appended. Re-applying a row already seen is
/// harmless, because it replaces itself. Cursors are never compared.
#[derive(Clone, Debug, Default)]
pub struct LatestRows {
    rows: Vec<StepRow>,
    index: HashMap<(String, String), usize>,
    resume_after: Option<Cursor>,
}

impl LatestRows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps `row` as the latest for its identity.
    pub fn apply(&mut self, row: StepRow) -> Applied {
        let key = (row.harness.clone(), row.step_ref.clone());
        match self.index.get(&key) {
            Some(&slot) => {
                self.rows[slot] = row;
                Applied::Replaced
            }
            None => {
                self.index.insert(key, self.rows.len());
                self.rows.push(row);
                Applied::Added
            }
        }
    }

    /// Applies a reply's rows in order and records where the next read
    /// starts.
    pub fn apply_reply(&mut self, reply: ReadReply) {
        self.resume_after = Some(reply.resume_after().clone());
        for row in reply.rows {
            self.apply(row);
        }
    }

    /// The cursor to pass as the next read's `after`, once a reply was
    /// applied.
    pub fn resume_after(&self) -> Option<&Cursor> {
        self.resume_after.as_ref()
    }

    /// The kept row for an identity.
    pub fn get(&self, harness: &str, step_ref: &str) -> Option<&StepRow> {
        self.index
            .get(&(harness.to_owned(), step_ref.to_owned()))
            .map(|&slot| &self.rows[slot])
    }

    /// Kept rows, one per identity, in the order each identity was first
    /// seen.
    pub fn iter(&self) -> impl Iterator<Item = &StepRow> {
        self.rows.iter()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Forgets every row and the resume cursor, as after `cursor_expired`.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}
