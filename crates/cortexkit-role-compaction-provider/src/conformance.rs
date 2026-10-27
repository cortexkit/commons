//! Focused probes for single provider rules, not a full conformance kit.
//!
//! Enable the non-default `conformance` feature to drive these rules against
//! a provider through an implementation-specific adapter:
//! [`conversation_state_ignores_project_root`] for conversation identity and
//! [`held_copies_follow_reasoning_pruned`] for held copies after an archive
//! prune.

use cortexkit_role_llm_runner::archive_prunes_reasoning::{PrunedPartAt, ReasoningPruned};
use serde_json::{json, Value};

mod transcripts;

pub use transcripts::{probe_parts, NotApplicable};
use transcripts::{PruneMode, Transcript};

use crate::{
    digest::content_digest,
    lineage::DescendsFrom,
    notice::{AdvanceNotice, NoticeDescent},
    setup::{SetupAnswer, SetupRequest, ACCEPTS_PRUNED_REASONING},
    view::{CompactionView, ViewRange},
};

/// The case name included in every failure returned by the probe.
pub const CASE_NAME: &str = "conversation_state_ignores_project_root";

/// The minimum adapter needed to observe a provider's per-session state.
///
/// Use a fresh test store. The adapter supplies the daemon-attested runner
/// principal's bare module id, never its `reserved:<id>` display form or any
/// prefix. Module-id arguments select real attested runner connections, never
/// body fields. Capture markers from provider state;
/// do not reconstruct them from the requested identity or route metadata.
pub trait StateProbe {
    /// Deliver Setup on a route with these identity fields and folder spelling,
    /// then read an opaque, nonempty marker for the provider's session state.
    /// Repeated Setup for the same state must return the same marker; distinct
    /// state must have distinct markers. Route bind harness/session may be
    /// generic/empty: the harness and session below are Setup's body fields.
    fn setup_and_state_marker(
        &mut self,
        runner_module_id: &str,
        harness: &str,
        session: &str,
        project_root: &str,
    ) -> Result<String, String>;
}

/// The same runner/harness/session through folder aliases reaches the same
/// provider state; another attested runner reaches different state.
///
/// The adapter's test environment must make `/var/probe-project` and
/// `/private/var/probe-project` spellings of the same project. Neither folder
/// spelling is allowed to participate in the provider's state key.
pub fn conversation_state_ignores_project_root(
    adapter: &mut impl StateProbe,
) -> Result<(), String> {
    let mut observe = |runner, root| {
        let marker = adapter
            .setup_and_state_marker(runner, "harness-a", "session-a", root)
            .map_err(|reason| format!("{CASE_NAME}: {reason}"))?;
        if marker.is_empty() {
            return Err(format!(
                "{CASE_NAME}: provider returned an empty state marker"
            ));
        }
        Ok(marker)
    };
    let original = observe("runner-a", "/var/probe-project")?;
    let alias = observe("runner-a", "/private/var/probe-project")?;
    if original != alias {
        return Err(format!(
            "{CASE_NAME}: folder aliases reached different state"
        ));
    }
    let other_runner = observe("runner-b", "/var/probe-project")?;
    if original == other_runner {
        return Err(format!("{CASE_NAME}: different runners shared state"));
    }
    Ok(())
}

/// The case name included in every failure of
/// [`held_copies_follow_reasoning_pruned`].
pub const HELD_COPIES_CASE: &str = "held_copies_follow_reasoning_pruned";

/// The bare module id of the runner the held-copies probe plays. The adapter
/// delivers its Setup calls and notices on routes the daemon attests as this
/// runner.
pub const PROBE_RUNNER: &str = "probe-runner";

/// The harness the held-copies probe's Setup requests name.
pub const PROBE_HARNESS: &str = "probe-harness";

/// The runner side of the held-copies probe: a scripted transcript the
/// provider reads back over its reverse `session.read` and `session.head`
/// calls. The adapter answers those calls for the probe's conversation with
/// [`ScriptedRunner::read`] and [`ScriptedRunner::head`], unchanged, and
/// never from a copy of its own.
#[derive(Debug)]
pub struct ScriptedRunner {
    state: std::sync::Mutex<RunnerState>,
}

#[derive(Debug)]
struct RunnerState {
    transcript: Transcript,
    lineage_id: String,
    first_ordinal: u64,
    /// Every message as written, by ordinal from 0, and as served now.
    originals: Vec<Value>,
    served: Vec<Value>,
    pruned: Vec<PrunedPartAt>,
    reasoning_pruned: Option<ReasoningPruned>,
    /// Ordinals served to the provider since the probe last cleared it.
    reads: Vec<u64>,
    /// Positions below the lineage's first ordinal the provider asked for.
    below_cut: Vec<u64>,
}

fn part_key(ordinal: u64, index: usize) -> String {
    format!("m{ordinal}#{index}")
}

impl ScriptedRunner {
    fn new(transcript: Transcript, lineage_id: &str, through: u64) -> Self {
        let messages: Vec<Value> = (0..=through).map(|o| transcript.message(o)).collect();
        Self {
            state: std::sync::Mutex::new(RunnerState {
                transcript,
                lineage_id: lineage_id.into(),
                first_ordinal: 0,
                originals: messages.clone(),
                served: messages,
                pruned: Vec::new(),
                reasoning_pruned: None,
                reads: Vec::new(),
                below_cut: Vec::new(),
            }),
        }
    }

