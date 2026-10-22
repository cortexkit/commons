//! Per-case progress while a run is in flight.
//!
//! A full run takes many minutes, and a CI job can be cut off before
//! [`run_suite`](crate::run_suite) returns its report. A progress callback
//! passed through [`SuiteOptions`] hears about every case as it starts and as
//! it ends, so a cut-off log still names the case that was running.

use std::io::Write;

use crate::{
    report::{CaseOutcome, CASES},
    subject::Capability,
};

/// One step of a run, passed to the progress callback.
///
/// Every case produces a [`CaseStarted`](Self::CaseStarted) and then a
/// [`CaseFinished`](Self::CaseFinished), in [`CASES`] order. That includes a
/// case skipped for a missing capability or reported inapplicable before it
/// runs: it still starts and finishes. A capability skip is decided without
/// touching the runner, so its elapsed time is near zero; an inapplicability
/// check may ask the runner first and take a little longer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SuiteProgress {
    /// The suite has reached the case and is about to check whether it can
    /// run, then run it. `index` is its zero-based position in
    /// [`CASES`]; `total` is the number of cases in the run.
    CaseStarted {
        case: &'static str,
        index: usize,
        total: usize,
    },
    /// The case's outcome is known. `outcome` is the same value the final
    /// report carries for the case; `elapsed_ms` is the wall-clock time spent
    /// on the case, measured from just after the
    /// [`CaseStarted`](Self::CaseStarted) callback returned.
    CaseFinished {
        case: &'static str,
        outcome: CaseOutcome,
        elapsed_ms: u64,
    },
}

/// The callback type [`SuiteOptions::with_progress`] stores.
type ProgressCallback = Box<dyn FnMut(&SuiteProgress) + Send>;

/// Options for [`run_suite_with`](crate::run_suite_with). Build with
/// [`SuiteOptions::new`] and the `with_*` methods; new options may be added
/// in later releases without breaking callers.
#[derive(Default)]
#[non_exhaustive]
pub struct SuiteOptions {
    progress: Option<ProgressCallback>,
}

impl SuiteOptions {
    /// Options that change nothing: a run with them behaves exactly like
    /// [`run_suite`](crate::run_suite).
    pub fn new() -> Self {
        Self::default()
    }

    /// Call `callback` synchronously before each case starts and right after
    /// its outcome is known. The run waits for the callback, so it should be
    /// quick.
    pub fn with_progress(mut self, callback: impl FnMut(&SuiteProgress) + Send + 'static) -> Self {
        self.progress = Some(Box::new(callback));
        self
    }

    /// Pass `event` to the progress callback, if there is one. `event` is
    /// built only when a callback is set, so a run without one does no extra
    /// work.
    pub(crate) fn report(&mut self, event: impl FnOnce() -> SuiteProgress) {
        if let Some(callback) = self.progress.as_mut() {
            callback(&event());
        }
    }
}

impl std::fmt::Debug for SuiteOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SuiteOptions")
            .field("progress", &self.progress.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

/// A progress callback that writes one line per event to stderr and flushes
/// after each, so every line is already out of the process if the job is
/// killed:
///
/// ```text
/// START <case>
/// END <case> <PASS|FAIL|SKIP|N/A> <elapsed ms>
/// ```
///
/// The outcome word is the one [`SuiteReport::render`](crate::SuiteReport::render)
/// starts the case's line with.
pub fn stderr_lines() -> impl FnMut(&SuiteProgress) + Send + 'static {
    lines_to(std::io::stderr())
}

/// [`stderr_lines`] for any writer. Each line goes out in one `write_all`
/// and is flushed at once. A write error is ignored: progress output must
/// never fail the run.
pub(crate) fn lines_to<W: Write + Send + 'static>(
    mut out: W,
) -> impl FnMut(&SuiteProgress) + Send + 'static {
    move |event| {
        let _ = out.write_all(line(event).as_bytes());
        let _ = out.flush();
    }
}

/// The line, newline included, [`stderr_lines`] writes for `event`.
pub(crate) fn line(event: &SuiteProgress) -> String {
    match event {
        SuiteProgress::CaseStarted { case, .. } => format!("START {case}\n"),
        SuiteProgress::CaseFinished {
            case,
            outcome,
            elapsed_ms,
        } => format!("END {case} {} {elapsed_ms}\n", word(case, outcome)),
    }
}

