//! What a step's reasoning costs when it is sent to the model again.
//!
//! A runner can count everything in a request except reasoning it holds only
//! in encrypted or signed form, and the cost of sending that reasoning again
//! is what a compaction provider needs to know before dropping it. Two numbers
//! stand in for it:
//!
//! - [`generated`]: what the step produced, from the step's own usage. This is
//!   available at once and is the first estimate. Oh My Pi's catalog records
//!   that current Claude lines bill replayed thinking at that size, and OpenAI
//!   bills replayed Responses reasoning at the turn's reported reasoning
//!   tokens, so the estimate is usually right.
//! - [`measured`]: the residual of two consecutive requests. When request N's
//!   whole prompt is a prefix of request N+1's, the provider's input count
//!   grows by what was appended between them. If the caller counts
//!   everything appended except the reasoning, the residual is the
//!   reasoning's cost.
//!
//! The arithmetic is only meaningful across two requests that sent the same
//! history with nothing changed in between: no earlier prompt content
//! rewritten or removed. Only the caller that rendered both requests can know
//! that, so it states it ([`Pair::same_prefix`]) and never infers it from
//! cache figures. Once reasoning is removed from later requests, its cost
//! stops mattering; a cost describes the part only while it is sent.

/// The step's usage, as the provider reported it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepUsage {
    /// All output tokens the step was billed for, reasoning included.
    pub output: u64,
    /// The reasoning tokens, when the provider reports them on their own
    /// (OpenAI, Gemini); `None` when it doesn't (Anthropic).
    pub reasoning: Option<u64>,
}

/// What a step's reasoning cost to generate. Returns the reported reasoning
/// tokens when the provider gives them, ignoring `visible`; otherwise
/// `output.saturating_sub(visible)`, where `visible` is the content-token
/// count of the step's visible output text and tool-call content, excluding
/// reasoning and any message framing.
pub fn generated(usage: StepUsage, visible: u64) -> u64 {
    usage
        .reasoning
        .unwrap_or_else(|| usage.output.saturating_sub(visible))
}

/// Two consecutive requests of one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    /// Total input tokens the provider reported for request N, cached and
    /// uncached together.
    pub earlier_input: u64,
    /// The same for request N+1.
    pub later_input: u64,
    /// The counted tokens of everything appended between them except the
    /// reasoning being measured: the step's text and tool calls, tool
    /// results, any new user message, and their message frames. A cost left
    /// out here stays in the residual and is wrongly read as reasoning.
    pub appended: u64,
    /// Whether request N's whole prompt is a byte prefix of request N+1's,
    /// with no earlier content rewritten or removed between them. The caller
    /// states this from its own renders.
    pub same_prefix: bool,
}

/// Why a pair gave no measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unmeasured {
    /// The two requests didn't share a prefix, so their difference includes
    /// edits, not just appended messages.
    PrefixChanged,
    /// The difference was negative: something counted as appended wasn't
    /// sent, or the count is wrong.
    Negative,
    /// The residual exceeds the supplied generation estimate, which this
    /// function uses as its acceptance ceiling; the pair or its accounting
    /// may be unsuitable.
    AboveGenerated,
}

/// The residual `later_input - earlier_input - appended`, accepted when the
/// caller asserts an unchanged prefix and the residual lies in
/// `0..=generated`. These checks are only arithmetic plausibility: the caller
/// must establish that the requests are consecutive, that their usage totals
/// are comparable, and that `appended` holds every other added cost,
/// framing included. On rejection, callers keep their previous estimate.
pub fn measured(pair: Pair, generated: u64) -> Result<u64, Unmeasured> {
    if !pair.same_prefix {
        return Err(Unmeasured::PrefixChanged);
    }
    let residual =
        i128::from(pair.later_input) - i128::from(pair.earlier_input) - i128::from(pair.appended);
    if residual < 0 {
        return Err(Unmeasured::Negative);
    }
    let residual = residual as u64;
    if residual > generated {
        return Err(Unmeasured::AboveGenerated);
    }
    Ok(residual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_prefers_the_reported_reasoning() {
        let openai = StepUsage {
            output: 900,
            reasoning: Some(700),
        };
        assert_eq!(generated(openai, 200), 700);
        let anthropic = StepUsage {
            output: 900,
            reasoning: None,
        };
        assert_eq!(generated(anthropic, 200), 700);
        assert_eq!(generated(anthropic, 2000), 0);
    }

    fn pair(earlier_input: u64, later_input: u64, appended: u64) -> Pair {
        Pair {
            earlier_input,
            later_input,
            appended,
            same_prefix: true,
        }
    }

    #[test]
    fn the_difference_is_the_reasoning() {
        // 10,000 tokens before; the step's text, tool call and result add 1,200
        // and its reasoning 650.
        assert_eq!(measured(pair(10_000, 11_850, 1_200), 700), Ok(650));
        // Reasoning the provider didn't replay costs nothing.
        assert_eq!(measured(pair(10_000, 11_200, 1_200), 700), Ok(0));
    }

    #[test]
    fn a_pair_across_a_change_or_out_of_bounds_measures_nothing() {
        let changed = Pair {
            same_prefix: false,
            ..pair(10_000, 11_850, 1_200)
        };
        assert_eq!(measured(changed, 700), Err(Unmeasured::PrefixChanged));
        assert_eq!(
            measured(pair(10_000, 11_000, 1_200), 700),
            Err(Unmeasured::Negative)
        );
        assert_eq!(
            measured(pair(10_000, 13_000, 1_200), 700),
            Err(Unmeasured::AboveGenerated)
        );
    }
}
