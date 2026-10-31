//! Per-run token usage (`run_usage`, contract §6.4): what the provider
//! reported for a run's model steps, on `run.result` and `run.status`.
//!
//! Every total is optional and absent until some step reports it, so an
//! absent total means "not known", never zero. A present total is a lower
//! bound whenever `unreported_steps > 0` or some reported steps omitted that
//! field.

use serde::{Deserialize, Serialize};

/// What one completed model step's provider reported. A field the provider
/// did not report stays `None`; a step with no field at all is unreported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StepUsage {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

impl StepUsage {
    /// A step whose provider reported no usage field at all.
    pub fn unreported() -> Self {
        Self::default()
    }

    pub fn with_input_tokens(mut self, tokens: u64) -> Self {
        self.input_tokens = Some(tokens);
        self
    }

    pub fn with_cached_input_tokens(mut self, tokens: u64) -> Self {
        self.cached_input_tokens = Some(tokens);
        self
    }

    pub fn with_cache_write_tokens(mut self, tokens: u64) -> Self {
        self.cache_write_tokens = Some(tokens);
        self
    }

    pub fn with_output_tokens(mut self, tokens: u64) -> Self {
        self.output_tokens = Some(tokens);
        self
    }

    pub fn with_reasoning_tokens(mut self, tokens: u64) -> Self {
        self.reasoning_tokens = Some(tokens);
        self
    }

    /// Whether the provider reported any usage field for the step, even a
    /// zero one. A step that reported nothing counts in `unreported_steps`.
    pub fn is_reported(&self) -> bool {
        self.totals().iter().any(Option::is_some)
    }

    fn totals(&self) -> [Option<u64>; 5] {
        [
            self.input_tokens,
            self.cached_input_tokens,
            self.cache_write_tokens,
            self.output_tokens,
            self.reasoning_tokens,
        ]
    }
}

/// Add `step` to `total`: a total stays absent until a step reports it.
fn add(total: &mut Option<u64>, step: Option<u64>) {
    if let Some(step) = step {
        *total = Some(total.unwrap_or(0).saturating_add(step));
    }
}

/// The `usage` member of `run.result` and `run.status`: the run's step
/// counts and token totals, and the same per model in `by_model`. Decoded
/// leniently.
///
/// Non-exhaustive so later optional members are additive: build one with
/// [`RunUsage::new`] and [`RunUsage::record_step`], or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunUsage {
    /// Completed model steps in the run.
    pub steps: u64,
    /// Steps whose provider reported no usage field at all.
    pub unreported_steps: u64,
    /// The sum over the steps that reported it; absent until one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// One entry per distinct (provider, model, variant), in order of first
    /// use. Decoded as empty when absent, which a zero-step usage may be;
    /// always encoded.
    #[serde(default)]
    pub by_model: Vec<ModelUsage>,
    /// The run's fixed request level, omitted for standard. Producers fill it
    /// from ServiceTier::as_str; readers retain unknown record levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_applied: Option<String>,
    /// Present only when all steps that reported a served tier agreed.
    /// Never inferred from the applied level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_served: Option<String>,
}

/// Durable session spend on `session.head`, across runs, rewinds and cuts.
/// The runner updates it atomically with each step's usage, never by scanning
/// history. Imported history that was never counted is excluded explicitly by
/// `since`; restarts retain both the totals and their starting run.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SessionUsage {
    /// The first counted run's `run_id`, or the first run after adoption when
    /// the runner did not count the imported history.
    pub since: String,
    /// The same fields and absence rules as `run_usage`, flattened on the wire.
    #[serde(flatten)]
    pub totals: RunUsage,
}

/// Largest usage count exactly representable by readers using JSON doubles.
pub const MAX_EXACT_USAGE_COUNT: u64 = (1 << 53) - 1;

impl SessionUsage {
    pub fn new(since: impl Into<String>) -> Self {
        Self {
            since: since.into(),
            totals: RunUsage::new(),
        }
    }

    /// Count a step in the same durable commit that stores its run usage.
    pub fn record_step(
        &mut self,
        provider: &str,
        model: &str,
        variant: Option<&str>,
        step: StepUsage,
    ) {
        self.totals.record_step(provider, model, variant, step);
    }