    /// The serializer profile whose native format this runner serves, as
    /// the probe's Setup requests name it in `params.serializer_profile`;
    /// `None` for the generic transcript.
    pub fn serializer_profile(&self) -> Option<&'static str> {
        self.lock().transcript.profile()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RunnerState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Answer the provider's reverse `session.read` for this conversation:
    /// `Ok` with the page, or `Err` with an `ERROR` body. Raw view only;
    /// `from_ordinal`, `after_mid`, `lineage_id` and `limit` are honoured.
    pub fn read(&self, params: &Value) -> Result<Value, Value> {
        let mut guard = self.lock();
        let state = &mut *guard;
        let refuse = |code: &str, detail: Value| {
            Err(
                json!({"code":code,"message":format!("probe runner refused: {code}"),"detail":detail}),
            )
        };
        if let Some(lineage) = params.get("lineage_id").and_then(Value::as_str) {
            if lineage != state.lineage_id {
                return refuse("lineage_changed", Value::Null);
            }
        }
        let from = if let Some(from) = params.get("from_ordinal").and_then(Value::as_u64) {
            from
        } else if let Some(mid) = params.get("after_mid").and_then(Value::as_str) {
            match (0..state.served.len() as u64).find(|o| state.transcript.message_id(*o) == mid) {
                Some(ordinal) => ordinal + 1,
                None => return refuse("unknown_mid", Value::Null),
            }
        } else {
            state.first_ordinal
        };
        if from < state.first_ordinal {
            state.below_cut.push(from);
            return refuse(
                "before_lineage_start",
                json!({"first_ordinal": state.first_ordinal}),
            );
        }
        let end = state.served.len() as u64;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX);
        let stop = end.min(from.saturating_add(limit));
        let mut messages = Vec::new();
        let mut listed = Vec::new();
        for ordinal in from..stop {
            let original = &state.originals[ordinal as usize];
            let parts = state.transcript.parts(original);
            let mut entry = json!({
                "ordinal": ordinal,
                "mid": state.transcript.message_id(ordinal),
                "message": state.served[ordinal as usize],
                "digest": content_digest(original),
                "part_digests": parts.iter().enumerate().map(|(index, part)| json!({
                    "part": part_key(ordinal, index),
                    "digest": content_digest(part),
                })).collect::<Vec<_>>(),
            });
            if state.transcript.serves_tags() && !parts.is_empty() {
                // A descriptors read's rows: every part's key, in order.
                entry["tags"] = (0..parts.len())
                    .map(|index| json!({"part": part_key(ordinal, index)}))
                    .collect();
            }
            messages.push(entry);
            for pruned in state.pruned.iter().filter(|p| p.ordinal == ordinal) {
                let index: usize = pruned.part.rsplit('#').next().unwrap().parse().unwrap();
                let original_part = &parts[index];
                listed.push(json!({
                    "mid": state.transcript.message_id(ordinal),
                    "part": pruned.part,
                    "kind": pruned.kind,
                    "bytes": serde_jcs::to_vec(original_part).map(|b| b.len()).unwrap_or(0),
                    "sha256": content_digest(original_part),
                }));
            }
            state.reads.push(ordinal);
        }
        let mut page = json!({"messages": messages, "lineage_id": state.lineage_id});
        if stop < end {
            page["next_from_ordinal"] = json!(stop);
        }
        if !listed.is_empty() {
            page["pruned_reasoning"] = json!(listed);
        }
        if let Some(pruned) = state.reasoning_pruned {
            page["reasoning_pruned"] = serde_json::to_value(pruned).unwrap();
        }
        Ok(page)
    }

    /// Answer the provider's reverse `session.head` for this conversation.
    pub fn head(&self) -> Value {
        let state = self.lock();
        let mut head = json!({
            "lineage_id": state.lineage_id,
            "last_ordinal": state.served.len() as u64 - 1,
            "first_ordinal": state.first_ordinal,
            "history_version": 1,
            "updated_at": 1_767_225_600_000u64,
        });
        if state.first_ordinal > 0 {
            let cut = state.first_ordinal - 1;
            head["cut"] = json!({
                "mid": state.transcript.message_id(cut),
                "digest": content_digest(&state.originals[cut as usize]),
                "written_at": 1_767_225_600_000u64,
            });
        }
        if let Some(pruned) = state.reasoning_pruned {
            head["reasoning_pruned"] = serde_json::to_value(pruned).unwrap();
        }
        head
    }

    fn append_through(&self, through: u64) {
        let mut state = self.lock();
        while (state.served.len() as u64) <= through {
            let message = state.transcript.message(state.served.len() as u64);
            state.originals.push(message.clone());
            state.served.push(message);
        }
    }

    fn cut(&self, lineage_id: &str, first_ordinal: u64) {
        let mut state = self.lock();
        state.lineage_id = lineage_id.into();
        state.first_ordinal = first_ordinal;
    }

    /// Prune every assistant message at or below `through`, in the served
    /// transcript's own terms (`llm-runner/v1` §9.14.1, per-profile rules):
    /// empty the reasoning, remove the opaque payload where the format
    /// carries one beside a summary, and remove or empty the signatures on
    /// the tool call and the state-carrying text. Raise the generation in
    /// the same step.
    fn prune_reasoning_through(&self, through: u64, mode: PruneMode) -> ReasoningPruned {
        let mut state = self.lock();
        let transcript = state.transcript;
        for ordinal in (1..=through).step_by(2) {
            if state.pruned.iter().any(|p| p.ordinal == ordinal) {
                continue;
            }
            for (index, kind, served) in transcript.prunes(ordinal, mode) {
                let parts = transcript
                    .parts_mut(&mut state.served[ordinal as usize])
                    .expect("scripted answers have a part array");
                parts[index] = served;
                state
                    .pruned
                    .push(PrunedPartAt::new(ordinal, part_key(ordinal, index), kind));
            }
        }
        let generation = state.reasoning_pruned.map_or(1, |p| p.generation + 1);
        let pruned = ReasoningPruned::new(generation, through, 1_767_312_000_000);
        state.reasoning_pruned = Some(pruned);
        pruned
    }

    fn take_reads(&self) -> Vec<u64> {
        std::mem::take(&mut self.lock().reads)
    }

    fn below_cut(&self) -> Vec<u64> {
        self.lock().below_cut.clone()
    }

    fn digest_at(&self, ordinal: u64) -> String {
        content_digest(&self.lock().originals[ordinal as usize])
    }

    fn pruned(&self) -> Vec<PrunedPartAt> {
        self.lock().pruned.clone()
    }

    fn first_ordinal(&self) -> u64 {
        self.lock().first_ordinal
    }
}

/// The adapter for [`held_copies_follow_reasoning_pruned`]. Use a fresh
/// provider store. The provider's reverse calls for the probe's conversation
/// go to the [`ScriptedRunner`] passed in, and nowhere else.
pub trait PrunedReasoningProbe {
    /// The serializer profile (`llm-runner/v1` §4.4) the provider is set up
    /// with: the value of its plan item's `params.serializer_profile`. The
    /// probe sends it there in every Setup request and serves its transcript
    /// in that profile's native format: `anthropic-messages`,
    /// `opencode-aisdk`, `owned-broca` or `pi`. `None`, the default, sends
    /// no profile and serves the generic transcript of earlier releases. A
    /// profile the runner contract does not name fails the probe.
    fn serializer_profile(&self) -> Option<String> {
        None
    }

    /// Deliver `request` as `compaction.setup` on a route the daemon attests
    /// as [`PROBE_RUNNER`], and return the provider's answer as it sent it.
    fn setup(
        &mut self,
        runner: &ScriptedRunner,
        request: &SetupRequest,
    ) -> Result<SetupAnswer, String>;

    /// Publish `notice` to the provider as the runner's advance notice, and
    /// return once the provider has finished the reads it makes for it.
    fn notice(&mut self, runner: &ScriptedRunner, notice: &AdvanceNotice) -> Result<(), String>;

    /// Have the provider build its next view of the conversation now, and
    /// return it as published. Configure the provider (its preset or params)
    /// so the view renders the messages it holds as held, in replace ranges
    /// (a verbatim fold) or keep-range part edits: a view that neither
    /// replaces nor edits a pruned message checks nothing and fails the
    /// probe.
    fn next_view(
        &mut self,
        runner: &ScriptedRunner,
        session_ref: &str,
    ) -> Result<CompactionView, String>;
}

struct Conversation<'a, A: PrunedReasoningProbe> {
    adapter: &'a mut A,
    runner: ScriptedRunner,
    session: String,
    session_ref: String,
    seq: u64,
    setups: u64,
}

