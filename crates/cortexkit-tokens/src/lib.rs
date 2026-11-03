//! Token counts that every CortexKit module computes the same way.
//!
//! In CortexKit, a *runner* is the module that renders each model request and
//! holds every message part's full content. A *compaction provider* is a
//! module that rewrites a session's history (dropping or summarizing old
//! parts) and sees only references and sizes. The runner counts each part
//! once with this crate and sends the counts; the provider counts the text it
//! writes itself with the same crate. Both sides then use the same per-part
//! content-cost convention; how a whole request is framed and re-rendered
//! remains the caller's responsibility.
//!
//! - [`count`]: content tokens of a text for a tokenizer [`Family`], with no
//!   message frame. Each family is an offline reconstruction of one provider
//!   tokenizer, and no correction ratio is applied on top.
//! - [`family_for_model`]: which family to count a model's text with, from
//!   name rules. An unrecognized model gets o200k as a stand-in, marked as
//!   not exact.
//! - [`image`]: an estimate of what one image costs the model that reads it.
//! - [`reasoning`]: what a step's reasoning costs when it is sent again, which
//!   no tokenizer can see when the reasoning is encrypted.
//!
//! Counts are comparable only under the same [`Family::token_family`] label,
//! which names the family and its counting revision. Callers store the label
//! next to their counts.
//!
//! Each family's vocabulary is behind a cargo feature (all on by default), so
//! a runner can build only the families it serves; [`count`] returns `None`
//! for a family that isn't built in. o200k is always built.

// The tokenizer is upstream code kept whole, so updating it stays a copy.
// This crate exposes only counts, so upstream's encode paths and a few
// helpers are unused here.
#[allow(dead_code, unused_imports)]
mod utok;

pub mod family;
pub mod image;
pub mod reasoning;

pub use family::{Family, FamilyChoice, family_for_model};

/// Content tokens of `text` for `family`: the text alone, with no special
/// tokens and no message frame around it. `None` when `family`'s vocabulary
/// isn't built into this binary (see [`Family::is_built`]).
///
/// For the Claude families the count is a reconstructed single-user-message
/// count minus its fixed message overhead, so the generation's boundary and
/// whitespace behaviour remains (for example, empty text costs one token
/// under v3 and v4.7 and none under v5). It is not a boundary-independent
/// tokenization of arbitrary fragments.
pub fn count(family: Family, text: &str) -> Option<u64> {
    family
        .is_built()
        .then(|| u64::from(family.encoding().count(text)))
}

#[cfg(all(test, not(feature = "claude")))]
mod without_a_family {
    use super::{Family, count};

    #[test]
    fn a_family_left_out_of_the_build_has_no_count() {
        assert!(!Family::ClaudeV5.is_built());
        assert_eq!(count(Family::ClaudeV5, "hello"), None);
        assert!(count(Family::O200k, "hello").is_some());
    }
}

#[cfg(test)]
mod revision_pins {
    //! Sample counts per family under its current counting revision. A
    //! change that moves any of these numbers changes what a stored count
    //! means, so it must bump that family's `Family::revision` before the pin
    //! here is updated; callers then see a new label and recompute. One text
    //! per family is a regression guard, not proof that every input counts
    //! the same.

    use super::{Family, count};

    const SAMPLE: &str = "fn main() {\n    println!(\"Héllo, wörld — 你好, 1234567 tokens!\");\n}\n\nThe QUICK brown fox's tail isn't here.";

    /// Families left out of the build are skipped; CI runs this with
    /// `--all-features` so every pin is checked.
    #[test]
    fn sample_counts_are_pinned_per_revision() {
        let pins: Vec<(String, u64)> = Family::ALL
            .into_iter()
            .filter_map(|family| Some((family.token_family(), count(family, SAMPLE)?)))
            .collect();
        let expected: Vec<(String, u64)> = [
            ("o200k/1", 34),
            ("cl100k/1", 36),
            ("claude-v3/1", 42),
            ("claude-v47/1", 53),
            ("claude-v5/1", 53),
            ("claude-v5-sonnet/1", 53),
            ("qwen3/1", 42),
            ("deepseek-v3/1", 36),
            ("kimi-k2/1", 35),
            ("glm5/1", 36),
        ]
        .into_iter()
        .filter(|(name, _)| {
            Family::from_token_family(name).is_some_and(|(family, _)| family.is_built())
        })
        .map(|(name, count)| (name.to_owned(), count))
        .collect();
        assert_eq!(pins, expected);
    }
}
