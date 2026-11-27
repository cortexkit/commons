use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{EvidenceValue, Fix, FixResult, Severity};

use super::*;

const MODULE: &str = "fixture-module";
const ID: &str = "history.backlog";

fn finding(summary: &str, steps: &[&str], count: u64) -> Finding {
    let id = FindingId::new(ID).unwrap();
    let fix = Fix::safe(
        &id,
        true,
        steps.iter().map(|step| (*step).into()).collect(),
        1_000,
    )
    .unwrap();
    let evidence = Evidence::new("count", EvidenceValue::Count(count)).unwrap();
    Finding::new(id, Severity::Warning, summary, vec![evidence], fix).unwrap()
}

enum Reply<T> {
    Return(T),
    Panic,
}

impl<T> Reply<T> {
    fn call(self) -> T {
        match self {
            Self::Return(value) => value,
            Self::Panic => panic!("fixture callback panic"),
        }
    }
}

type Trace = Arc<Mutex<Vec<&'static str>>>;

fn record(trace: &Trace, event: &'static str) {
    trace.lock().unwrap().push(event);
}

struct ScriptedTerminal {
    read: Option<TerminalRead>,
    trace: Trace,
    elapsed: Arc<AtomicU64>,
    after_read: Option<u64>,
}

impl Terminal for ScriptedTerminal {
    fn write_prompt(&mut self, text: &str) -> io::Result<()> {
        record(&self.trace, "write");
        assert!(text.starts_with("CortexKit fixture-module: "));
        assert!(text.ends_with("\nType fixture-module to confirm: "));
        Ok(())
    }

    fn read_line(&mut self, deadline: Instant) -> io::Result<TerminalRead> {
        record(&self.trace, "read");
        assert!(deadline > Instant::now());
        if let Some(secs) = self.after_read {
            self.elapsed.store(secs, Ordering::Relaxed);
        }
        Ok(self.read.take().unwrap())
    }
}

type CheckReply = Reply<Result<Vec<Finding>, CheckError>>;
type PreflightReply = Reply<Result<(), PreflightError>>;

struct Scenario {
    first: Finding,
    id: FindingId,
    digest: String,
    checks: VecDeque<CheckReply>,
    preflights: VecDeque<PreflightReply>,
    fix: Reply<FixOutcome>,
    presence: Arc<Presence>,
    terminal: Option<TerminalRead>,
    timing: Timing,
    mode: Mode,
    after_preflight: [Option<u64>; 2],
    after_presence: Option<u64>,
    after_read: Option<u64>,
}

impl Scenario {
    fn new() -> Self {
        Self::with_finding(finding(
            "History needs attention.",
            &["Prune history", "Rebuild index"],
            1,
        ))
    }

    fn with_finding(first: Finding) -> Self {
        Self {
            id: first.id().clone(),
            digest: first.fix().plan().unwrap().digest().into(),
            checks: [
                Reply::Return(Ok(vec![first.clone()])),
                Reply::Return(Ok(vec![first.clone()])),
            ]
            .into(),
            preflights: [Reply::Return(Ok(())), Reply::Return(Ok(()))].into(),
            fix: Reply::Return(FixOutcome::fixed(vec![]).unwrap()),
            presence: Arc::new(|_, _| Outcome::Approved),
            terminal: None,
            timing: Timing::default(),
            mode: Mode::Quick,
            after_preflight: [None; 2],
            after_presence: None,
            after_read: None,
            first,
        }
    }

    fn run(mut self) -> (FixOutput, Vec<&'static str>) {
        let entry = Instant::now();
        let elapsed = Arc::new(AtomicU64::new(0));
        let trace = Trace::default();
        let presence_trace = trace.clone();
        let presence_elapsed = elapsed.clone();
        let hooks = Hooks {
            now: &|| entry + Duration::from_secs(elapsed.load(Ordering::Relaxed)),
            presence: Arc::new(move |text, publish| {
                record(&presence_trace, "presence");
                if let Some(secs) = self.after_presence {
                    presence_elapsed.store(secs, Ordering::Relaxed);
                }
                (self.presence)(text, publish)
            }),
            open_terminal: &|| {
                record(&trace, "terminal");
                self.terminal.clone().map(|read| {
                    Box::new(ScriptedTerminal {
                        read: Some(read),
                        trace: trace.clone(),
                        elapsed: elapsed.clone(),
                        after_read: self.after_read,
                    }) as Box<dyn Terminal>
                })
            },
            timing: self.timing,
        };
        let request = FixRequest {
            id: &self.id,
            digest: &self.digest,
            module_id: &ModuleId::new(MODULE).unwrap(),
            entry,
            mode: self.mode,
        };
        let mut preflight_step = 0;
        let mut offered = Vec::new();
        let mut applied = None;
        let output = request
            .apply(
                &mut |mode| {
                    assert_eq!(mode, self.mode);
                    record(&trace, "check");
                    self.checks.pop_front().expect("unexpected check").call()
                },
                &mut |finding| {
                    record(&trace, "preflight");
                    offered.push(finding.clone());
                    if let Some(secs) = self.after_preflight[preflight_step] {
                        elapsed.store(secs, Ordering::Relaxed);
                    }
                    preflight_step += 1;
                    self.preflights
                        .pop_front()
                        .expect("unexpected preflight")
                        .call()
                },
                |finding| {
                    record(&trace, "fix");
                    applied = Some(finding);
                    self.fix.call()
                },
                &hooks,
            )
            .unwrap_or_else(Stop::output);
        assert!(offered.iter().all(|finding| finding == &self.first));
        if let Some(applied) = applied {
            assert_eq!(applied, self.first);
        }
        let trace = trace.lock().unwrap().clone();
        (output, trace)
    }

