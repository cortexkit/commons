//! The crash-cut harness a role implementation supplies to its conformance
//! suite.
//!
//! A role's conformance suite has to prove what a module does across a crash:
//! that a call held at one durable point is never run after a restart, that
//! ids are never reused, and so on. To do that the suite needs three things
//! from each implementation, and nothing else:
//!
//! - [`Harness::spawn`] a fresh module on a state root it owns;
//! - [`Harness::kill_at`] a named point, leaving the root holding exactly the
//!   durable state up to and including that point;
//! - [`Harness::restart`] on the same root, so the new process sees only that
//!   state.
//!
//! plus [`Harness::route`] to reach the module the way a real caller does.
//!
//! # What `kill_at` means
//!
//! A kill is defined by what is left on disk, not by how it was done: after
//! `kill_at(point)`, the durable state on the root is exactly everything up to
//! and including `point` and nothing after it, and the next `restart` sees only
//! that. The implementation picks the mechanism (truncating a log, a fault hook
//! inside the module, or a real signal) and reports the one it used in the
//! [`KillReport`]. Point names are opaque strings; each role crate lists its
//! own vocabulary.
//!
//! # Why a run needs a real kill
//!
//! A kill that only truncates a log, or only discards state inside a live
//! process, cannot catch a module that keeps something important in memory
//! and would lose it in a real crash. So [`KillLedger::verdict`] reports
//! whether any kill in a run was a real process kill, and a suite fails its own
//! run when none was. A store where truncation is not meaningful (a SQLite
//! database, say) may reach its points only with a fault hook followed by a
//! real kill, and declares so in [`Harness::declared_points`].
//!
//! # One kill per point, one root per kill
//!
//! A suite calls `kill_at` once per point in a run, each time on a module it
//! spawned on a fresh state root, and never kills one trigger at several
//! points. That keeps each kill's durable state attributable to exactly one
//! point: a root killed twice would mix the leftovers of two cuts, and
//! nothing could say which one a restarted module was recovering from.
//!
//! Suites drive a harness through [`CrashDriver`], which enforces all of this,
//! checks every kill against the implementation's declaration and records it
//! in the ledger.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeSet,
    fmt,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
};

use async_trait::async_trait;

/// The name of a durable point a module can be killed at.
///
/// Opaque to this crate: each role crate lists the names its suite uses, and
/// an implementation declares which of them it can reach.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KillPoint(String);

impl KillPoint {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KillPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How one kill was carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KillMechanism {
    /// The module ran on, and its durable log was cut back to the point
    /// afterwards. Nothing was killed, so in-memory state never had to survive.
    LogTruncation,
    /// A hook inside the module stopped it right after the point became
    /// durable, and its in-memory state was discarded without ending a process
    /// (an in-process module, for instance).
    FaultHook,
    /// A hook inside the module stopped it right after the point became
    /// durable, and then the module's process was killed.
    FaultHookThenProcessKill,
    /// The module's process was killed by a signal once the point was durable,
    /// with no cooperation from the module.
    ProcessSignal,
}

impl KillMechanism {
    /// Whether this kill ended a real process, so anything the module kept
    /// only in memory was really lost.
    pub fn is_real_process_kill(self) -> bool {
        matches!(self, Self::FaultHookThenProcessKill | Self::ProcessSignal)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::LogTruncation => "log_truncation",
            Self::FaultHook => "fault_hook",
            Self::FaultHookThenProcessKill => "fault_hook_then_process_kill",
            Self::ProcessSignal => "process_signal",
        }
    }
}

impl fmt::Display for KillMechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One point an implementation can be killed at, and the mechanisms it may
/// use there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointDeclaration {
    pub point: KillPoint,
    /// Every mechanism a kill at this point may report. A kill that reports
    /// any other mechanism is refused by [`CrashDriver::kill_at`].
    pub mechanisms: Vec<KillMechanism>,
    /// Why these mechanisms, for a reader of the suite's report; for example
    /// "approval state lives in SQLite, so truncation is not meaningful".
    pub note: String,
}

/// What an implementation reports after a kill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KillReport {
    /// The point the durable state now ends at. Must equal the point asked for.
    pub point: KillPoint,
    pub mechanism: KillMechanism,
}