impl<A: PrunedReasoningProbe> Conversation<'_, A> {
    fn fail(&self, why: impl std::fmt::Display) -> String {
        format!("{HELD_COPIES_CASE}: {why}")
    }

    /// Setup's `params`: the serializer profile, when the provider has one.
    fn params(&self) -> serde_json::Map<String, Value> {
        let mut params = serde_json::Map::new();
        if let Some(profile) = self.runner.serializer_profile() {
            params.insert("serializer_profile".into(), json!(profile));
        }
        params
    }

    fn setup(
        &mut self,
        lineage_id: &str,
        descends_from: Option<DescendsFrom>,
        pruned: Option<ReasoningPruned>,
    ) -> Result<(), String> {
        self.setups += 1;
        let mut request = SetupRequest::new(
            self.session.clone(),
            PROBE_HARNESS,
            format!("probe-setup-{}", self.setups),
            serde_json::Map::new(),
            "probe/model",
            1_767_225_600_000,
        )
        .with_lineage(lineage_id);
        request.params = self.params();
        if let Some(descent) = descends_from {
            request = request.with_descends_from(descent);
        }
        if let Some(pruned) = pruned {
            request = request.with_reasoning_pruned(pruned);
        }
        let answer = self
            .adapter
            .setup(&self.runner, &request)
            .map_err(|e| self.fail(e))?;
        if !matches!(answer, SetupAnswer::Ready { .. })
            || !answer
                .accepts()
                .iter()
                .any(|a| a == ACCEPTS_PRUNED_REASONING)
        {
            return Err(self.fail(
                "the provider's Setup answer does not accept pruned_reasoning; the probe \
                 applies only to providers that do",
            ));
        }
        Ok(())
    }

    fn notice(
        &mut self,
        lineage_id: &str,
        through: u64,
        descent: Option<NoticeDescent>,
        pruned: Option<ReasoningPruned>,
    ) -> Result<(), String> {
        self.seq += 1;
        let mut notice = AdvanceNotice::new(
            self.session_ref.clone(),
            self.seq,
            lineage_id,
            through,
            self.runner.digest_at(through),
            1000,
            "probe/model",
        )
        .with_first_ordinal(self.runner.first_ordinal());
        if let Some(descent) = descent {
            notice = notice.with_descends_from(descent);
        }
        if let Some(pruned) = pruned {
            notice = notice.with_reasoning_pruned(pruned);
        }
        self.adapter
            .notice(&self.runner, &notice)
            .map_err(|e| self.fail(e))
    }

    /// Check a view against the runner's prune record: no replace range may
    /// carry content the prune removed from an ordinal it stands for, and at
    /// least one must stand for a pruned ordinal, or the view checked nothing.
    fn check_view(&mut self) -> Result<(), String> {
        let view = self
            .adapter
            .next_view(&self.runner, &self.session_ref)
            .map_err(|e| self.fail(e))?;
        let pruned = self.runner.pruned();
        let first = self.runner.first_ordinal();
        let transcript = self.runner.lock().transcript;
        let restored = transcript
            .restored(&view, &pruned, first)
            .map_err(|e| self.fail(e))?;
        if let Some(part) = restored {
            return Err(self.fail(format!(
                "the view renders pruned content of part {} at ordinal {} from a held copy",
                part.part, part.ordinal
            )));
        }
        // Something was checked only if a replace range stands for a pruned
        // ordinal or a part edit sends bytes into a pruned message.
        let checked = view
            .ranges
            .iter()
            .enumerate()
            .any(|(index, range)| match range {
                ViewRange::Replace { from, to, .. } => {
                    // The head range of a cut lineage carries everything before it.
                    let start = if index == 0 && first > 0 { 0 } else { *from };
                    pruned.iter().any(|p| (start..*to).contains(&p.ordinal))
                }
                ViewRange::Keep { edits, .. } => edits
                    .iter()
                    .any(|e| e.bytes.is_some() && pruned.iter().any(|p| p.ordinal == e.ordinal)),
            });
        if !checked {
            return Err(self.fail(
                "the view neither replaces nor edits a pruned message, so it checks nothing; \
                 configure the provider to fold held messages verbatim",
            ));
        }
        Ok(())
    }

    /// After a generation change, every held ordinal from the cut through
    /// `through` was read again, and nothing below the cut was asked for.
    fn check_reconciled(&self, through: u64) -> Result<(), String> {
        if let Some(below) = self.runner.below_cut().first() {
            return Err(self.fail(format!(
                "the provider read from ordinal {below}, below its cut at {}",
                self.runner.first_ordinal()
            )));
        }
        let reads = self.runner.take_reads();
        for ordinal in self.runner.first_ordinal()..=through {
            if !reads.contains(&ordinal) {
                return Err(self.fail(format!(
                    "after the prune generation changed, the provider did not re-read its held \
                     ordinal {ordinal}"
                )));
            }
        }
        Ok(())
    }

    /// A notice repeating the reconciled generation re-reads nothing held
    /// below the provider's newest held ordinal.
    ///
    /// The newest held ordinal itself may be read again. A provider can keep
    /// a standing continuity read: on every notice it reads from the newest
    /// ordinal it holds onward, a one-entry overlap, to confirm the
    /// transcript still continues from what it holds. When the reconciled
    /// `through` is the newest message, that overlap touches `through`
    /// without reconciling anything. A second reconcile reads held ordinals
    /// from the cut onward, so it always reads one below the newest held
    /// ordinal, and that is what fails. The newest held ordinal is the
    /// runner's newest served one, since the provider read through it
    /// before this notice.
    fn check_no_second_reconcile(&mut self) -> Result<(), String> {
        let lineage = self.runner.lock().lineage_id.clone();
        let last = self.runner.lock().served.len() as u64 - 1;
        let pruned = self.runner.lock().reasoning_pruned;
        self.notice(&lineage, last, None, pruned)?;
        self.check_view()?;
        if let Some(ordinal) = self.runner.take_reads().iter().find(|o| **o < last) {
            return Err(self.fail(format!(
                "the provider re-read ordinal {ordinal} again although the generation it \
                 reconciled had not changed"
            )));
        }
        Ok(())
    }
}

/// A provider that accepts pruned reasoning reconciles its held copies when
/// the runner's prune generation changes, and never renders pruned content
/// from a copy it holds (`compaction-provider/v1` §4, `llm-runner/v1`
/// §9.14.1).
///
/// The probe plays a runner in two conversations. In the first, it prunes
/// messages the provider already read, calls the post-prune Setup carrying
/// the new `reasoning_pruned`, and checks that the provider re-read every
/// held message at or below `through_ordinal`, that its next view renders no
/// pruned content in any replace range or part edit (the prune removes
/// reasoning, a tool call's thought signature and a provider-state
/// signature, so an edit re-serializing a held part fails too), and that a
/// later notice with the same generation causes no second re-read: it may
/// read from its newest held message onward, a continuity overlap, but
/// nothing held below it. In the second, a cut first moves the conversation to a
/// lineage starting at ordinal 2, so the provider holds copies from before
/// its cut; after the prune it must re-read only from the cut and must not
/// render those older copies' pruned content. Both prune with the
/// `reasoning` and `signature` kinds. A profile transcript adds a third
/// conversation, pruning with `opaque_reasoning` and `signature`, where only
/// opaque payloads go and readable reasoning stays; a runner's check would
/// refuse that readable reasoning beside a `reasoning` prune in the same
/// fold, so the kinds are not mixed.
///
/// The transcript follows the adapter's
/// [`PrunedReasoningProbe::serializer_profile`]. A logical part or prune
/// kind that profile cannot carry is not checked: `Ok` here means every
/// part the profile carries passed. Use
/// [`held_copies_follow_reasoning_pruned_report`] to see which were checked
/// and which were not applicable, by name.
pub fn held_copies_follow_reasoning_pruned(
    adapter: &mut impl PrunedReasoningProbe,
) -> Result<(), String> {
    held_copies_follow_reasoning_pruned_report(adapter).map(|_| ())
}

/// What [`held_copies_follow_reasoning_pruned_report`] checked: the
/// transcript's profile, the logical parts ([`probe_parts`]) and prune
/// kinds it carried, and those its profile cannot carry, which were not
/// checked and did not pass.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeldCopiesReport {
    /// The serializer profile whose native transcript was served; `None`
    /// for the generic transcript.
    pub profile: Option<&'static str>,
    pub parts_checked: Vec<&'static str>,
    pub kinds_checked: Vec<&'static str>,
    pub parts_not_applicable: Vec<NotApplicable>,
    pub kinds_not_applicable: Vec<NotApplicable>,
}