    fn expect(self, result: FixResult, code: Option<&str>, events: &[&str]) {
        let (output, trace) = self.run();
        assert_eq!((output.result(), output.code()), (result, code));
        assert_eq!(trace, events);
    }

    fn refuses(self, code: &str, events: &[&str]) {
        self.expect(FixResult::Refused, Some(code), events);
    }

    fn fails(self, code: &str, events: &[&str]) {
        self.expect(FixResult::Failed, Some(code), events);
    }
}

const PREFLIGHT: &[&str] = &["check", "preflight"];
const PROMPT: &[&str] = &["check", "preflight", "presence"];
const RECHECK: &[&str] = &["check", "preflight", "presence", "check"];
const RECHECK_PREFLIGHT: &[&str] = &["check", "preflight", "presence", "check", "preflight"];
const APPLY: &[&str] = &[
    "check",
    "preflight",
    "presence",
    "check",
    "preflight",
    "fix",
];
const OPEN_TERMINAL: &[&str] = &["check", "preflight", "presence", "terminal"];
const READ_TERMINAL: &[&str] = &[
    "check",
    "preflight",
    "presence",
    "terminal",
    "write",
    "read",
];
const APPLY_TERMINAL: &[&str] = &[
    "check",
    "preflight",
    "presence",
    "terminal",
    "write",
    "read",
    "check",
    "preflight",
    "fix",
];

#[test]
fn module_helper_timing_defaults() {
    assert_eq!(
        Timing::default(),
        Timing {
            prompt_limit: Duration::from_secs(120),
            terminal_limit: Duration::from_secs(120),
            confirm_gate: Duration::from_secs(10),
            fix_gate: Duration::from_secs(140),
            full_confirm_gate: Duration::from_secs(1800),
            full_fix_gate: Duration::from_secs(3720),
        }
    );
}

#[test]
fn module_helper_refuses_before_confirming_or_applying() {
    let mut absent = Scenario::new();
    absent.id = FindingId::new("unknown").unwrap();
    absent.refuses("unknown_finding", &["check"]);

    let mut stale = Scenario::new();
    stale.digest = format!("sha256:{}", "0".repeat(64));
    stale.refuses("plan_changed", &["check"]);

    let mut none = Scenario::new();
    none.checks[0] = Reply::Return(Ok(vec![Finding::new(
        none.id.clone(),
        Severity::Info,
        "History is read only.",
        vec![],
        Fix::none(false),
    )
    .unwrap()]));
    none.refuses("plan_changed", &["check"]);

    for (err, code) in [
        (PreflightError::ModuleRunning, "module_running"),
        (PreflightError::FilesInUse, "files_in_use"),
    ] {
        let mut refusal = Scenario::new();
        refusal.preflights[0] = Reply::Return(Err(err));
        refusal.refuses(code, PREFLIGHT);
    }
}

#[test]
fn module_helper_callback_errors_and_panics_stop_the_flow() {
    for (step, events) in [(0, &["check"][..]), (1, RECHECK)] {
        let mut check_error = Scenario::new();
        check_error.checks[step] = Reply::Return(Err(CheckError));
        check_error.fails("check_failed", events);

        let mut duplicates = Scenario::new();
        duplicates.checks[step] = Reply::Return(Ok(vec![duplicates.first.clone(); 2]));
        duplicates.fails("check_failed", events);

        let mut check_panic = Scenario::new();
        check_panic.checks[step] = Reply::Panic;
        check_panic.fails("callback_panicked", events);
    }

    for (step, events) in [(0, PREFLIGHT), (1, RECHECK_PREFLIGHT)] {
        let mut preflight_error = Scenario::new();
        preflight_error.preflights[step] = Reply::Return(Err(PreflightError::Failed));
        preflight_error.fails("preflight_failed", events);

        let mut preflight_panic = Scenario::new();
        preflight_panic.preflights[step] = Reply::Panic;
        preflight_panic.fails("callback_panicked", events);
    }

    let mut fix_panic = Scenario::new();
    fix_panic.fix = Reply::Panic;
    fix_panic.fails("callback_panicked", APPLY);
}