/// The scope a route is opened under, as the daemon stamps it:
/// `(owner, ref, scope_epoch)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopeStamp {
    /// The principal that owns the scope, for example `reserved:prefrontal-core`.
    pub owner: String,
    pub scope_ref: String,
    pub scope_epoch: u64,
}

/// The identity a route is opened with: the principal the daemon stamps on it
/// and, when the route is opened in a session's name, that session's scope.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RouteStamp {
    /// A principal string such as `reserved:broca`.
    pub principal: String,
    pub scope: Option<ScopeStamp>,
}

/// A harness operation that could not be carried out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessError {
    pub message: String,
}

impl HarnessError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HarnessError {}

/// The suite's action that drives the module towards a kill point, for
/// example sending a call that the module will hold.
pub type Trigger<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// What a role implementation supplies so a conformance suite can spawn it,
/// reach it, kill it at a named point and restart it.
///
/// Dropping a [`Harness::Handle`] must stop the module it names, so a suite
/// that finishes with a handle never leaves a module running.
#[async_trait]
pub trait Harness: Send + Sync {
    /// A live module instance.
    type Handle: Send + Sync;
    /// The implementation's way to reach the module's route: whatever the
    /// role's suite needs to send requests over a real route and observe the
    /// frames that come back.
    type Route: Send + Sync;

    /// Every point this implementation can be killed at, with the mechanisms
    /// it uses there. A point missing here cannot be asked for.
    fn declared_points(&self) -> Vec<PointDeclaration>;

    /// Start a module on `state_root`, which the suite created empty and which
    /// no other module uses.
    async fn spawn(&self, state_root: &Path) -> Result<Self::Handle, HarnessError>;

    /// Open a route to the module under `stamp`, as a real caller with that
    /// identity would.
    async fn route(
        &self,
        handle: &Self::Handle,
        stamp: &RouteStamp,
    ) -> Result<Self::Route, HarnessError>;

    /// Run `trigger` and kill the module once `point` is durable, so that the
    /// state root holds exactly everything up to and including `point` and
    /// nothing after it.
    ///
    /// The trigger is the suite's action; the module reaches `point` while it
    /// runs. The implementation arms whatever it needs before polling the
    /// trigger, may drop the trigger unfinished once the kill is done, and
    /// returns an error when the trigger finishes without the module reaching
    /// `point`.
    async fn kill_at(
        &self,
        handle: Self::Handle,
        point: &KillPoint,
        trigger: Trigger<'_>,
    ) -> Result<KillReport, HarnessError>;

    /// Start a module again on a root a previous `kill_at` left behind.
    async fn restart(&self, state_root: &Path) -> Result<Self::Handle, HarnessError>;
}

/// Whether any kill in a run ended a real process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealKillVerdict {
    /// The run killed nothing.
    NoKills,
    /// Every kill was simulated (truncation or an in-process hook), so the run
    /// cannot tell whether the module keeps something only in memory.
    OnlySimulated,
    /// At least one kill ended a real process.
    RealKillPresent,
}

/// Every kill a run made, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KillLedger {
    kills: Vec<KillReport>,
}

impl KillLedger {
    pub fn record(&mut self, report: KillReport) {
        self.kills.push(report);
    }

    pub fn kills(&self) -> &[KillReport] {
        &self.kills
    }

    pub fn verdict(&self) -> RealKillVerdict {
        if self.kills.is_empty() {
            RealKillVerdict::NoKills
        } else if self
            .kills
            .iter()
            .any(|kill| kill.mechanism.is_real_process_kill())
        {
            RealKillVerdict::RealKillPresent
        } else {
            RealKillVerdict::OnlySimulated
        }
    }
}

/// Why the driver refused or failed a harness operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverError {
    /// The implementation itself failed.
    Harness(HarnessError),
    /// The suite asked for a point the implementation never declared.
    UndeclaredPoint(KillPoint),
    /// The kill used a mechanism the implementation did not declare for that
    /// point. The kill still happened and is in the ledger.
    MechanismNotDeclared {
        point: KillPoint,
        used: KillMechanism,
    },
    /// The implementation reports a different point than the one asked for.
    /// The kill still happened and is in the ledger.
    ReportedWrongPoint {
        asked: KillPoint,
        reported: KillPoint,
    },
    /// The suite spawned on a root this run already used. Every spawn needs a
    /// fresh root.
    RootAlreadyUsed(PathBuf),
    /// The handle's state root was already killed once in this run (the
    /// handle came from a restart). Each root is killed at most once.
    RootAlreadyKilled(PathBuf),
    /// This run already killed at this point. Each point is cut once.
    PointAlreadyKilled(KillPoint),
    /// The suite restarted on a root this run never killed.
    RootNotKilled(PathBuf),
}