/// The outcome word [`SuiteReport::render`](crate::SuiteReport::render) uses
/// for `case`. A capability skip of a case requiring `compaction` or
/// `model_view` is reported there as N/A, not SKIP, so the case's
/// requirements are looked up in [`CASES`] the same way the report records
/// them.
fn word(case: &str, outcome: &CaseOutcome) -> &'static str {
    match outcome {
        CaseOutcome::Passed | CaseOutcome::PassedWithInapplicableRule { .. } => "PASS",
        CaseOutcome::Failed { .. } => "FAIL",
        CaseOutcome::Skipped { .. } => {
            let not_applicable = CASES.iter().any(|spec| {
                spec.name == case
                    && spec
                        .requires
                        .iter()
                        .chain(spec.requires_any)
                        .any(|c| matches!(c, Capability::Compaction | Capability::ModelView))
            });
            if not_applicable {
                "N/A"
            } else {
                "SKIP"
            }
        }
        CaseOutcome::SkippedWithReason { .. } => "SKIP",
        CaseOutcome::Inapplicable { .. } => "N/A",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::report::{CaseReport, SuiteReport, SuiteVerdict};

    fn finished(case: &'static str, outcome: CaseOutcome, elapsed_ms: u64) -> SuiteProgress {
        SuiteProgress::CaseFinished {
            case,
            outcome,
            elapsed_ms,
        }
    }

    #[test]
    fn the_line_format_is_pinned() {
        let reason = || "why".to_owned();
        let cases = [
            (
                SuiteProgress::CaseStarted {
                    case: "tail_read_is_newest_page",
                    index: 3,
                    total: 9,
                },
                "START tail_read_is_newest_page\n",
            ),
            (
                finished("tail_read_is_newest_page", CaseOutcome::Passed, 1234),
                "END tail_read_is_newest_page PASS 1234\n",
            ),
            (
                finished(
                    "tail_read_is_newest_page",
                    CaseOutcome::PassedWithInapplicableRule { reason: reason() },
                    5,
                ),
                "END tail_read_is_newest_page PASS 5\n",
            ),
            (
                finished(
                    "tail_read_is_newest_page",
                    CaseOutcome::Failed { reason: reason() },
                    17,
                ),
                "END tail_read_is_newest_page FAIL 17\n",
            ),
            (
                finished(
                    "tail_read_is_newest_page",
                    CaseOutcome::Skipped {
                        missing: vec![Capability::Queue],
                    },
                    0,
                ),
                "END tail_read_is_newest_page SKIP 0\n",
            ),
            (
                finished(
                    "cut_starts_a_new_lineage",
                    CaseOutcome::Skipped {
                        missing: vec![Capability::Compaction],
                    },
                    0,
                ),
                "END cut_starts_a_new_lineage N/A 0\n",
            ),
            (
                finished(
                    "tail_read_is_newest_page",
                    CaseOutcome::SkippedWithReason {
                        missing: vec![Capability::RetentionClock],
                        reason: reason(),
                    },
                    2,
                ),
                "END tail_read_is_newest_page SKIP 2\n",
            ),
            (
                finished(
                    "tail_read_is_newest_page",
                    CaseOutcome::Inapplicable { reason: reason() },
                    0,
                ),
                "END tail_read_is_newest_page N/A 0\n",
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(line(&event), expected, "{event:?}");
        }
    }

    /// The END word must be the word the final report starts the case's
    /// line with, for every case and every kind of outcome, so a progress
    /// log and the report never disagree about a case.
    #[test]
    fn the_outcome_word_matches_the_report_for_every_case() {
        let outcomes = |missing: Vec<Capability>| {
            vec![
                CaseOutcome::Passed,
                CaseOutcome::PassedWithInapplicableRule { reason: "r".into() },
                CaseOutcome::Failed { reason: "r".into() },
                CaseOutcome::Skipped {
                    missing: missing.clone(),
                },
                CaseOutcome::SkippedWithReason {
                    missing,
                    reason: "r".into(),
                },
                CaseOutcome::Inapplicable { reason: "r".into() },
            ]
        };
        for spec in CASES {
            let requires: Vec<Capability> = spec
                .requires
                .iter()
                .chain(spec.requires_any)
                .copied()
                .collect();
            for outcome in outcomes(requires.clone()) {
                let report = SuiteReport {
                    cases: vec![CaseReport {
                        case: spec.name,
                        requires: requires.clone(),
                        outcome: outcome.clone(),
                    }],
                    kills: Vec::new(),
                    verdict: SuiteVerdict::Passed,
                };
                let rendered = report.render();
                let report_word = rendered.split_whitespace().next().unwrap();
                assert_eq!(
                    word(spec.name, &outcome),
                    report_word,
                    "{}: {outcome:?}",
                    spec.name
                );
            }
        }
    }

    /// A writer that keeps what is written in `pending` and makes it
    /// visible in `flushed` only on `flush`, as a buffered pipe would.
    struct HoldsUntilFlush {
        pending: Vec<u8>,
        flushed: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for HoldsUntilFlush {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.pending.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushed.lock().unwrap().extend(self.pending.drain(..));
            Ok(())
        }
    }

    #[test]
    fn each_line_is_flushed_as_soon_as_it_is_written() {
        let flushed = Arc::new(Mutex::new(Vec::new()));
        let mut callback = lines_to(HoldsUntilFlush {
            pending: Vec::new(),
            flushed: Arc::clone(&flushed),
        });
        let visible = || String::from_utf8(flushed.lock().unwrap().clone()).unwrap();

        callback(&SuiteProgress::CaseStarted {
            case: "tail_read_is_newest_page",
            index: 0,
            total: 1,
        });
        assert_eq!(visible(), "START tail_read_is_newest_page\n");
        callback(&finished(
            "tail_read_is_newest_page",
            CaseOutcome::Passed,
            7,
        ));
        assert_eq!(
            visible(),
            "START tail_read_is_newest_page\nEND tail_read_is_newest_page PASS 7\n"
        );
    }
}
