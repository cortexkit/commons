use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Finding, ModuleId};

use super::{
    callback, terminal, LocalAuthenticationFailure, Outcome, Stop, Terminal, TerminalRead, Withdraw,
};

/// Limits for the confirmation waits and the gates measured from helper entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub prompt_limit: Duration,
    pub terminal_limit: Duration,
    pub confirm_gate: Duration,
    pub fix_gate: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            prompt_limit: Duration::from_secs(120),
            terminal_limit: Duration::from_secs(120),
            confirm_gate: Duration::from_secs(10),
            fix_gate: Duration::from_secs(140),
        }
    }
}

/// Supplies the handle the helper calls to withdraw a timed-out prompt.
pub type PublishWithdraw = Box<dyn FnOnce(Arc<dyn Withdraw>) + Send>;

/// A blocking OS prompt, run on a separate thread.
pub type Presence = dyn Fn(&str, PublishWithdraw) -> Outcome + Send + Sync;

/// Test-only inputs to [`super::run_with`]. Production code calls [`super::run`].
///
/// For `cortexkit-doctor-conformance`, supply these hooks only through the
/// module binary's test-only switch. See [`super::run_with`].
pub struct Hooks<'a> {
    pub now: &'a dyn Fn() -> Instant,
    pub presence: Arc<Presence>,
    pub open_terminal: &'a dyn Fn() -> Option<Box<dyn Terminal>>,
    pub timing: Timing,
}

impl Default for Hooks<'_> {
    fn default() -> Self {
        Self {
            now: &Instant::now,
            presence: Arc::new(subc_presence::prompt),
            open_terminal: &terminal::open,
            timing: Timing::default(),
        }
    }
}

pub(super) fn confirm(
    hooks: &Hooks<'_>,
    entry: Instant,
    module_id: &ModuleId,
    finding: &Finding,
) -> Result<(), Stop> {
    let budget = || {
        if (hooks.now)().duration_since(entry) > hooks.timing.confirm_gate {
            Err(Stop::Refused("out_of_time"))
        } else {
            Ok(())
        }
    };
    budget()?;
    let steps = finding
        .fix()
        .plan()
        .expect("finding was verified")
        .steps()
        .join("; ");
    let text = format!("CortexKit {module_id}: {} Fix: {steps}", finding.summary());
    let prompt = text.chars().take(400).collect();
    match prompt_until(&hooks.presence, prompt, hooks.timing.prompt_limit)? {
        Outcome::Approved => Ok(()),
        Outcome::Declined
        | Outcome::AuthorizedWithoutPrompt
        | Outcome::LocalAuthenticationError {
            failure: LocalAuthenticationFailure::Declined,
            ..
        } => Err(Stop::Refused("not_confirmed")),
        _ => {
            budget()?;
            terminal_confirmation(hooks, module_id, &text)
        }
    }
}

#[derive(Default)]
enum PromptState {
    #[default]
    Pending,
    Published(Arc<dyn Withdraw>),
    TimedOut,
}

fn prompt_until(presence: &Arc<Presence>, text: String, limit: Duration) -> Result<Outcome, Stop> {
    let started = Instant::now();
    let (send, receive) = mpsc::channel();
    let state = Arc::new(Mutex::new(PromptState::Pending));
    let publish_state = state.clone();
    let publish = Box::new(move |handle: Arc<dyn Withdraw>| {
        {
            let mut state = publish_state.lock().expect("prompt state lock");
            if !matches!(*state, PromptState::TimedOut) {
                *state = PromptState::Published(handle);
                return;
            }
        }
        // The provider can publish while initializing, after the deadline.
        // Withdraw even then, without holding the lock over a provider call.
        handle.withdraw();
    });

    let presence = presence.clone();
    if thread::Builder::new()
        .spawn(move || {
            let result = callback(|| presence(&text, publish));
            let _ = send.send((started.elapsed(), result));
        })
        .is_err()
    {
        return Ok(Outcome::ProviderError);
    }

    if let Ok((elapsed, result)) = receive.recv_timeout(limit.saturating_sub(started.elapsed())) {
        if elapsed <= limit {
            return result;
        }
    }
    let previous = {
        let mut state = state.lock().expect("prompt state lock");
        std::mem::replace(&mut *state, PromptState::TimedOut)
    };
    if let PromptState::Published(handle) = previous {
        let _ = callback(|| handle.withdraw());
    }
    // Do not join the provider thread: some OS calls may still be blocked.
    Err(Stop::Refused("not_confirmed"))
}

fn terminal_confirmation(hooks: &Hooks<'_>, module_id: &ModuleId, text: &str) -> Result<(), Stop> {
    let no_confirmation = Stop::Refused("not_confirmed");
    let mut terminal = (hooks.open_terminal)().ok_or(Stop::Refused("no_terminal"))?;
    terminal
        .write_prompt(&format!("{text}\nType {module_id} to confirm: "))
        .map_err(|_| Stop::Refused("not_confirmed"))?;
    let deadline = Instant::now()
        .checked_add(hooks.timing.terminal_limit)
        .ok_or(Stop::Refused("not_confirmed"))?;
    if let Ok(TerminalRead::Line(line)) = terminal.read_line(deadline) {
        if let Some(line) = line.strip_suffix(b"\n") {
            if Instant::now() <= deadline
                && line.strip_suffix(b"\r").unwrap_or(line) == module_id.as_str().as_bytes()
            {
                return Ok(());
            }
        }
    }
    Err(no_confirmation)
}