impl From<HarnessError> for DriverError {
    fn from(error: HarnessError) -> Self {
        Self::Harness(error)
    }
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Harness(error) => write!(f, "harness failed: {error}"),
            Self::UndeclaredPoint(point) => {
                write!(
                    f,
                    "kill point {point} is not declared by the implementation"
                )
            }
            Self::MechanismNotDeclared { point, used } => write!(
                f,
                "kill at {point} used {used}, which the implementation did not declare for it"
            ),
            Self::ReportedWrongPoint { asked, reported } => {
                write!(
                    f,
                    "asked to kill at {asked}, the implementation reports {reported}"
                )
            }
            Self::RootAlreadyUsed(root) => {
                write!(
                    f,
                    "state root {} was already used in this run",
                    root.display()
                )
            }
            Self::RootAlreadyKilled(root) => {
                write!(
                    f,
                    "state root {} was already killed in this run",
                    root.display()
                )
            }
            Self::PointAlreadyKilled(point) => write!(f, "this run already killed at {point}"),
            Self::RootNotKilled(root) => write!(
                f,
                "state root {} was never killed, so there is nothing to restart",
                root.display()
            ),
        }
    }
}

impl std::error::Error for DriverError {}

/// A module the driver started, with the state root it runs on.
pub struct DriverHandle<H: Harness> {
    inner: H::Handle,
    root: PathBuf,
}

impl<H: Harness> DriverHandle<H> {
    pub fn inner(&self) -> &H::Handle {
        &self.inner
    }

    pub fn state_root(&self) -> &Path {
        &self.root
    }
}

/// The only way a suite kills a module: every kill is checked against the
/// implementation's declaration and the one-kill rules, and recorded in a
/// [`KillLedger`].
pub struct CrashDriver<'h, H: Harness> {
    harness: &'h H,
    declared: Vec<PointDeclaration>,
    ledger: KillLedger,
    used_roots: BTreeSet<PathBuf>,
    killed_roots: BTreeSet<PathBuf>,
    killed_points: BTreeSet<KillPoint>,
}

impl<'h, H: Harness> CrashDriver<'h, H> {
    pub fn new(harness: &'h H) -> Self {
        Self {
            declared: harness.declared_points(),
            harness,
            ledger: KillLedger::default(),
            used_roots: BTreeSet::new(),
            killed_roots: BTreeSet::new(),
            killed_points: BTreeSet::new(),
        }
    }

