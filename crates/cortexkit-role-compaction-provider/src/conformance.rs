//! Focused probes for single provider rules, not a full conformance kit.
//!
//! Enable the non-default `conformance` feature to drive these rules against
//! a provider through an implementation-specific adapter:
//! [`conversation_state_ignores_project_root`] for conversation identity and
//! [`held_copies_follow_reasoning_pruned`] for held copies after an archive
//! prune.

use cortexkit_role_llm_runner::archive_prunes_reasoning::{kinds, PrunedPartAt, ReasoningPruned};
use serde_json::{json, Value};

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

#[derive(Debug, Default)]
struct RunnerState {
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

fn message_id(ordinal: u64) -> String {
    format!("probe-m{ordinal}")
}

/// Message `ordinal` of the probe's transcript: a user prompt at even
/// ordinals, an assistant answer at odd ones. The answer carries signed
/// reasoning (part 0, pruned with the `reasoning` kind), plain text, one
/// part holding both a tool call with its thought signature and that call's
/// result (part 2), and `text_with_state` whose provider state holds a
/// signature (part 3); parts 2 and 3 are pruned with the `signature` kind.
/// A provider that edits the tool output or the text from a held pre-prune
/// copy re-serializes the signature the prune removed.
fn scripted_message(ordinal: u64) -> Value {
    if ordinal.is_multiple_of(2) {
        json!({"role":"user","content":[{"type":"text","text":format!("Question {ordinal}.")}]})
    } else {
        json!({"role":"assistant","content":[
            {"type":"reasoning","text":format!("Private thought {ordinal}."),"signature":format!("invented-signature-{ordinal}")},
            {"type":"text","text":format!("Answer {ordinal}.")},
            {"type":"tool_call","id":format!("call-{ordinal}"),"name":"open_door","arguments":format!("{{\"door\": {ordinal}}}"),
             "thought_signature":format!("invented-call-signature-{ordinal}"),
             "result":{"output":format!("Door {ordinal} is open, and here is a long listing of what is behind it.")}},
            {"type":"text_with_state","text":format!("Noted door {ordinal}, with a long explanation."),
             "provider_state":{"signature":format!("invented-state-signature-{ordinal}"),"format":"example"}}
        ]})
    }
}

impl ScriptedRunner {
    fn new(lineage_id: &str, through: u64) -> Self {
        let messages: Vec<Value> = (0..=through).map(scripted_message).collect();
        Self {
            state: std::sync::Mutex::new(RunnerState {
                lineage_id: lineage_id.into(),
                originals: messages.clone(),
                served: messages,
                ..RunnerState::default()
            }),
        }
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
            match (0..state.served.len() as u64).find(|o| message_id(*o) == mid) {
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
            let parts = original["content"].as_array().cloned().unwrap_or_default();
            messages.push(json!({
                "ordinal": ordinal,
                "mid": message_id(ordinal),
                "message": state.served[ordinal as usize],
                "digest": content_digest(original),
                "part_digests": parts.iter().enumerate().map(|(index, part)| json!({
                    "part": part_key(ordinal, index),
                    "digest": content_digest(part),
                })).collect::<Vec<_>>(),
            }));
            for pruned in state.pruned.iter().filter(|p| p.ordinal == ordinal) {
                let index: usize = pruned.part.rsplit('#').next().unwrap().parse().unwrap();
                let original_part = &parts[index];
                listed.push(json!({
                    "mid": message_id(ordinal),
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
                "mid": message_id(cut),
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
            let message = scripted_message(state.served.len() as u64);
            state.originals.push(message.clone());
            state.served.push(message);
        }
    }

    fn cut(&self, lineage_id: &str, first_ordinal: u64) {
        let mut state = self.lock();
        state.lineage_id = lineage_id.into();
        state.first_ordinal = first_ordinal;
    }

    /// Prune every assistant message at or below `through`, as a runner
    /// declaring the `reasoning` and `signature` kinds does: empty the
    /// reasoning, drop the call's thought signature, and empty the signature
    /// inside the provider state. Raise the generation in the same step.
    fn prune_reasoning_through(&self, through: u64) -> ReasoningPruned {
        let mut state = self.lock();
        for ordinal in (1..=through).step_by(2) {
            let key = part_key(ordinal, 0);
            if state
                .pruned
                .iter()
                .any(|p| p.ordinal == ordinal && p.part == key)
            {
                continue;
            }
            let content = &mut state.served[ordinal as usize]["content"];
            content[0] = json!({"type":"reasoning","text":""});
            if let Some(call) = content[2].as_object_mut() {
                call.remove("thought_signature");
            }
            content[3]["provider_state"]["signature"] = json!("");
            state
                .pruned
                .push(PrunedPartAt::new(ordinal, key, kinds::REASONING));
            for index in [2, 3] {
                state.pruned.push(PrunedPartAt::new(
                    ordinal,
                    part_key(ordinal, index),
                    kinds::SIGNATURE,
                ));
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
        if let Some(part) = view.restored_pruned_part(&pruned, first) {
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

    /// A notice repeating the reconciled generation re-reads nothing held.
    fn check_no_second_reconcile(&mut self, through: u64) -> Result<(), String> {
        let lineage = self.runner.lock().lineage_id.clone();
        let last = self.runner.lock().served.len() as u64 - 1;
        let pruned = self.runner.lock().reasoning_pruned;
        self.notice(&lineage, last, None, pruned)?;
        self.check_view()?;
        if let Some(ordinal) = self.runner.take_reads().iter().find(|o| **o <= through) {
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
/// later notice with the same generation causes
/// no second re-read. In the second, a cut first moves the conversation to a
/// lineage starting at ordinal 2, so the provider holds copies from before
/// its cut; after the prune it must re-read only from the cut and must not
/// render those older copies' pruned content.
pub fn held_copies_follow_reasoning_pruned(
    adapter: &mut impl PrunedReasoningProbe,
) -> Result<(), String> {
    for (label, cut) in [("no-cut", false), ("cut", true)] {
        let session = format!("probe-pruned-{label}");
        let session_ref =
            cortexkit_role_llm_runner::session_ref(&cortexkit_role_llm_runner::SessionIdentity {
                runner: PROBE_RUNNER,
                harness: PROBE_HARNESS,
                session: &session,
            });
        let mut conversation = Conversation {
            adapter: &mut *adapter,
            runner: ScriptedRunner::new("probe-lineage-a", 3),
            session,
            session_ref,
            seq: 0,
            setups: 0,
        };
        conversation.setup("probe-lineage-a", None, None)?;
        conversation.notice("probe-lineage-a", 3, None, None)?;
        let mut through = 3;
        if cut {
            // A cut after ordinal 1: the new lineage starts at 2, and the
            // provider keeps its copies of 0 and 1 from before it.
            conversation.runner.append_through(5);
            conversation.runner.cut("probe-lineage-b", 2);
            let descent = DescendsFrom {
                lineage_id: "probe-lineage-a".into(),
                through_ordinal: 1,
            };
            conversation.setup("probe-lineage-b", Some(descent), None)?;
            let digest = conversation.runner.digest_at(1);
            conversation.notice(
                "probe-lineage-b",
                5,
                Some(NoticeDescent::new("probe-lineage-a", 1, digest)),
                None,
            )?;
            through = 5;
        }
        conversation.runner.take_reads();
        let pruned = conversation.runner.prune_reasoning_through(through);
        let lineage = conversation.runner.lock().lineage_id.clone();
        conversation.setup(&lineage, None, Some(pruned))?;
        conversation.check_view()?;
        conversation.check_reconciled(through)?;
        conversation.check_no_second_reconcile(through)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        /// The one conversation this provider holds state for; a Setup for
        /// another starts from nothing, as a per-session store would.
        session: String,
        held: BTreeMap<u64, Value>,
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

        /// Part edits that trim each held answer's tool output and its
        /// state-carrying text, re-serializing the rest of each part.
        fn trim_edits(&self) -> Vec<crate::view::PartEdit> {
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
                let mut call = source(HoldingFault::EditsHeldToolOutput)["content"][2].clone();
                call["result"]["output"] = json!("[output trimmed]");
                let mut text = source(HoldingFault::EditsHeldStateText)["content"][3].clone();
                text["text"] = json!("[text trimmed]");
                for (index, part) in [(2, call), (3, text)] {
                    edits.push(crate::view::PartEdit::replace(
                        *ordinal,
                        format!("probe-m{ordinal}"),
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
            if self.fault == Some(HoldingFault::IgnoresGeneration) {
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
        fn setup(
            &mut self,
            runner: &ScriptedRunner,
            request: &SetupRequest,
        ) -> Result<SetupAnswer, String> {
            if request.session != self.session {
                *self = Self {
                    fault: self.fault,
                    edits_mode: self.edits_mode,
                    session: request.session.clone(),
                    ..Self::default()
                };
            }
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
                    mid: format!("probe-m{last}"),
                },
                ranges,
            ))
        }
    }

    #[test]
    fn reconciling_provider_passes_the_held_copies_probe() {
        held_copies_follow_reasoning_pruned(&mut HoldingProvider::default()).unwrap();
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
        assert!(failure.contains("re-read ordinal 0 again"), "{failure}");
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
}
