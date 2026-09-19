//! Date-stamped log segments: `<dir>/<module_id>.<YYYY-MM-DD>.log`.
//!
//! The file is named by the UTC calendar day of each write and is NEVER
//! renamed. Every process that logs for a module — the supervised process and
//! every harness-hosted plugin — computes the same name from the same clock
//! and opens it `O_APPEND`, so any number of writers share one segment with
//! no lock and no coordinator: the kernel lands each `write(2)` at a line
//! boundary, and midnight rolls the name for everyone at once.
//!
//! This is what makes one-file-per-module safe. The r1 design split plugins
//! into per-harness files because rename-based rotation cannot be shared: a
//! writer still holding the old descriptor keeps appending into `.log.1`,
//! silently, because its writes succeed. Removing the rename removes the race.
//!
//! What it costs, accepted deliberately: no size cap within a day. A segment
//! that crosses `alarm_segment_mb` is reported once, because a module writing
//! that much in a day has a defect worth surfacing and truncating it would
//! hide the defect.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, NaiveDate, Utc};

/// Retention for date segments: an age window pruned by the writer, and a
/// per-segment size at which the writer raises an alarm rather than rotating.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentRetention {
    /// Segments strictly older than `today - max_age_days` are unlinked at
    /// process start and at each day roll. `0` keeps today only.
    pub max_age_days: u32,
    /// Size in mebibytes at which today's segment is reported as oversized,
    /// once per process. Never truncates.
    pub alarm_segment_mb: u32,
}

impl Default for SegmentRetention {
    fn default() -> Self {
        Self {
            max_age_days: 14,
            alarm_segment_mb: 256,
        }
    }
}

/// The filename for `module_id`'s segment on the UTC day containing `at`.
pub fn segment_name(module_id: &str, at: SystemTime) -> String {
    format!("{module_id}.{}.log", utc_day(at))
}

/// Parses `<module_id>.<YYYY-MM-DD>.log` back into its day, or `None` for any
/// name that is not exactly that shape — including the r1 pid-suffixed files
/// and a malformed date, which are left alone rather than guessed at.
pub fn segment_day(module_id: &str, file_name: &str) -> Option<NaiveDate> {
    let rest = file_name.strip_prefix(module_id)?.strip_prefix('.')?;
    let day = rest.strip_suffix(".log")?;
    if day.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

/// Segments older than the window, decided by filename alone. Names that are
/// not this module's segments are never returned.
pub fn prune_candidates<'a>(
    module_id: &str,
    today: NaiveDate,
    retention: SegmentRetention,
    present: impl IntoIterator<Item = &'a str>,
) -> Vec<&'a str> {
    let boundary = today
        .checked_sub_days(chrono::Days::new(u64::from(retention.max_age_days)))
        .unwrap_or(today);
    present
        .into_iter()
        .filter(|name| segment_day(module_id, name).is_some_and(|day| day < boundary))
        .collect()
}

fn utc_day(at: SystemTime) -> NaiveDate {
    DateTime::<Utc>::from(at).date_naive()
}

pub(crate) struct SegmentDestination {
    dir: PathBuf,
    module_id: String,
    retention: SegmentRetention,
    open: Option<(NaiveDate, File)>,
    alarmed: bool,
}

/// What a write reported beyond success: the caller decides how to surface it.
pub(crate) enum SegmentNotice {
    /// Today's segment crossed `alarm_segment_mb`. Reported at most once.
    Oversized { path: PathBuf, bytes: u64 },
    /// A day roll or first open pruned aged segments.
    Pruned { removed: usize, kept: usize },
}

impl SegmentDestination {
    pub(crate) fn open(
        dir: &Path,
        module_id: &str,
        retention: SegmentRetention,
        now: SystemTime,
        enforce_directory_mode: bool,
    ) -> io::Result<(Self, Option<SegmentNotice>)> {
        prepare_dir(dir, enforce_directory_mode)?;
        let mut destination = Self {
            dir: dir.to_owned(),
            module_id: module_id.to_owned(),
            retention,
            open: None,
            alarmed: false,
        };
        let notice = destination.roll_to(utc_day(now))?;
        Ok((destination, notice))
    }

    /// Writes one framed line to the segment for `now`, reopening on a day
    /// roll. Returns any notice the write produced.
    pub(crate) fn write(
        &mut self,
        bytes: &[u8],
        now: SystemTime,
    ) -> io::Result<Option<SegmentNotice>> {
        let today = utc_day(now);
        let mut notice = None;
        if self.open.as_ref().map(|(day, _)| *day) != Some(today) {
            notice = self.roll_to(today)?;
        }
        let (_, file) = self
            .open
            .as_mut()
            .ok_or_else(|| io::Error::other("log segment is not open"))?;
        file.write_all(bytes)?;

        if !self.alarmed {
            let len = file.metadata()?.len();
            let cap = u64::from(self.retention.alarm_segment_mb) * 1024 * 1024;
            if len > cap {
                self.alarmed = true;
                notice = Some(SegmentNotice::Oversized {
                    path: self.dir.join(segment_name(&self.module_id, now)),
                    bytes: len,
                });
            }
        }
        Ok(notice)
    }

    // Opening a segment also prunes: the writer is the only party that runs
    // unconditionally whenever the module runs, so it is the only party that
    // can bound the set without a daemon. Pruning decides by filename, never
    // by stat, and a lost race with another writer's unlink is ENOENT, which
    // is the correct outcome.
    fn roll_to(&mut self, day: NaiveDate) -> io::Result<Option<SegmentNotice>> {
        let (removed, kept) = self.prune(day)?;
        let path = self.dir.join(format!("{}.{day}.log", self.module_id));
        let file = open_append(&path)?;
        self.open = Some((day, file));
        self.alarmed = false;
        Ok((removed > 0).then_some(SegmentNotice::Pruned { removed, kept }))
    }

    fn prune(&self, today: NaiveDate) -> io::Result<(usize, usize)> {
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            if let Some(name) = entry?.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        let owned: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|name| segment_day(&self.module_id, name).is_some())
            .collect();
        let doomed = prune_candidates(
            &self.module_id,
            today,
            self.retention,
            owned.iter().copied(),
        );
        for name in &doomed {
            match fs::remove_file(self.dir.join(name)) {
                Ok(()) => {}
                // Another writer for this module pruned it first. That is the
                // design working, not a failure.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok((doomed.len(), owned.len() - doomed.len()))
    }
}

fn prepare_dir(dir: &Path, enforce_directory_mode: bool) -> io::Result<()> {
    let existed = dir.exists();
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    if enforce_directory_mode || !existed {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = (enforce_directory_mode, existed);
    Ok(())
}

fn open_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    if file.metadata()?.file_type().is_file() {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}