    pub fn harness(&self) -> &'h H {
        self.harness
    }

    pub fn declared_points(&self) -> &[PointDeclaration] {
        &self.declared
    }

    pub fn ledger(&self) -> &KillLedger {
        &self.ledger
    }

    /// Start a module on a root this run has not used before.
    pub async fn spawn(&mut self, state_root: &Path) -> Result<DriverHandle<H>, DriverError> {
        if !self.used_roots.insert(state_root.to_owned()) {
            return Err(DriverError::RootAlreadyUsed(state_root.to_owned()));
        }
        Ok(DriverHandle {
            inner: self.harness.spawn(state_root).await?,
            root: state_root.to_owned(),
        })
    }

    pub async fn route(
        &self,
        handle: &DriverHandle<H>,
        stamp: &RouteStamp,
    ) -> Result<H::Route, DriverError> {
        Ok(self.harness.route(&handle.inner, stamp).await?)
    }

    /// Kill at `point` through the implementation, after checking the point is
    /// declared, that this run has not cut it before, and that the handle's
    /// root has not been killed before. The kill is recorded even when its
    /// report breaks the declaration, because it happened either way.
    pub async fn kill_at(
        &mut self,
        handle: DriverHandle<H>,
        point: &KillPoint,
        trigger: Trigger<'_>,
    ) -> Result<KillReport, DriverError> {
        let Some(declaration) = self.declared.iter().find(|d| &d.point == point) else {
            return Err(DriverError::UndeclaredPoint(point.clone()));
        };
        if self.killed_roots.contains(&handle.root) {
            return Err(DriverError::RootAlreadyKilled(handle.root));
        }
        if self.killed_points.contains(point) {
            return Err(DriverError::PointAlreadyKilled(point.clone()));
        }
        let allowed = declaration.mechanisms.clone();
        self.killed_roots.insert(handle.root.clone());
        self.killed_points.insert(point.clone());
        let report = self.harness.kill_at(handle.inner, point, trigger).await?;
        self.ledger.record(report.clone());
        if &report.point != point {
            return Err(DriverError::ReportedWrongPoint {
                asked: point.clone(),
                reported: report.point,
            });
        }
        if !allowed.contains(&report.mechanism) {
            return Err(DriverError::MechanismNotDeclared {
                point: report.point,
                used: report.mechanism,
            });
        }
        Ok(report)
    }

    /// Start a module again on a root this run killed.
    pub async fn restart(&self, state_root: &Path) -> Result<DriverHandle<H>, DriverError> {
        if !self.killed_roots.contains(state_root) {
            return Err(DriverError::RootNotKilled(state_root.to_owned()));
        }
        Ok(DriverHandle {
            inner: self.harness.restart(state_root).await?,
            root: state_root.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// A harness with no module behind it: it reports whatever mechanism it
    /// was built with, so the driver's checks can be exercised directly.
    struct Scripted {
        reports: Mutex<Vec<KillReport>>,
        declared: Vec<PointDeclaration>,
    }

    impl Scripted {
        fn new(declared: &[(&str, &[KillMechanism])], reports: Vec<KillReport>) -> Self {
            Self {
                reports: Mutex::new(reports),
                declared: declared
                    .iter()
                    .map(|(point, mechanisms)| PointDeclaration {
                        point: KillPoint::new(*point),
                        mechanisms: mechanisms.to_vec(),
                        note: String::new(),
                    })
                    .collect(),
            }
        }
    }

    #[async_trait]
    impl Harness for Scripted {
        type Handle = ();
        type Route = ();

        fn declared_points(&self) -> Vec<PointDeclaration> {
            self.declared.clone()
        }

        async fn spawn(&self, _: &Path) -> Result<(), HarnessError> {
            Ok(())
        }

        async fn route(&self, _: &(), _: &RouteStamp) -> Result<(), HarnessError> {
            Ok(())
        }

        async fn kill_at(
            &self,
            _: (),
            _: &KillPoint,
            trigger: Trigger<'_>,
        ) -> Result<KillReport, HarnessError> {
            trigger.await;
            Ok(self.reports.lock().unwrap().remove(0))
        }

        async fn restart(&self, _: &Path) -> Result<(), HarnessError> {
            Ok(())
        }
    }

    fn report(point: &str, mechanism: KillMechanism) -> KillReport {
        KillReport {
            point: KillPoint::new(point),
            mechanism,
        }
    }

    fn noop() -> Trigger<'static> {
        Box::pin(async {})
    }

    #[test]
    fn only_signal_and_hook_then_kill_are_real_process_kills() {
        assert!(!KillMechanism::LogTruncation.is_real_process_kill());
        assert!(!KillMechanism::FaultHook.is_real_process_kill());
        assert!(KillMechanism::FaultHookThenProcessKill.is_real_process_kill());
        assert!(KillMechanism::ProcessSignal.is_real_process_kill());
    }

    #[test]
    fn a_ledger_of_simulated_kills_is_not_a_real_kill() {
        let mut ledger = KillLedger::default();
        assert_eq!(ledger.verdict(), RealKillVerdict::NoKills);
        ledger.record(report("A", KillMechanism::LogTruncation));
        ledger.record(report("B", KillMechanism::FaultHook));
        assert_eq!(ledger.verdict(), RealKillVerdict::OnlySimulated);
        ledger.record(report("C", KillMechanism::FaultHookThenProcessKill));
        assert_eq!(ledger.verdict(), RealKillVerdict::RealKillPresent);
    }

    #[tokio::test]
    async fn the_driver_refuses_an_undeclared_point_before_killing() {
        let harness = Scripted::new(&[("A", &[KillMechanism::ProcessSignal])], vec![]);
        let mut driver = CrashDriver::new(&harness);
        let handle = driver.spawn(Path::new("/r")).await.unwrap();
        let error = driver
            .kill_at(handle, &KillPoint::new("B"), noop())
            .await
            .unwrap_err();
        assert_eq!(error, DriverError::UndeclaredPoint(KillPoint::new("B")));
        assert!(driver.ledger().kills().is_empty());
    }

    #[tokio::test]
    async fn the_driver_records_and_refuses_an_undeclared_mechanism() {
        let harness = Scripted::new(
            &[("A", &[KillMechanism::FaultHookThenProcessKill])],
            vec![report("A", KillMechanism::LogTruncation)],
        );
        let mut driver = CrashDriver::new(&harness);
        let handle = driver.spawn(Path::new("/r")).await.unwrap();
        let error = driver
            .kill_at(handle, &KillPoint::new("A"), noop())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            DriverError::MechanismNotDeclared {
                point: KillPoint::new("A"),
                used: KillMechanism::LogTruncation,
            }
        );
        assert_eq!(driver.ledger().kills().len(), 1);
    }

    #[tokio::test]
    async fn the_driver_refuses_a_report_for_another_point() {
        let harness = Scripted::new(
            &[("A", &[KillMechanism::ProcessSignal])],
            vec![report("B", KillMechanism::ProcessSignal)],
        );
        let mut driver = CrashDriver::new(&harness);
        let handle = driver.spawn(Path::new("/r")).await.unwrap();
        let error = driver
            .kill_at(handle, &KillPoint::new("A"), noop())
            .await
            .unwrap_err();
        assert!(matches!(error, DriverError::ReportedWrongPoint { .. }));
    }

    #[tokio::test]
    async fn the_driver_runs_the_trigger_and_records_a_declared_kill() {
        let harness = Scripted::new(
            &[("A", &[KillMechanism::ProcessSignal])],
            vec![report("A", KillMechanism::ProcessSignal)],
        );
        let mut driver = CrashDriver::new(&harness);
        let ran = std::sync::atomic::AtomicBool::new(false);
        let handle = driver.spawn(Path::new("/r")).await.unwrap();
        let kill = driver
            .kill_at(
                handle,
                &KillPoint::new("A"),
                Box::pin(async { ran.store(true, std::sync::atomic::Ordering::SeqCst) }),
            )
            .await
            .unwrap();
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(kill, report("A", KillMechanism::ProcessSignal));
        assert_eq!(driver.ledger().verdict(), RealKillVerdict::RealKillPresent);
    }

    #[tokio::test]
    async fn the_driver_kills_each_root_and_each_point_once() {
        let harness = Scripted::new(
            &[
                ("A", &[KillMechanism::ProcessSignal]),
                ("B", &[KillMechanism::ProcessSignal]),
            ],
            vec![
                report("A", KillMechanism::ProcessSignal),
                report("B", KillMechanism::ProcessSignal),
            ],
        );
        let mut driver = CrashDriver::new(&harness);
        let root = Path::new("/r1");
        assert_eq!(
            driver.restart(root).await.err(),
            Some(DriverError::RootNotKilled(root.to_owned()))
        );
        let handle = driver.spawn(root).await.unwrap();
        assert_eq!(
            driver.spawn(root).await.err(),
            Some(DriverError::RootAlreadyUsed(root.to_owned()))
        );
        driver
            .kill_at(handle, &KillPoint::new("A"), noop())
            .await
            .unwrap();

        // A restarted handle's root was already killed: a second cut there
        // would mix the leftovers of two points.
        let restarted = driver.restart(root).await.unwrap();
        assert_eq!(
            driver
                .kill_at(restarted, &KillPoint::new("B"), noop())
                .await
                .err(),
            Some(DriverError::RootAlreadyKilled(root.to_owned()))
        );

        // A fresh root may not cut a point this run already cut.
        let fresh = driver.spawn(Path::new("/r2")).await.unwrap();
        assert_eq!(
            driver
                .kill_at(fresh, &KillPoint::new("A"), noop())
                .await
                .err(),
            Some(DriverError::PointAlreadyKilled(KillPoint::new("A")))
        );
        let fresh = driver.spawn(Path::new("/r3")).await.unwrap();
        driver
            .kill_at(fresh, &KillPoint::new("B"), noop())
            .await
            .unwrap();
        assert_eq!(driver.ledger().kills().len(), 2);
    }
}