#[test]
fn module_helper_presence_uses_exact_text_and_unicode_limit() {
    let mut plain = Scenario::new();
    plain.presence = Arc::new(|text, _| {
        assert_eq!(
            text,
            "CortexKit fixture-module: History needs attention. Fix: Prune history; Rebuild index"
        );
        Outcome::Approved
    });
    plain.expect(FixResult::Fixed, None, APPLY);

    let summary = "é".repeat(200);
    let step = "界".repeat(200);
    let mut long = Scenario::with_finding(finding(&summary, &[&step], 1));
    let prefix = format!("CortexKit fixture-module: {summary} Fix: ");
    let expected = format!("{prefix}{}", "界".repeat(400 - prefix.chars().count()));
    long.presence = Arc::new(move |text, _| {
        assert_eq!(text, expected);
        Outcome::Approved
    });
    long.expect(FixResult::Fixed, None, APPLY);
}

fn authentication_error(failure: LocalAuthenticationFailure) -> Outcome {
    Outcome::LocalAuthenticationError {
        failure,
        domain: "fixture".into(),
        code: -1,
    }
}

#[test]
fn module_helper_maps_presence_outcomes_without_implicit_approval() {
    for outcome in [
        Outcome::Declined,
        Outcome::AuthorizedWithoutPrompt,
        authentication_error(LocalAuthenticationFailure::Declined),
    ] {
        let mut scenario = Scenario::new();
        scenario.presence = Arc::new(move |_, _| outcome.clone());
        scenario.refuses("not_confirmed", PROMPT);
    }

    for outcome in [
        Outcome::NoPresence,
        Outcome::UnsupportedPlatform,
        Outcome::SetupRequired,
        Outcome::ProviderError,
        Outcome::ProviderErrorWithReason { reason: "fixture" },
        authentication_error(LocalAuthenticationFailure::NoPresence),
        authentication_error(LocalAuthenticationFailure::ProviderError),
    ] {
        let mut scenario = Scenario::new();
        scenario.presence = Arc::new(move |_, _| outcome.clone());
        scenario.refuses("no_terminal", OPEN_TERMINAL);
    }
}

#[test]
fn module_helper_terminal_requires_the_complete_module_id_line() {
    for ending in ["\n", "\r\n"] {
        let mut scenario = Scenario::new();
        scenario.presence = Arc::new(|_, _| Outcome::NoPresence);
        scenario.terminal = Some(TerminalRead::Line(format!("{MODULE}{ending}").into_bytes()));
        scenario.expect(FixResult::Fixed, None, APPLY_TERMINAL);
    }
    for reply in [
        TerminalRead::Line(b"another-module\n".to_vec()),
        TerminalRead::Line(format!("{MODULE}\r\r\n").into_bytes()),
        TerminalRead::Line(MODULE.as_bytes().to_vec()),
        TerminalRead::Eof,
        TerminalRead::TimedOut,
    ] {
        let mut scenario = Scenario::new();
        scenario.presence = Arc::new(|_, _| Outcome::NoPresence);
        scenario.terminal = Some(reply);
        scenario.refuses("not_confirmed", READ_TERMINAL);
    }
}

#[test]
fn module_helper_budget_gates_are_measured_from_entry() {
    let mut before_prompt = Scenario::new();
    before_prompt.after_preflight[0] = Some(11);
    before_prompt.refuses("out_of_time", PREFLIGHT);

    let mut before_terminal = Scenario::new();
    before_terminal.presence = Arc::new(|_, _| Outcome::NoPresence);
    before_terminal.after_presence = Some(11);
    before_terminal.refuses("out_of_time", PROMPT);

    let mut within_budget = Scenario::new();
    within_budget.presence = Arc::new(|_, _| Outcome::NoPresence);
    within_budget.after_presence = Some(9);
    within_budget.terminal = Some(TerminalRead::Line(b"fixture-module\n".to_vec()));
    within_budget.after_read = Some(128);
    within_budget.expect(FixResult::Fixed, None, APPLY_TERMINAL);

    let mut on_boundaries = Scenario::new();
    on_boundaries.after_preflight = [Some(10), Some(140)];
    on_boundaries.expect(FixResult::Fixed, None, APPLY);

    let mut before_fix = Scenario::new();
    before_fix.after_preflight[1] = Some(141);
    before_fix.refuses("out_of_time", RECHECK_PREFLIGHT);
}