/// [`held_copies_follow_reasoning_pruned`], returning on success what was
/// checked and what was not applicable to the provider's serializer
/// profile.
pub fn held_copies_follow_reasoning_pruned_report(
    adapter: &mut impl PrunedReasoningProbe,
) -> Result<HeldCopiesReport, String> {
    let profile = adapter.serializer_profile();
    let transcript = Transcript::for_profile(profile.as_deref()).map_err(|e| {
        format!(
            "{HELD_COPIES_CASE}: the adapter's serializer profile {:?} is not one the \
                 runner contract names ({e})",
            e.profile
        )
    })?;
    let mut conversations = vec![
        ("no-cut", false, PruneMode::Reasoning),
        ("cut", true, PruneMode::Reasoning),
    ];
    if transcript.opaque_mode() {
        conversations.push(("opaque", false, PruneMode::Opaque));
    }
    for (label, cut, mode) in conversations {
        let session = format!("probe-pruned-{label}");
        let session_ref =
            cortexkit_role_llm_runner::session_ref(&cortexkit_role_llm_runner::SessionIdentity {
                runner: PROBE_RUNNER,
                harness: PROBE_HARNESS,
                session: &session,
            });
        // Every notice ends on an even ordinal: a prompt or a tool result.
        // Where a profile stores a call's result in the next message, an
        // answer at an odd ordinal ends on a call with no result yet, which
        // a runner does announce but which is not what this probe is about;
        // ending on a complete call and result keeps the probe to pruning.
        let mut conversation = Conversation {
            adapter: &mut *adapter,
            runner: ScriptedRunner::new(transcript, "probe-lineage-a", 4),
            session,
            session_ref,
            seq: 0,
            setups: 0,
        };
        conversation.setup("probe-lineage-a", None, None)?;
        conversation.notice("probe-lineage-a", 4, None, None)?;
        let mut through = 4;
        if cut {
            // A cut after ordinal 1: the new lineage starts at 2, and the
            // provider keeps its copies of 0 and 1 from before it.
            conversation.runner.append_through(6);
            conversation.runner.cut("probe-lineage-b", 2);
            let descent = DescendsFrom {
                lineage_id: "probe-lineage-a".into(),
                through_ordinal: 1,
            };
            conversation.setup("probe-lineage-b", Some(descent), None)?;
            let digest = conversation.runner.digest_at(1);
            conversation.notice(
                "probe-lineage-b",
                6,
                Some(NoticeDescent::new("probe-lineage-a", 1, digest)),
                None,
            )?;
            through = 6;
        }
        conversation.runner.take_reads();
        let pruned = conversation.runner.prune_reasoning_through(through, mode);
        let lineage = conversation.runner.lock().lineage_id.clone();
        conversation.setup(&lineage, None, Some(pruned))?;
        conversation.check_view()?;
        conversation.check_reconciled(through)?;
        conversation.check_no_second_reconcile()?;
    }
    Ok(HeldCopiesReport {
        profile: transcript.profile(),
        parts_checked: transcript.parts_checked(),
        kinds_checked: transcript.kinds_checked(),
        parts_not_applicable: transcript.parts_not_applicable(),
        kinds_not_applicable: transcript.kinds_not_applicable(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_role_llm_runner::serializer_profile::{
        ANTHROPIC_MESSAGES, OPENCODE_AISDK, OWNED_BROCA, PI,
    };
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeProvider {
        folder_keyed: bool,
        ignores_runner: bool,
        runner_ids: Vec<String>,
        states: BTreeMap<(String, String, String), String>,
    }

    impl StateProbe for FakeProvider {
        fn setup_and_state_marker(
            &mut self,
            runner_module_id: &str,
            harness: &str,
            session: &str,
            project_root: &str,
        ) -> Result<String, String> {
            self.runner_ids.push(runner_module_id.into());
            let identity = if self.folder_keyed {
                project_root
            } else if self.ignores_runner {
                ""
            } else {
                runner_module_id
            };
            let key = (identity.into(), harness.into(), session.into());
            let next_marker = format!("state-{}", self.states.len());
            Ok(self.states.entry(key).or_insert(next_marker).clone())
        }
    }

    #[test]
    fn faithful_provider_passes() {
        conversation_state_ignores_project_root(&mut FakeProvider::default()).unwrap();
    }

    #[test]
    fn probe_uses_bare_runner_module_ids() {
        let mut provider = FakeProvider::default();
        conversation_state_ignores_project_root(&mut provider).unwrap();
        assert_eq!(provider.runner_ids, ["runner-a", "runner-a", "runner-b"]);
    }

    #[test]
    fn folder_keyed_provider_fails_by_name() {
        let failure = conversation_state_ignores_project_root(&mut FakeProvider {
            folder_keyed: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(
            failure,
            format!("{CASE_NAME}: folder aliases reached different state")
        );
    }

    #[test]
    fn runner_blind_provider_fails_by_name() {
        let failure = conversation_state_ignores_project_root(&mut FakeProvider {
            ignores_runner: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(
            failure,
            format!("{CASE_NAME}: different runners shared state")
        );
    }

    /// A deliberately simple provider for the held-copies probe: it holds
    /// raw copies by ordinal and folds them verbatim into one replace range.
    #[derive(Default)]
    struct HoldingProvider {
        fault: Option<HoldingFault>,
        /// The serializer profile the provider is set up with.
        profile: Option<String>,
        /// The one conversation this provider holds state for; a Setup for
        /// another starts from nothing, as a per-session store would.
        session: String,
        held: BTreeMap<u64, Value>,
        /// Each held message's `mid`, as the runner served it.
        mids: BTreeMap<u64, String>,
        /// Render through keep-range part edits instead of one fold.
        edits_mode: bool,
        /// The copies part edits are built from. A correct provider keeps it
        /// equal to `held`; the stale-index faults never refresh it.
        part_index: BTreeMap<u64, Value>,
        lineage: String,
        first_ordinal: u64,
        cursor: u64,
        reconciled: Option<u64>,
        pre_cut_through: Option<u64>,
        version: u64,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum HoldingFault {
        IgnoresGeneration,
        RendersPreCutCopy,
        ReadsBelowCut,
        RereadsEveryNotice,
        /// Trims a tool output in a part edit built from a held pre-prune
        /// copy, re-serializing the call's thought signature.
        EditsHeldToolOutput,
        /// Trims `text_with_state` text in a part edit built from a held
        /// pre-prune copy, re-serializing its provider-state signature.
        EditsHeldStateText,
        /// Reconciles after the `reasoning` prunes but not after the
        /// `opaque_reasoning` one, so it renders held opaque payloads.
        IgnoresOpaquePrune,
    }

    impl HoldingProvider {
        fn with(fault: HoldingFault) -> Self {
            Self {
                fault: Some(fault),
                ..Self::default()
            }
        }

        fn editing(fault: Option<HoldingFault>) -> Self {
            Self {
                fault,
                edits_mode: true,
                ..Self::default()
            }
        }

        fn on(profile: &str, fault: Option<HoldingFault>, edits_mode: bool) -> Self {
            Self {
                fault,
                edits_mode,
                profile: Some(profile.into()),
                ..Self::default()
            }
        }

        fn transcript(&self) -> Transcript {
            Transcript::for_profile(self.profile.as_deref()).unwrap()
        }

        /// Part edits that trim each held answer's tool call (its output
        /// where the part holds one, else its arguments) and its
        /// state-carrying text, re-serializing the rest of each part.
        /// `anthropic-messages` has neither signature, so its edits trim the
        /// `tool_use` input and the plain text instead.
        fn trim_edits(&self) -> Vec<crate::view::PartEdit> {
            let transcript = self.transcript();
            let (call_index, text_index) = match transcript.profile() {
                None => (2, 3),
                Some(ANTHROPIC_MESSAGES) => (3, 2),
                Some(_) => (3, 4),
            };
            let mut edits = Vec::new();
            for (ordinal, current) in self.held.range(self.first_ordinal..) {
                if ordinal.is_multiple_of(2) {
                    continue;
                }
                let source = |stale: HoldingFault| {
                    if self.fault == Some(stale) {
                        &self.part_index[ordinal]
                    } else {
                        current
                    }
                };
                let mut call =
                    transcript.parts(source(HoldingFault::EditsHeldToolOutput))[call_index].clone();
                let trimmed = json!("[trimmed]");
                match transcript.profile() {
                    None => call["result"]["output"] = trimmed,
                    Some(OPENCODE_AISDK) => call["state"]["output"] = trimmed,
                    Some(PI) => call["arguments"] = json!({"door": trimmed}),
                    Some(_) => call["input"] = json!({"door": trimmed}),
                }
                let mut text =
                    transcript.parts(source(HoldingFault::EditsHeldStateText))[text_index].clone();
                text["text"] = json!("[text trimmed]");
                let mut pair = [(call_index, call), (text_index, text)];
                pair.sort_by_key(|(index, _)| *index);
                for (index, part) in pair {
                    edits.push(crate::view::PartEdit::replace(
                        *ordinal,
                        self.mids[ordinal].clone(),
                        format!("m{ordinal}#{index}"),
                        serde_json::to_string(&part).unwrap(),
                    ));
                }
            }
            edits
        }

        fn store(&mut self, page: &Value) {
            for entry in page["messages"].as_array().unwrap() {
                let ordinal = entry["ordinal"].as_u64().unwrap();
                self.held.insert(ordinal, entry["message"].clone());
                self.mids
                    .insert(ordinal, entry["mid"].as_str().unwrap().to_owned());
                let stale = matches!(
                    self.fault,
                    Some(HoldingFault::EditsHeldToolOutput | HoldingFault::EditsHeldStateText)
                );
                if !(stale && self.part_index.contains_key(&ordinal)) {
                    self.part_index.insert(ordinal, entry["message"].clone());
                }
                self.cursor = self.cursor.max(ordinal + 1);
            }
        }

        fn read_forward(&mut self, runner: &ScriptedRunner) {
            let from = self.cursor.max(self.first_ordinal);
            let page = runner.read(&json!({"from_ordinal": from})).unwrap();
            self.store(&page);
        }

        fn reconcile(&mut self, runner: &ScriptedRunner, state: ReasoningPruned) {
            if self.fault == Some(HoldingFault::IgnoresGeneration)
                || (self.fault == Some(HoldingFault::IgnoresOpaquePrune)
                    && self.session == "probe-pruned-opaque")
            {
                return;
            }
            if self.reconciled == Some(state.generation)
                && self.fault != Some(HoldingFault::RereadsEveryNotice)
            {
                return;
            }
            let from = if self.fault == Some(HoldingFault::ReadsBelowCut) {
                0
            } else {
                self.first_ordinal
            };
            let last = self.held.keys().next_back().copied().unwrap_or(0);
            let to = last.min(state.through_ordinal);
            let page = runner
                .read(&json!({"from_ordinal": from, "limit": to + 1 - from}))
                .or_else(|_| {
                    runner.read(&json!({
                        "from_ordinal": self.first_ordinal,
                        "limit": to + 1 - self.first_ordinal,
                    }))
                })
                .unwrap();
            self.store(&page);
            if self.first_ordinal > 0 {
                self.pre_cut_through = Some(state.through_ordinal);
            }
            self.reconciled = Some(state.generation);
        }
    }

    impl PrunedReasoningProbe for HoldingProvider {
        fn serializer_profile(&self) -> Option<String> {
            self.profile.clone()
        }

        fn setup(
            &mut self,
            runner: &ScriptedRunner,
            request: &SetupRequest,
        ) -> Result<SetupAnswer, String> {
            if request.session != self.session {
                *self = Self {
                    fault: self.fault,
                    profile: self.profile.take(),
                    edits_mode: self.edits_mode,
                    session: request.session.clone(),
                    ..Self::default()
                };
            }
            // The probe names the provider's profile where a plan item
            // would: in Setup's params.
            assert_eq!(
                request
                    .params
                    .get("serializer_profile")
                    .and_then(Value::as_str),
                self.profile.as_deref()
            );
            if let Some(lineage) = &request.lineage_id {
                self.lineage = lineage.clone();
            }
            if let Some(state) = request.reasoning_pruned {
                self.reconcile(runner, state);
            }
            Ok(SetupAnswer::ready(
                request.request_id.clone(),
                crate::message::CompactionMessage::new(
                    "probe-head",
                    1,
                    crate::message::Range::new(0, 0),
                    vec![],
                ),
            )
            .with_accepts(vec![ACCEPTS_PRUNED_REASONING.into()]))
        }

        fn notice(
            &mut self,
            runner: &ScriptedRunner,
            notice: &AdvanceNotice,
        ) -> Result<(), String> {
            self.lineage = notice.lineage_id.clone();
            self.first_ordinal = notice.first_ordinal.unwrap_or(0);
            if let Some(state) = notice.reasoning_pruned {
                self.reconcile(runner, state);
            }
            self.read_forward(runner);
            Ok(())
        }

        fn next_view(
            &mut self,
            _runner: &ScriptedRunner,
            session_ref: &str,
        ) -> Result<CompactionView, String> {
            let last = *self.held.keys().next_back().unwrap();
            let mut messages = Vec::new();
            let mut folded = 0;
            for (ordinal, message) in self.held.range(..self.first_ordinal) {
                let reconciled = self.pre_cut_through.is_some_and(|t| *ordinal <= t);
                if reconciled && self.fault != Some(HoldingFault::RendersPreCutCopy) {
                    folded += 1;
                } else {
                    messages.push(message.clone());
                }
            }
            if folded > 0 {
                messages.insert(
                    0,
                    json!({"role":"user","content":[{"type":"text","text":format!("{folded} earlier messages, summarised.")}]}),
                );
            }
            let ranges = if self.edits_mode {
                let mut ranges = Vec::new();
                if self.first_ordinal > 0 {
                    ranges.push(ViewRange::replace(
                        self.first_ordinal,
                        self.first_ordinal,
                        serde_json::to_string(&messages).unwrap(),
                    ));
                }
                ranges.push(ViewRange::keep_with_edits(
                    self.first_ordinal,
                    last + 1,
                    self.trim_edits(),
                ));
                ranges
            } else {
                messages.extend(
                    self.held
                        .range(self.first_ordinal..)
                        .map(|(_, m)| m.clone()),
                );
                vec![ViewRange::replace(
                    self.first_ordinal,
                    last + 1,
                    serde_json::to_string(&messages).unwrap(),
                )]
            };
            self.version += 1;
            Ok(CompactionView::new(
                session_ref,
                self.version,
                self.lineage.clone(),
                crate::lineage::MessageRef {
                    ordinal: last,
                    mid: self.mids[&last].clone(),
                },
                ranges,
            ))
        }
    }

    #[test]
    fn reconciling_provider_passes_the_held_copies_probe() {
        held_copies_follow_reasoning_pruned(&mut HoldingProvider::default()).unwrap();
    }

    /// A reconciling provider with a standing continuity read: on every
    /// notice it reads from the newest ordinal it holds onward, so the first
    /// entry served overlaps what it holds, and it checks that entry's `mid`
    /// still matches before storing what follows. Everything else is
    /// [`HoldingProvider`]'s.
    #[derive(Default)]
    struct OverlapReadingProvider {
        inner: HoldingProvider,
    }

    impl OverlapReadingProvider {
        fn on(profile: Option<&str>) -> Self {
            Self {
                inner: HoldingProvider {
                    profile: profile.map(Into::into),
                    ..HoldingProvider::default()
                },
            }
        }
    }

    impl PrunedReasoningProbe for OverlapReadingProvider {
        fn serializer_profile(&self) -> Option<String> {
            self.inner.serializer_profile()
        }

        fn setup(
            &mut self,
            runner: &ScriptedRunner,
            request: &SetupRequest,
        ) -> Result<SetupAnswer, String> {
            self.inner.setup(runner, request)
        }

        fn notice(
            &mut self,
            runner: &ScriptedRunner,
            notice: &AdvanceNotice,
        ) -> Result<(), String> {
            let inner = &mut self.inner;
            inner.lineage = notice.lineage_id.clone();
            inner.first_ordinal = notice.first_ordinal.unwrap_or(0);
            if let Some(state) = notice.reasoning_pruned {
                inner.reconcile(runner, state);
            }
            let newest = inner
                .held
                .range(inner.first_ordinal..)
                .next_back()
                .map(|(ordinal, _)| *ordinal);
            let from = newest.unwrap_or(inner.first_ordinal);
            let page = runner
                .read(&json!({"from_ordinal": from}))
                .map_err(|e| e.to_string())?;
            if let Some(newest) = newest {
                let served = page["messages"][0]["mid"].as_str();
                if served != Some(inner.mids[&newest].as_str()) {
                    return Err(format!(
                        "the transcript no longer continues from held ordinal {newest}"
                    ));
                }
            }
            inner.store(&page);
            Ok(())
        }

        fn next_view(
            &mut self,
            runner: &ScriptedRunner,
            session_ref: &str,
        ) -> Result<CompactionView, String> {
            self.inner.next_view(runner, session_ref)
        }
    }

    #[test]
    fn provider_with_a_one_entry_overlap_read_passes_on_every_transcript() {
        let profiles = std::iter::once(None).chain(PROFILES.map(Some));
        for profile in profiles {
            held_copies_follow_reasoning_pruned(&mut OverlapReadingProvider::on(profile))
                .unwrap_or_else(|e| panic!("{profile:?}: {e}"));
        }
    }

    #[test]
    fn provider_rendering_a_held_pre_prune_copy_fails_by_name() {
        let failure = held_copies_follow_reasoning_pruned(&mut HoldingProvider::with(
            HoldingFault::IgnoresGeneration,
        ))
        .unwrap_err();
        assert!(
            failure.starts_with(&format!("{HELD_COPIES_CASE}: ")),
            "{failure}"
        );
        assert!(
            failure.contains("renders pruned content of part m1#0 at ordinal 1"),
            "{failure}"
        );
    }

    #[test]
    fn provider_rendering_a_copy_from_before_its_cut_fails_by_name() {
        let failure = held_copies_follow_reasoning_pruned(&mut HoldingProvider::with(
            HoldingFault::RendersPreCutCopy,
        ))
        .unwrap_err();
        assert_eq!(
            failure,
            format!(
                "{HELD_COPIES_CASE}: the view renders pruned content of part m1#0 at ordinal 1 \
                 from a held copy"
            )
        );
    }

    #[test]
    fn provider_reading_below_its_cut_fails_by_name() {
        let failure = held_copies_follow_reasoning_pruned(&mut HoldingProvider::with(
            HoldingFault::ReadsBelowCut,
        ))
        .unwrap_err();
        assert_eq!(
            failure,
            format!("{HELD_COPIES_CASE}: the provider read from ordinal 0, below its cut at 2")
        );
    }

    #[test]
    fn provider_re_reading_without_a_generation_change_fails_by_name() {
        let failure = held_copies_follow_reasoning_pruned(&mut HoldingProvider::with(
            HoldingFault::RereadsEveryNotice,
        ))
        .unwrap_err();
        assert_eq!(
            failure,
            format!(
                "{HELD_COPIES_CASE}: the provider re-read ordinal 0 again although the \
                 generation it reconciled had not changed"
            )
        );
    }

    #[test]
    fn reconciling_provider_rendering_part_edits_passes_the_held_copies_probe() {
        held_copies_follow_reasoning_pruned(&mut HoldingProvider::editing(None)).unwrap();
    }

    #[test]
    fn provider_editing_a_held_tool_output_fails_by_name() {
        let failure = held_copies_follow_reasoning_pruned(&mut HoldingProvider::editing(Some(
            HoldingFault::EditsHeldToolOutput,
        )))
        .unwrap_err();
        assert_eq!(
            failure,
            format!(
                "{HELD_COPIES_CASE}: the view renders pruned content of part m1#2 at ordinal 1 \
                 from a held copy"
            )
        );
    }

    #[test]
    fn provider_editing_held_state_text_fails_by_name() {
        let failure = held_copies_follow_reasoning_pruned(&mut HoldingProvider::editing(Some(
            HoldingFault::EditsHeldStateText,
        )))
        .unwrap_err();
        assert_eq!(
            failure,
            format!(
                "{HELD_COPIES_CASE}: the view renders pruned content of part m1#3 at ordinal 1 \
                 from a held copy"
            )
        );
    }

    const PROFILES: [&str; 4] = [ANTHROPIC_MESSAGES, OPENCODE_AISDK, OWNED_BROCA, PI];

    fn names(list: &[NotApplicable]) -> Vec<&'static str> {
        list.iter().map(|n| n.name).collect()
    }

    #[test]
    fn reconciling_provider_passes_on_every_profile_transcript() {
        for profile in PROFILES {
            for edits_mode in [false, true] {
                let report = held_copies_follow_reasoning_pruned_report(&mut HoldingProvider::on(
                    profile, None, edits_mode,
                ))
                .unwrap_or_else(|e| panic!("{profile}: {e}"));
                assert_eq!(report.profile, Some(profile));
            }
        }
    }

    #[test]
    fn reports_name_what_each_transcript_cannot_carry() {
        let generic =
            held_copies_follow_reasoning_pruned_report(&mut HoldingProvider::default()).unwrap();
        assert_eq!(generic.profile, None);
        assert_eq!(names(&generic.parts_not_applicable), ["opaque_reasoning"]);
        assert_eq!(names(&generic.kinds_not_applicable), ["opaque_reasoning"]);
        assert_eq!(generic.kinds_checked, ["reasoning", "signature"]);
        for profile in PROFILES {
            let report = held_copies_follow_reasoning_pruned_report(&mut HoldingProvider::on(
                profile, None, false,
            ))
            .unwrap();
            let (parts_missing, kinds_missing): (&[&str], &[&str]) = match profile {
                ANTHROPIC_MESSAGES => (&["thought_signature", "text_with_state"], &["signature"]),
                OWNED_BROCA => (&["text_with_state"], &[]),
                _ => (&[], &[]),
            };
            assert_eq!(
                names(&report.parts_not_applicable),
                parts_missing,
                "{profile}"
            );
            assert_eq!(
                names(&report.kinds_not_applicable),
                kinds_missing,
                "{profile}"
            );
            for name in parts_missing {
                assert!(!report.parts_checked.contains(name), "{profile}: {name}");
            }
            for kind in kinds_missing {
                assert!(!report.kinds_checked.contains(kind), "{profile}: {kind}");
            }
            assert_eq!(
                report.parts_checked.len() + parts_missing.len(),
                probe_parts::ALL.len()
            );
        }
    }

    #[test]
    fn named_faults_fail_by_name_on_every_profile_transcript() {
        let at = |part: &str| {
            format!(
                "{HELD_COPIES_CASE}: the view renders pruned content of part {part} at ordinal 1 \
                 from a held copy"
            )
        };
        for profile in PROFILES {
            let run = |fault, edits_mode| {
                held_copies_follow_reasoning_pruned_report(&mut HoldingProvider::on(
                    profile,
                    Some(fault),
                    edits_mode,
                ))
            };
            assert_eq!(
                run(HoldingFault::IgnoresGeneration, false).unwrap_err(),
                at("m1#0"),
                "{profile}"
            );
            assert_eq!(
                run(HoldingFault::RendersPreCutCopy, false).unwrap_err(),
                at("m1#0"),
                "{profile}"
            );
            assert_eq!(
                run(HoldingFault::ReadsBelowCut, false).unwrap_err(),
                format!("{HELD_COPIES_CASE}: the provider read from ordinal 0, below its cut at 2"),
                "{profile}"
            );
            let rereads = run(HoldingFault::RereadsEveryNotice, false).unwrap_err();
            assert!(
                rereads.contains("re-read ordinal 0 again"),
                "{profile}: {rereads}"
            );
            // The first part an `opaque_reasoning` prune changes: the
            // signed reasoning where its signature is the opaque payload,
            // else the part holding the encrypted item.
            let first_opaque = match profile {
                ANTHROPIC_MESSAGES | OWNED_BROCA => "m1#0",
                _ => "m1#1",
            };
            assert_eq!(
                run(HoldingFault::IgnoresOpaquePrune, false).unwrap_err(),
                at(first_opaque),
                "{profile}"
            );
            let call = run(HoldingFault::EditsHeldToolOutput, true);
            let text = run(HoldingFault::EditsHeldStateText, true);
            if profile == ANTHROPIC_MESSAGES {
                // A tool_use block has no signature, so the fault cannot
                // happen: the report says so by name instead of passing.
                let call = call.unwrap();
                assert!(names(&call.parts_not_applicable).contains(&"thought_signature"));
                assert!(!call.parts_checked.contains(&"thought_signature"));
            } else {
                assert_eq!(call.unwrap_err(), at("m1#3"), "{profile}");
            }
            if matches!(profile, ANTHROPIC_MESSAGES | OWNED_BROCA) {
                // No served text carries a provider-state signature, so the
                // fault cannot happen: the report says so by name.
                let text = text.unwrap();
                assert!(names(&text.parts_not_applicable).contains(&"text_with_state"));
                assert!(!text.parts_checked.contains(&"text_with_state"));
            } else {
                assert_eq!(text.unwrap_err(), at("m1#4"), "{profile}");
            }
        }
    }

    #[test]
    fn a_profile_the_runner_contract_does_not_name_fails_the_probe_by_name() {
        let failure = held_copies_follow_reasoning_pruned(&mut HoldingProvider::on(
            "future-profile",
            None,
            false,
        ))
        .unwrap_err();
        assert_eq!(
            failure,
            format!(
                "{HELD_COPIES_CASE}: the adapter's serializer profile \"future-profile\" is not \
                 one the runner contract names (no archive-pruning rules for serializer profile: \
                 future-profile)"
            )
        );
    }

    /// The host's own id for a message body, where the profile's stored
    /// format carries one: OpenCode's `info.id` and a `pi` journal entry's
    /// `id`. The other profiles' bodies carry no message id, so the runner
    /// mints its own `mid`.
    fn native_message_id(profile: &str, message: &Value) -> Option<String> {
        match profile {
            OPENCODE_AISDK => message.pointer("/info/id"),
            PI => message.get("id"),
            _ => None,
        }
        .and_then(Value::as_str)
        .map(str::to_owned)
    }

    /// The tool-call ids `message` opens and the ids it answers, in
    /// `transcript`'s stored format. A call whose result rides in the same
    /// part (the generic transcript's `result`, an `opencode-aisdk` tool
    /// part in a final state) opens nothing.
    fn tool_arcs(transcript: Transcript, message: &Value) -> (Vec<String>, Vec<String>) {
        let id = |part: &Value, key: &str| part[key].as_str().unwrap_or_default().to_owned();
        let mut calls = Vec::new();
        let mut results = Vec::new();
        if transcript.profile() == Some(PI) && message["message"]["role"] == "toolResult" {
            results.push(id(&message["message"], "toolCallId"));
        }
        for part in transcript.parts(message) {
            match (transcript.profile(), part["type"].as_str()) {
                (None, Some("tool_call")) if part.get("result").is_none() => {
                    calls.push(id(&part, "id"));
                }
                (Some(OPENCODE_AISDK), Some("tool"))
                    if !matches!(
                        part["state"]["status"].as_str(),
                        Some("completed" | "error")
                    ) =>
                {
                    calls.push(id(&part, "callID"));
                }
                (Some(OWNED_BROCA), Some("tool_call")) => calls.push(id(&part, "tool_call_id")),
                (Some(OWNED_BROCA), Some("tool_result")) => {
                    results.push(id(&part, "tool_call_id"));
                }
                (Some(ANTHROPIC_MESSAGES), Some("tool_use")) => calls.push(id(&part, "id")),
                (Some(ANTHROPIC_MESSAGES), Some("tool_result")) => {
                    results.push(id(&part, "tool_use_id"));
                }
                (Some(PI), Some("toolCall")) => calls.push(id(&part, "id")),
                _ => {}
            }
        }
        (calls, results)
    }

    /// An `owned-broca` page serves parts as the runner's read surface does:
    /// a stored text-with-state part is served as plain `text`, because the
    /// provider state never leaves the runner, and a tool result's `output`
    /// is an object tagged on `kind`, never an array.
    fn served_owned_broca_shape_defects(
        transcript: Transcript,
        message: &Value,
        ordinal: u64,
        defects: &mut Vec<String>,
    ) {
        for part in transcript.parts(message) {
            if part["type"] == "text_with_state" {
                defects.push(format!(
                    "owned-broca m{ordinal}: a served text_with_state part"
                ));
            }
            if part["type"] == "tool_result" {
                let output = &part["output"];
                let tagged = matches!(
                    output["kind"].as_str(),
                    Some("text" | "json" | "parts" | "denied" | "error")
                );
                if output.is_array() || !tagged {
                    defects.push(format!(
                        "owned-broca m{ordinal}: tool_result output {output} is not tagged on kind"
                    ));
                }
            }
        }
    }

    /// A holding provider that also records, for every notice the probe
    /// sends, its `through_ordinal` and the messages served up to it.
    struct NoticeRecorder {
        inner: HoldingProvider,
        notices: Vec<(String, u64, Vec<Value>)>,
    }

    impl PrunedReasoningProbe for NoticeRecorder {
        fn serializer_profile(&self) -> Option<String> {
            self.inner.serializer_profile()
        }

        fn setup(
            &mut self,
            runner: &ScriptedRunner,
            request: &SetupRequest,
        ) -> Result<SetupAnswer, String> {
            self.inner.setup(runner, request)
        }

        fn notice(
            &mut self,
            runner: &ScriptedRunner,
            notice: &AdvanceNotice,
        ) -> Result<(), String> {
            // Read the runner's state directly, so recording adds no reads
            // the probe would count.
            let through = notice.through_ordinal;
            let served = runner.lock().served[..=through as usize].to_vec();
            self.notices
                .push((self.inner.session.clone(), through, served));
            self.inner.notice(runner, notice)
        }

        fn next_view(
            &mut self,
            runner: &ScriptedRunner,
            session_ref: &str,
        ) -> Result<CompactionView, String> {
            self.inner.next_view(runner, session_ref)
        }
    }

    /// The transcripts are ones a real runner could serve, so a provider
    /// that strictly decodes the stored formats gets as far as the prune:
    /// on a host profile every page's `mid` (and every `mid` naming that
    /// message elsewhere) is the host's own message id, an `owned-broca`
    /// page serves only the part shapes its read surface serves, and no
    /// notice the probe sends ends on an assistant message whose tool call
    /// has no result yet.
    #[test]
    fn probe_transcripts_keep_host_mids_and_notices_end_on_complete_tool_arcs() {
        let mut defects = Vec::new();
        for profile in PROFILES {
            let transcript = Transcript::for_profile(Some(profile)).unwrap();
            for mode in [PruneMode::Reasoning, PruneMode::Opaque] {
                let runner = ScriptedRunner::new(transcript, "probe-lineage", 6);
                let before = runner.read(&json!({})).unwrap();
                runner.prune_reasoning_through(6, mode);
                let after = runner.read(&json!({})).unwrap();
                for page in [before, after] {
                    for entry in page["messages"].as_array().unwrap() {
                        let native = native_message_id(profile, &entry["message"]);
                        if matches!(profile, OPENCODE_AISDK | PI) && native.is_none() {
                            defects.push(format!("{profile}: a host body without its id"));
                        }
                        let mid = entry["mid"].as_str().unwrap();
                        if native.as_deref().is_some_and(|native| native != mid) {
                            defects.push(format!(
                                "{profile}: page mid {mid} is not the host's id {native:?}"
                            ));
                        }
                        let ordinal = entry["ordinal"].as_u64().unwrap();
                        if profile == OWNED_BROCA {
                            served_owned_broca_shape_defects(
                                transcript,
                                &entry["message"],
                                ordinal,
                                &mut defects,
                            );
                        }
                        for listed in page["pruned_reasoning"].as_array().into_iter().flatten() {
                            let at = listed["part"].as_str().unwrap();
                            if at.starts_with(&format!("m{ordinal}#")) && listed["mid"] != mid {
                                defects.push(format!(
                                    "{profile}: pruned_reasoning names {} for {mid}",
                                    listed["mid"]
                                ));
                            }
                        }
                    }
                }
                runner.cut("probe-lineage-b", 2);
                let cut = &runner.head()["cut"]["mid"];
                let native = native_message_id(profile, &transcript.message(1));
                if native.is_some_and(|native| *cut != json!(native)) {
                    defects.push(format!(
                        "{profile}: head cut mid {cut} is not the host's id"
                    ));
                }
            }
        }
        let transcripts = std::iter::once(None).chain(PROFILES.map(Some));
        for profile in transcripts {
            let transcript = Transcript::for_profile(profile).unwrap();
            let mut recorder = NoticeRecorder {
                inner: HoldingProvider {
                    profile: profile.map(str::to_owned),
                    ..HoldingProvider::default()
                },
                notices: Vec::new(),
            };
            held_copies_follow_reasoning_pruned(&mut recorder)
                .unwrap_or_else(|e| panic!("{profile:?}: {e}"));
            assert!(!recorder.notices.is_empty(), "{profile:?}");
            for (session, through, served) in recorder.notices {
                let (calls, _) = tool_arcs(transcript, &served[through as usize]);
                let answered: Vec<String> = served
                    .iter()
                    .flat_map(|m| tool_arcs(transcript, m).1)
                    .collect();
                for call in calls.iter().filter(|c| !answered.contains(c)) {
                    defects.push(format!(
                        "{profile:?} {session}: a notice through {through} ends on call {call} \
                         with no result"
                    ));
                }
            }
        }
        assert!(defects.is_empty(), "{}", defects.join("\n"));
    }

    /// Every member of `served` exists in `original`, checked recursively
    /// through nested objects; array contents are not inspected.
    fn adds_nothing(original: &Value, served: &Value) -> bool {
        match (original, served) {
            (Value::Object(original), Value::Object(served)) => {
                served.iter().all(|(key, value)| {
                    original
                        .get(key)
                        .is_some_and(|before| adds_nothing(before, value))
                })
            }
            _ => true,
        }
    }

    /// Every profile transcript, before and after each prune mode, is one a
    /// runner of that profile would serve: the runner crate's own profile
    /// code accepts every entry, and each prune follows its per-profile
    /// rules. A fixture drifting from its profile fails here.
    #[test]
    fn every_profile_transcript_passes_the_runner_crates_own_profile_code() {
        use cortexkit_role_llm_runner::{
            archive_prunes_reasoning::{carries_pruned_content_in, kinds},
            display::{self, DisplayPart},
            read::ReadPage,
            serializer_profile::{strip_cache_control, top_level_part_count},
        };
        for profile in PROFILES {
            let transcript = Transcript::for_profile(Some(profile)).unwrap();
            let mut shown = std::collections::BTreeSet::new();
            for mode in [PruneMode::Reasoning, PruneMode::Opaque] {
                let runner = ScriptedRunner::new(transcript, "probe-lineage", 5);
                let before = runner.read(&json!({})).unwrap();
                runner.prune_reasoning_through(5, mode);
                let after = runner.read(&json!({})).unwrap();
                for page in [before, after] {
                    let page: ReadPage = serde_json::from_value(page)
                        .unwrap_or_else(|e| panic!("{profile}: page does not decode: {e}"));
                    for entry in &page.messages {
                        let m = &entry.message;
                        entry
                            .check_descriptor_rows(profile)
                            .unwrap_or_else(|e| panic!("{profile} {}: {e}", entry.mid));
                        assert_eq!(
                            top_level_part_count(profile, m),
                            Ok(transcript.parts(m).len()),
                            "{profile} {}",
                            entry.mid
                        );
                        if profile == ANTHROPIC_MESSAGES {
                            assert_eq!(&strip_cache_control(m), m, "{profile} {}", entry.mid);
                        }
                        if matches!(profile, ANTHROPIC_MESSAGES | OPENCODE_AISDK) {
                            let row = display::project(profile, m, None)
                                .unwrap_or_else(|e| panic!("{profile} {}: {e}", entry.mid));
                            for part in row.parts {
                                shown.insert(match part {
                                    DisplayPart::Text(_) => "text",
                                    DisplayPart::Reasoning(_) => "reasoning",
                                    DisplayPart::ToolCall(_) => "tool_call",
                                    DisplayPart::ToolResult(_) => "tool_result",
                                    _ => "other",
                                });
                            }
                        }
                    }
                }
                // Folding the served transcript restores nothing; folding
                // the originals restores the first pruned part.
                let pruned = runner.pruned();
                let all = |messages: &[Value]| Value::Array(messages.to_vec());
                let (originals, served) = {
                    let state = runner.lock();
                    (all(&state.originals), all(&state.served))
                };
                let restored = |messages: &Value| {
                    cortexkit_role_llm_runner::archive_prunes_reasoning::restored_part_in(
                        profile,
                        &pruned,
                        0..6,
                        messages,
                    )
                    .unwrap()
                    .map(|p| p.part.clone())
                };
                assert_eq!(restored(&served), None, "{profile} {mode:?}");
                assert_eq!(
                    restored(&originals),
                    Some(pruned[0].part.clone()),
                    "{profile}"
                );
            }
            if matches!(profile, ANTHROPIC_MESSAGES | OPENCODE_AISDK) {
                for kind in ["text", "reasoning", "tool_call", "tool_result"] {
                    assert!(shown.contains(kind), "{profile}: no {kind} part");
                }
            }
            for ordinal in [1, 3, 5] {
                for part in transcript.answer_parts(ordinal) {
                    let original = &part.original;
                    if part.pruned.is_empty() {
                        for kind in kinds::ALL {
                            assert_eq!(
                                carries_pruned_content_in(profile, kind, original),
                                Ok(false),
                                "{profile}: a kept part carries {kind}: {original}"
                            );
                        }
                    }
                    for (kind, served) in &part.pruned {
                        assert_eq!(
                            carries_pruned_content_in(profile, kind, original),
                            Ok(true),
                            "{profile} {kind}: {original}"
                        );
                        assert_eq!(
                            carries_pruned_content_in(profile, kind, served),
                            Ok(false),
                            "{profile} {kind}: {served}"
                        );
                        assert_eq!(original["type"], served["type"], "{profile} {kind}");
                        assert!(adds_nothing(original, served), "{profile} {kind}: {served}");
                    }
                }
            }
        }
    }
}