    pub fn consistent(&self) -> bool {
        let bounded = |steps, unreported_steps, tokens: [Option<u64>; 5]| {
            steps <= MAX_EXACT_USAGE_COUNT
                && unreported_steps <= MAX_EXACT_USAGE_COUNT
                && tokens
                    .into_iter()
                    .flatten()
                    .all(|n| n <= MAX_EXACT_USAGE_COUNT)
        };
        self.totals.consistent()
            && bounded(
                self.totals.steps,
                self.totals.unreported_steps,
                self.totals.totals(),
            )
            && self
                .totals
                .by_model
                .iter()
                .all(|m| bounded(m.steps, m.unreported_steps, m.totals()))
    }
}

/// One `by_model` entry: the steps and totals of one (provider, model,
/// variant) selection within the run.
///
/// Non-exhaustive: build one with [`ModelUsage::new`], or let
/// [`RunUsage::record_step`] build it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ModelUsage {
    pub provider: String,
    pub model: String,
    /// Absent when the selection names no variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub steps: u64,
    pub unreported_steps: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

impl ModelUsage {
    /// An entry with no steps yet and every total absent.
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            variant: None,
            steps: 0,
            unreported_steps: 0,
            input_tokens: None,
            cached_input_tokens: None,
            cache_write_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
        }
    }

    pub fn with_variant(mut self, variant: impl Into<String>) -> Self {
        self.variant = Some(variant.into());
        self
    }

    /// Count one completed step of this selection.
    pub fn record_step(&mut self, step: StepUsage) {
        self.steps = self.steps.saturating_add(1);
        if !step.is_reported() {
            self.unreported_steps = self.unreported_steps.saturating_add(1);
        }
        add(&mut self.input_tokens, step.input_tokens);
        add(&mut self.cached_input_tokens, step.cached_input_tokens);
        add(&mut self.cache_write_tokens, step.cache_write_tokens);
        add(&mut self.output_tokens, step.output_tokens);
        add(&mut self.reasoning_tokens, step.reasoning_tokens);
    }

    fn totals(&self) -> [Option<u64>; 5] {
        [
            self.input_tokens,
            self.cached_input_tokens,
            self.cache_write_tokens,
            self.output_tokens,
            self.reasoning_tokens,
        ]
    }
}