#[test]
fn module_helper_full_fix_budget_gates_are_measured_from_entry() {
    let full = || Scenario {
        mode: Mode::Full,
        ..Scenario::new()
    };

    let mut before_prompt = full();
    before_prompt.after_preflight[0] = Some(1801);
    before_prompt.refuses("out_of_time", PREFLIGHT);

    let mut before_terminal = full();
    before_terminal.after_preflight[0] = Some(1799);
    before_terminal.presence = Arc::new(|_, _| Outcome::NoPresence);
    before_terminal.after_presence = Some(1801);
    before_terminal.refuses("out_of_time", PROMPT);

    let mut on_boundaries = full();
    on_boundaries.presence = Arc::new(|_, _| Outcome::NoPresence);
    on_boundaries.terminal = Some(TerminalRead::Line(b"fixture-module\n".to_vec()));
    on_boundaries.after_preflight = [Some(1800), Some(3720)];
    on_boundaries.expect(FixResult::Fixed, None, APPLY_TERMINAL);

    let mut before_fix = full();
    before_fix.after_preflight = [Some(1800), Some(3721)];
    before_fix.refuses("out_of_time", RECHECK_PREFLIGHT);
}

#[test]
fn module_helper_rechecks_identity_plan_summary_and_liveness_after_confirmation() {
    let mut gone = Scenario::new();
    gone.checks[1] = Reply::Return(Ok(vec![]));
    gone.refuses("unknown_finding", RECHECK);

    let mut changed = Scenario::new();
    changed.checks[1] = Reply::Return(Ok(vec![finding(changed.first.summary(), &["New plan"], 1)]));
    changed.refuses("plan_changed", RECHECK);

    let mut new_summary = Scenario::new();
    new_summary.checks[1] = Reply::Return(Ok(vec![finding(
        "A different problem.",
        &["Prune history", "Rebuild index"],
        1,
    )]));
    new_summary.refuses("plan_changed", RECHECK);

    let mut now_running = Scenario::new();
    now_running.preflights[1] = Reply::Return(Err(PreflightError::ModuleRunning));
    now_running.refuses("module_running", RECHECK_PREFLIGHT);

    let mut new_evidence = Scenario::new();
    new_evidence.checks[1] = Reply::Return(Ok(vec![finding(
        new_evidence.first.summary(),
        &["Prune history", "Rebuild index"],
        2,
    )]));
    new_evidence.fix = Reply::Return(FixOutcome::refused(FixRefusal::PlanChanged));
    // The fixture verifies both preflights and the fix received the original Finding.
    new_evidence.refuses("plan_changed", APPLY);
}

struct WithdrawSpy(mpsc::Sender<()>);

impl Withdraw for WithdrawSpy {
    fn withdraw(&self) {
        self.0.send(()).unwrap();
    }
}

#[test]
fn module_helper_timeout_withdraws_even_if_provider_publishes_late() {
    for publish_before_timeout in [true, false] {
        let (unblock, blocked) = mpsc::channel();
        let blocked = Mutex::new(blocked);
        let (withdrawn, observed_withdrawal) = mpsc::channel();
        let (output, observed_output) = mpsc::channel();
        let (exited, provider_exited) = mpsc::channel();

        let mut scenario = Scenario::new();
        scenario.timing.prompt_limit = Duration::from_millis(200);
        scenario.presence = Arc::new(move |_, publish| {
            if publish_before_timeout {
                publish(Arc::new(WithdrawSpy(withdrawn.clone())));
                blocked.lock().unwrap().recv().unwrap();
            } else {
                blocked.lock().unwrap().recv().unwrap();
                publish(Arc::new(WithdrawSpy(withdrawn.clone())));
            }
            exited.send(()).unwrap();
            Outcome::Approved
        });
        let execution = std::thread::spawn(move || output.send(scenario.run()).unwrap());

        let result = observed_output.recv_timeout(Duration::from_secs(5));
        let withdrew_before_return = observed_withdrawal.try_recv().is_ok();
        unblock.send(()).unwrap();
        provider_exited
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        execution.join().unwrap();

        let (output, trace) = result.expect("helper must not join the presence thread");
        assert_eq!(
            (output.result(), output.code()),
            (FixResult::Refused, Some("not_confirmed"))
        );
        assert_eq!(trace, PROMPT);
        assert_eq!(withdrew_before_return, publish_before_timeout);
        if !publish_before_timeout {
            observed_withdrawal
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
        }
    }
}