impl RunUsage {
    /// Producer-side tier validation; readers keep unknown record levels.
    pub fn check_service_tier(&self) -> Result<(), crate::service_tier::TierProblem> {
        crate::service_tier::check_applied(self.service_tier_applied.as_deref())
    }
    /// Record tokens and one step's tier metadata. Durable tier state keeps a
    /// constant number of tier strings, independent of the number of steps,
    /// and examines only the new step. Persist `tiers` alongside token totals. Missing
    /// served values do not vote; disagreement remains absent after a restart.
    pub fn record_step_with_service_tier(
        &mut self,
        provider: &str,
        model: &str,
        variant: Option<&str>,
        step: StepUsage,
        tier: &crate::service_tier::ServiceTierUsage,
        tiers: &mut crate::service_tier::RunServiceTierUsage,
    ) -> Result<(), &'static str> {
        if tiers.steps() != self.steps {
            return Err("step_tiers_incomplete");
        }
        tier.check().map_err(|problem| problem.reason())?;
        if self.steps > 0
            && self.service_tier_applied.as_deref().unwrap_or("standard")
                != tier.service_tier_applied.as_deref().unwrap_or("standard")
        {
            return Err("run_tier_fixed");
        }
        tiers
            .record_step(tier)
            .map_err(|problem| problem.reason())?;
        self.record_step(provider, model, variant, step);
        let metadata = tiers.usage();
        self.service_tier_applied = metadata.service_tier_applied;
        self.service_tier_served = metadata.service_tier_served;
        Ok(())
    }
    /// The usage of a run with no completed step:
    /// `{"steps":0,"unreported_steps":0,"by_model":[]}`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one completed step served by `provider`/`model`/`variant`,
    /// adding it to the run's totals and to that selection's `by_model`
    /// entry, which is appended on the selection's first use.
    pub fn record_step(
        &mut self,
        provider: &str,
        model: &str,
        variant: Option<&str>,
        step: StepUsage,
    ) {
        self.steps = self.steps.saturating_add(1);
        if !step.is_reported() {
            self.unreported_steps = self.unreported_steps.saturating_add(1);
        }
        add(&mut self.input_tokens, step.input_tokens);
        add(&mut self.cached_input_tokens, step.cached_input_tokens);
        add(&mut self.cache_write_tokens, step.cache_write_tokens);
        add(&mut self.output_tokens, step.output_tokens);
        add(&mut self.reasoning_tokens, step.reasoning_tokens);
        let index = match self.by_model.iter().position(|entry| {
            entry.provider == provider
                && entry.model == model
                && entry.variant.as_deref() == variant
        }) {
            Some(index) => index,
            None => {
                let mut entry = ModelUsage::new(provider, model);
                entry.variant = variant.map(str::to_owned);
                self.by_model.push(entry);
                self.by_model.len() - 1
            }
        };
        self.by_model[index].record_step(step);
    }

    fn totals(&self) -> [Option<u64>; 5] {
        [
            self.input_tokens,
            self.cached_input_tokens,
            self.cache_write_tokens,
            self.output_tokens,
            self.reasoning_tokens,
        ]
    }

    /// Whether the usage agrees with itself as §6.4 requires: no more
    /// unreported steps than steps, at most one `by_model` entry per
    /// (provider, model, variant), every entry with at least one step, the
    /// entries' step counts and totals summing to the top-level ones (a
    /// total absent from every entry is absent at the top), and every total
    /// absent wherever every step was unreported. Saying nothing about the
    /// values a provider reported, it cannot tell an absent total written as
    /// `0` from a reported zero; a consumer treats an inconsistent usage as
    /// malformed.
    pub fn consistent(&self) -> bool {
        if self.unreported_steps > self.steps {
            return false;
        }
        if self.unreported_steps == self.steps && self.totals().iter().any(Option::is_some) {
            return false;
        }
        let mut steps = 0u64;
        let mut unreported = 0u64;
        let mut sums: [Option<u64>; 5] = [None; 5];
        for (index, entry) in self.by_model.iter().enumerate() {
            if entry.steps == 0 || entry.unreported_steps > entry.steps {
                return false;
            }
            if entry.unreported_steps == entry.steps && entry.totals().iter().any(Option::is_some) {
                return false;
            }
            let duplicate = self.by_model[..index].iter().any(|earlier| {
                earlier.provider == entry.provider
                    && earlier.model == entry.model
                    && earlier.variant == entry.variant
            });
            if duplicate {
                return false;
            }
            steps = steps.saturating_add(entry.steps);
            unreported = unreported.saturating_add(entry.unreported_steps);
            for (sum, total) in sums.iter_mut().zip(entry.totals()) {
                add(sum, total);
            }
        }
        steps == self.steps && unreported == self.unreported_steps && sums == self.totals()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::{run::RunStatus, vectors};

    #[test]
    fn session_usage_counts_must_be_exact_json_integers() {
        let vector: Value =
            serde_json::from_str(vectors::file("head-session-usage-bounds.json").unwrap()).unwrap();
        for (name, valid) in [("at_limit", true), ("above_limit", false)] {
            let head: crate::read::HeadMeta = serde_json::from_value(vector[name].clone()).unwrap();
            let usage = head.usage.unwrap();
            assert!(
                usage.totals.consistent(),
                "bound vectors isolate the numeric limit"
            );
            assert_eq!(usage.consistent(), valid, "{name}");
            assert_eq!(serde_json::to_value(&usage).unwrap(), vector[name]["usage"]);
        }
        for field in [
            "steps",
            "unreported_steps",
            "input_tokens",
            "cached_input_tokens",
            "cache_write_tokens",
            "output_tokens",
            "reasoning_tokens",
        ] {
            for count in [MAX_EXACT_USAGE_COUNT, MAX_EXACT_USAGE_COUNT + 1] {
                let mut usage = serde_json::json!({
                    "since": "run-first", "steps": 1, "unreported_steps": 0,
                    "by_model": [{"provider": "p", "model": "m", "steps": 1, "unreported_steps": 0}]
                });
                usage[field] = serde_json::json!(count);
                usage["by_model"][0][field] = serde_json::json!(count);
                if field == "unreported_steps" {
                    usage["steps"] = serde_json::json!(count);
                    usage["by_model"][0]["steps"] = serde_json::json!(count);
                }
                let usage: SessionUsage = serde_json::from_value(usage).unwrap();
                assert!(usage.totals.consistent());
                assert_eq!(
                    usage.consistent(),
                    count == MAX_EXACT_USAGE_COUNT,
                    "{field}: {count}"
                );
            }
            for value in [serde_json::json!(-1), serde_json::json!(1.5)] {
                let mut usage = serde_json::to_value(SessionUsage::new("run-first")).unwrap();
                usage[field] = value.clone();
                assert!(
                    serde_json::from_value::<SessionUsage>(usage).is_err(),
                    "{field}: {value}"
                );
            }
        }
    }

    #[test]
    fn session_usage_group_extends_head() {
        assert_eq!(
            crate::capabilities::group_ops(crate::capabilities::SESSION_USAGE),
            Some(&[crate::ops::SESSION_HEAD][..])
        );
    }

    #[test]
    fn session_head_optional_members_round_trip() {
        let cases: Value =
            serde_json::from_str(vectors::file("head-session-usage.json").unwrap()).unwrap();
        for case in cases["answers"].as_array().unwrap() {
            let head: crate::read::HeadMeta =
                serde_json::from_value(case["answer"].clone()).unwrap();
            assert!(head.lineage_consistent(), "{}", case["name"]);
            assert_eq!(
                serde_json::to_value(&head).unwrap(),
                case["answer"],
                "{}",
                case["name"]
            );
            if let Some(usage) = head.usage {
                assert!(usage.consistent());
            }
        }
    }

    #[test]
    fn session_usage_unknown_fields_stay_absent() {
        let mut usage = SessionUsage::new("run-first");
        usage.record_step("p", "m", None, StepUsage::unreported().with_input_tokens(4));
        let wire = serde_json::to_value(&usage).unwrap();
        for field in [
            "cached_input_tokens",
            "cache_write_tokens",
            "output_tokens",
            "reasoning_tokens",
        ] {
            assert!(
                wire.get(field).is_none(),
                "unknown {field} must be absent: {wire}"
            );
        }
        assert_eq!(wire["since"], "run-first");
        assert!(usage.consistent());
    }

    #[test]
    fn session_usage_requires_since() {
        let missing = serde_json::json!({"steps": 0, "unreported_steps": 0, "by_model": []});
        assert!(serde_json::from_value::<SessionUsage>(missing).is_err());
        let wire = serde_json::to_value(SessionUsage::new("run-first")).unwrap();
        assert_eq!(wire["since"], "run-first");
    }

    /// The `run.status` vector `name`: its text, and the byte
    /// range of its `usage` object.
    fn usage_span(name: &str) -> (&'static str, std::ops::Range<usize>) {
        let text = vectors::file(name).unwrap_or_else(|| panic!("{name} is not listed in FILES"));
        let marker = "\"usage\":";
        let start = text.find(marker).unwrap() + marker.len();
        let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
        stream.next().unwrap().unwrap();
        (text, start..start + stream.byte_offset())
    }

    /// Decode the vector's `usage` and check that encoding it gives back
    /// its bytes exactly, key order included, and that the whole body
    /// decodes as `RunStatus` carrying that usage and re-encodes to the body
    /// without the runner members the role does not type.
    fn decode_vector(name: &str) -> RunUsage {
        let (text, span) = usage_span(name);
        let usage: RunUsage = serde_json::from_str(&text[span.clone()])
            .unwrap_or_else(|e| panic!("{name}: usage does not decode: {e}"));
        assert_eq!(
            serde_json::to_string(&usage).unwrap(),
            &text[span],
            "{name}: usage bytes differ"
        );
        let body: Value = serde_json::from_str(text).unwrap();
        let status: RunStatus = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(status.usage.as_ref(), Some(&usage), "{name}");
        let mut typed = body;
        let members = typed.as_object_mut().unwrap();
        members.remove("final_step_finish_reason").unwrap();
        members.remove("retries_used").unwrap();
        assert_eq!(serde_json::to_value(&status).unwrap(), typed, "{name}");
        let again: RunStatus =
            serde_json::from_value(serde_json::to_value(&status).unwrap()).unwrap();
        assert_eq!(again, status, "{name}: second decode differs");
        usage
    }

    fn totals_of(usage: &RunUsage) -> [Option<u64>; 5] {
        usage.totals()
    }

    /// The entries' counts and totals, summed field by field here rather
    /// than through `consistent`, equal the top-level ones; a total absent
    /// from every entry is absent at the top.
    fn assert_by_model_sums(name: &str, usage: &RunUsage) {
        let steps: u64 = usage.by_model.iter().map(|m| m.steps).sum();
        let unreported: u64 = usage.by_model.iter().map(|m| m.unreported_steps).sum();
        assert_eq!(steps, usage.steps, "{name}: steps");
        assert_eq!(
            unreported, usage.unreported_steps,
            "{name}: unreported_steps"
        );
        for field in 0..5 {
            let entries: Vec<Option<u64>> =
                usage.by_model.iter().map(|m| m.totals()[field]).collect();
            let expected = if entries.iter().all(Option::is_none) {
                None
            } else {
                Some(entries.iter().map(|total| total.unwrap_or(0)).sum::<u64>())
            };
            assert_eq!(totals_of(usage)[field], expected, "{name}: total {field}");
        }
        assert!(usage.consistent(), "{name}: inconsistent");
    }

    fn reported(input: u64, cached: u64, write: u64, output: u64) -> StepUsage {
        StepUsage::default()
            .with_input_tokens(input)
            .with_cached_input_tokens(cached)
            .with_cache_write_tokens(write)
            .with_output_tokens(output)
    }

    #[test]
    fn usage_plain_round_trips_and_leaves_reasoning_absent() {
        let name = "run_status.usage_plain.json";
        let usage = decode_vector(name);
        assert_by_model_sums(name, &usage);
        assert_eq!((usage.steps, usage.unreported_steps), (3, 0));
        assert_eq!(
            totals_of(&usage),
            [Some(1200), Some(48000), Some(3000), Some(660), None]
        );
        assert_eq!(usage.by_model.len(), 1);
        assert_eq!(usage.by_model[0].variant, None);
        assert_eq!(usage.by_model[0].reasoning_tokens, None);
        let mut built = RunUsage::new();
        for _ in 0..3 {
            built.record_step("stub", "stub-model", None, reported(400, 16000, 1000, 220));
        }
        assert_eq!(built, usage);
    }

    #[test]
    fn usage_disjoint_input_round_trips_without_overlap() {
        let name = "run_status.usage_disjoint_input.json";
        let usage = decode_vector(name);
        assert_by_model_sums(name, &usage);
        assert_eq!((usage.steps, usage.unreported_steps), (1, 0));
        assert_eq!(
            totals_of(&usage),
            [Some(1250), Some(6000), Some(750), Some(200), None]
        );
        assert_eq!(usage.by_model.len(), 1);
        assert_eq!(
            usage.input_tokens.unwrap()
                + usage.cached_input_tokens.unwrap()
                + usage.cache_write_tokens.unwrap(),
            8000
        );
        let mut built = RunUsage::new();
        built.record_step("stub", "stub-model", None, reported(1250, 6000, 750, 200));
        assert_eq!(built, usage);
    }

    #[test]
    fn usage_unreported_step_counts_it_and_keeps_totals_as_reported() {
        let name = "run_status.usage_unreported_step.json";
        let usage = decode_vector(name);
        assert_by_model_sums(name, &usage);
        assert_eq!((usage.steps, usage.unreported_steps), (3, 1));
        assert_eq!(
            totals_of(&usage),
            [Some(400), Some(16000), Some(1000), Some(220), None]
        );
        assert_eq!(
            (usage.by_model[0].steps, usage.by_model[0].unreported_steps),
            (3, 1)
        );
        // Broca's run: the first step reported every count, the second
        // nothing, and the third only `output_tokens: 0`, which still makes
        // it a reported step.
        let mut built = RunUsage::new();
        built.record_step("stub", "stub-model", None, reported(400, 16000, 1000, 220));
        built.record_step("stub", "stub-model", None, StepUsage::unreported());
        built.record_step(
            "stub",
            "stub-model",
            None,
            StepUsage::default().with_output_tokens(0),
        );
        assert_eq!(built, usage);
    }

    #[test]
    fn usage_model_switch_keeps_one_entry_per_selection_in_first_use_order() {
        let name = "run_status.usage_model_switch.json";
        let usage = decode_vector(name);
        assert_by_model_sums(name, &usage);
        let selections: Vec<(&str, &str, Option<&str>, u64)> = usage
            .by_model
            .iter()
            .map(|m| {
                (
                    m.provider.as_str(),
                    m.model.as_str(),
                    m.variant.as_deref(),
                    m.steps,
                )
            })
            .collect();
        assert_eq!(
            selections,
            [
                ("stub", "stub-model", None, 1),
                ("stub", "other-model", Some("high"), 2)
            ]
        );
        let mut built = RunUsage::new();
        built.record_step("stub", "stub-model", None, reported(400, 16000, 1000, 220));
        for _ in 0..2 {
            built.record_step(
                "stub",
                "other-model",
                Some("high"),
                reported(400, 16000, 1000, 220),
            );
        }
        assert_eq!(built, usage);
    }

    #[test]
    fn usage_reasoning_reports_reasoning_where_present() {
        let name = "run_status.usage_reasoning.json";
        let usage = decode_vector(name);
        assert_by_model_sums(name, &usage);
        assert_eq!(usage.reasoning_tokens, Some(200));
        assert_eq!(usage.by_model[0].reasoning_tokens, Some(200));
        // Reasoning reported on only some steps: the total is the sum over
        // those, a lower bound for the run.
        let mut built = RunUsage::new();
        built.record_step(
            "stub",
            "stub-model",
            None,
            reported(400, 16000, 1000, 220).with_reasoning_tokens(200),
        );
        for _ in 0..2 {
            built.record_step("stub", "stub-model", None, reported(400, 16000, 1000, 220));
        }
        assert_eq!(built, usage);
    }

    #[test]
    fn zero_step_usage_decodes_without_by_model_and_encodes_it_empty() {
        let usage: RunUsage = serde_json::from_str(r#"{"steps":0,"unreported_steps":0}"#).unwrap();
        assert_eq!(usage, RunUsage::new());
        assert!(usage.by_model.is_empty());
        assert_eq!(totals_of(&usage), [None; 5]);
        assert!(usage.consistent());
        assert_eq!(
            serde_json::to_string(&usage).unwrap(),
            r#"{"steps":0,"unreported_steps":0,"by_model":[]}"#
        );
    }

    #[test]
    fn absent_usage_stays_absent_on_status_and_result() {
        let status: RunStatus = serde_json::from_str(r#"{"state":"active"}"#).unwrap();
        assert_eq!(status.usage, None);
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            r#"{"state":"active"}"#
        );
        let result: crate::run::RunResult =
            serde_json::from_str(r#"{"run_id":"r1","state":"active"}"#).unwrap();
        assert_eq!(result.usage, None);
        let result = result.with_usage(RunUsage::new());
        assert_eq!(
            serde_json::to_value(&result).unwrap()["usage"],
            serde_json::json!({"steps":0,"unreported_steps":0,"by_model":[]})
        );
    }

    #[test]
    fn a_step_reporting_only_a_zero_is_reported() {
        let mut usage = RunUsage::new();
        usage.record_step("p", "m", None, StepUsage::default().with_output_tokens(0));
        usage.record_step("p", "m", None, StepUsage::unreported());
        assert_eq!((usage.steps, usage.unreported_steps), (2, 1));
        assert_eq!(totals_of(&usage), [None, None, None, Some(0), None]);
        assert!(usage.consistent());
    }

    #[test]
    fn inconsistent_usage_is_detected() {
        let mut base = RunUsage::new();
        base.record_step("p", "m", None, reported(1, 2, 3, 4));
        base.record_step("p", "n", Some("high"), reported(5, 6, 7, 8));
        assert!(base.consistent());
        type Damage = fn(&mut RunUsage);
        let broken: Vec<(&str, Damage)> = vec![
            ("top total off by one", |u| u.input_tokens = Some(7)),
            ("total present at top only", |u| {
                u.reasoning_tokens = Some(0)
            }),
            ("entry total missing", |u| {
                u.by_model[1].output_tokens = None
            }),
            ("step count differs", |u| u.steps = 3),
            ("unreported above steps", |u| {
                u.by_model[0].unreported_steps = 2
            }),
            ("duplicate selection", |u| {
                u.by_model[1].model = "m".into();
                u.by_model[1].variant = None;
            }),
            ("entry without a step", |u| {
                u.by_model.push(ModelUsage::new("p", "z"));
            }),
            ("totals on an all-unreported run", |u| {
                *u = RunUsage::new();
                u.record_step("p", "m", None, StepUsage::unreported());
                u.input_tokens = Some(0);
                u.by_model[0].input_tokens = Some(0);
            }),
        ];
        for (what, damage) in broken {
            let mut usage = base.clone();
            damage(&mut usage);
            assert!(!usage.consistent(), "{what}");
        }
    }
}
