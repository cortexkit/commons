//! Which tokenizer a model reads text with.
//!
//! The mapping follows Oh My Pi's catalog (`model-tokenizer.ts`): Claude by
//! line and revision, OpenAI from GPT-5, and the open-weight families by name
//! rules. These are name-based rules, not a check of the provider's
//! tokenizer. Any model they don't recognize, including GPT-4o and older GPT
//! models, gets o200k as a stand-in with no guaranteed error bound, and the
//! choice says so ([`FamilyChoice::exact`] is false).

use crate::utok::Encoding;

/// A tokenizer family this crate can count with. Each is an offline
/// reconstruction of one provider tokenizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// OpenAI GPT-4o and GPT-5 onward.
    O200k,
    /// OpenAI GPT-3.5 and GPT-4.
    Cl100k,
    /// Claude 3 through Opus 4.6, and every Sonnet, Fable and Haiku before 5.
    ClaudeV3,
    /// Claude Opus 4.7 through 4.9.
    ClaudeV47,
    /// Claude Opus 5 onward.
    ClaudeV5,
    /// Claude Sonnet and Fable 5 onward, and Haiku 5.5 onward.
    ClaudeV5Sonnet,
    /// Qwen 3.5 onward.
    Qwen3,
    /// DeepSeek V3 through V4.
    DeepSeekV3,
    /// Kimi K2 onward.
    KimiK2,
    /// GLM-5 onward.
    Glm5,
}

impl Family {
    /// Every family, in a fixed order.
    pub const ALL: [Family; 10] = [
        Family::O200k,
        Family::Cl100k,
        Family::ClaudeV3,
        Family::ClaudeV47,
        Family::ClaudeV5,
        Family::ClaudeV5Sonnet,
        Family::Qwen3,
        Family::DeepSeekV3,
        Family::KimiK2,
        Family::Glm5,
    ];

    /// The family's stable name, as it appears in a `token_family`.
    pub fn name(self) -> &'static str {
        match self {
            Family::O200k => "o200k",
            Family::Cl100k => "cl100k",
            Family::ClaudeV3 => "claude-v3",
            Family::ClaudeV47 => "claude-v47",
            Family::ClaudeV5 => "claude-v5",
            Family::ClaudeV5Sonnet => "claude-v5-sonnet",
            Family::Qwen3 => "qwen3",
            Family::DeepSeekV3 => "deepseek-v3",
            Family::KimiK2 => "kimi-k2",
            Family::Glm5 => "glm5",
        }
    }

    /// The family a [`Family::name`] names.
    pub fn from_name(name: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|family| family.name() == name)
    }

    /// The family's counting revision, kept by hand. Maintainers must bump it
    /// whenever a change can alter any count for this family; an unrelated
    /// release of this crate leaves it alone, so stored counts stay usable.
    /// The sample counts pinned in `lib.rs` are a regression guard on one
    /// text per family, not proof that every input counts the same.
    pub fn revision(self) -> u32 {
        1
    }

    /// A compatibility label such as `claude-v5/1`: the family and its
    /// counting revision. Counts under different labels are not comparable.
    /// Callers store the label with their counts, and reject or recompute a
    /// count whose label differs from the one they expect; this crate stores
    /// nothing and invalidates nothing itself.
    pub fn token_family(self) -> String {
        format!("{}/{}", self.name(), self.revision())
    }

    /// The family and revision a [`Family::token_family`] label names. Any
    /// revision number parses; callers compare it with the revision they
    /// support.
    pub fn from_token_family(name: &str) -> Option<(Family, u32)> {
        let (family, revision) = name.split_once('/')?;
        Some((Family::from_name(family)?, revision.parse().ok()?))
    }

    /// Whether this binary holds the family's vocabulary. o200k is always
    /// built. The features are `cl100k`, `claude` (all four Claude families),
    /// `qwen`, `deepseek`, `kimi` and `glm`.
    pub fn is_built(self) -> bool {
        match self {
            Family::O200k => true,
            Family::Cl100k => cfg!(feature = "cl100k"),
            Family::ClaudeV3 | Family::ClaudeV47 | Family::ClaudeV5 | Family::ClaudeV5Sonnet => {
                cfg!(feature = "claude")
            }
            Family::Qwen3 => cfg!(feature = "qwen"),
            Family::DeepSeekV3 => cfg!(feature = "deepseek"),
            Family::KimiK2 => cfg!(feature = "kimi"),
            Family::Glm5 => cfg!(feature = "glm"),
        }
    }

    /// Tokens Claude's message frame adds to one user message, on top of its
    /// content; `None` for families whose frame was never measured.
    pub fn message_overhead(self) -> Option<u64> {
        match self {
            Family::ClaudeV3 => Some(7),
            Family::ClaudeV47 => Some(11),
            Family::ClaudeV5 | Family::ClaudeV5Sonnet => Some(6),
            _ => None,
        }
    }

    pub(crate) fn encoding(self) -> Encoding {
        match self {
            Family::O200k => Encoding::O200kBase,
            Family::Cl100k => Encoding::Cl100kBase,
            Family::ClaudeV3 => Encoding::ClaudeV3,
            Family::ClaudeV47 => Encoding::ClaudeV47,
            Family::ClaudeV5 => Encoding::ClaudeV5,
            Family::ClaudeV5Sonnet => Encoding::ClaudeV5Sonnet,
            Family::Qwen3 => Encoding::Qwen3,
            Family::DeepSeekV3 => Encoding::DeepSeekV3,
            Family::KimiK2 => Encoding::KimiK2,
            Family::Glm5 => Encoding::Glm5,
        }
    }
}

/// The family to count a model's text with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FamilyChoice {
    pub family: Family,
    /// True when the name rules map the model to its own line's tokenizer.
    /// False for the o200k stand-in, and for a known approximation (Haiku
    /// 5.5 onward, counted with Sonnet 5's vocabulary). It is not a check of
    /// the provider's tokenizer, and it says nothing about whether the
    /// family is built into this binary.
    pub exact: bool,
}

/// The family to count `model`'s text with, from catalog-style name rules on
/// the lowercased last path segment (`anthropic/claude-opus-5-5` reads
/// `claude-opus-5-5`). An unrecognized model, or an unsupported revision of a
/// known line, gets `{ family: O200k, exact: false }`. `Cl100k` is never
/// chosen here; a caller can still count with it explicitly.
pub fn family_for_model(model: &str) -> FamilyChoice {
    let id = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    match known_family(&id) {
        Some((family, exact)) => FamilyChoice { family, exact },
        None => FamilyChoice {
            family: Family::O200k,
            exact: false,
        },
    }
}

/// The recognized family and whether it is the model's own tokenizer.
fn known_family(id: &str) -> Option<(Family, bool)> {
    if let Some(choice) = claude_family(id) {
        return Some(choice);
    }
    let own = |family: Family| (family, true);
    if id.contains("deepseek") {
        return Some(own(Family::DeepSeekV3));
    }
    if id.contains("kimi") {
        return Some(own(Family::KimiK2));
    }
    if let Some(revision) = revision_after(id, "qwen") {
        return at_least(revision, (3, 5)).then_some(own(Family::Qwen3));
    }
    if let Some(revision) = revision_after(id, "glm") {
        return at_least(revision, (5, 0)).then_some(own(Family::Glm5));
    }
    if let Some(revision) = revision_after(id, "gpt") {
        return at_least(revision, (5, 0)).then_some(own(Family::O200k));
    }
    None
}

/// Claude's tokenizer by line and revision, as in Oh My Pi's catalog, and
/// whether it is that line's own tokenizer.
fn claude_family(id: &str) -> Option<(Family, bool)> {
    let lines = ["opus", "sonnet", "fable", "mythos", "haiku"];
    let line = lines.into_iter().find(|line| id.contains(line));
    if line.is_none() && !id.contains("claude") {
        return None;
    }
    // Current ids put the revision after the line (`claude-opus-5-5`); the
    // Claude 3 ids put it before (`claude-3-5-sonnet`), and those all count
    // with the v3 tokenizer.
    let revision = line.and_then(|line| revision_after(id, line));
    let choice = match (line, revision) {
        (Some("opus"), Some(revision)) if at_least(revision, (5, 0)) => (Family::ClaudeV5, true),
        (Some("opus"), Some(revision)) if at_least(revision, (4, 7)) => (Family::ClaudeV47, true),
        (Some("sonnet" | "fable" | "mythos"), Some(revision)) if at_least(revision, (5, 0)) => {
            (Family::ClaudeV5Sonnet, true)
        }
        // Haiku 5.5 moved to the post-4.7 tokenizer. No Haiku vocabulary is
        // measured; Sonnet 5's is the nearest, so this is an approximation.
        (Some("haiku"), Some(revision)) if at_least(revision, (5, 5)) => {
            (Family::ClaudeV5Sonnet, false)
        }
        // A Claude id naming no known line can't be placed by revision.
        (None, _) => (Family::ClaudeV3, false),
        _ => (Family::ClaudeV3, true),
    };
    Some(choice)
}

/// The `major.minor` revision written right after `name` in `id`: `5-5` and
/// `5.5` both read as (5, 5), a lone `5` as (5, 0). A date (`20250929`) or any
/// number of more than two digits is not a revision.
fn revision_after(id: &str, name: &str) -> Option<(u32, u32)> {
    let start = id.find(name)? + name.len();
    let rest = id[start..].trim_start_matches(['-', '_', '.', ' ']);
    let mut parts = rest.split(|c: char| !c.is_ascii_digit());
    let major = parts.next().filter(|part| (1..=2).contains(&part.len()))?;
    let after_major = &rest[major.len()..];
    let minor = match after_major.chars().next() {
        Some('-' | '.') => after_major[1..]
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .filter(|part| (1..=2).contains(&part.len()))
            .and_then(|part| part.parse().ok())
            .unwrap_or(0),
        _ => 0,
    };
    Some((major.parse().ok()?, minor))
}

fn at_least(revision: (u32, u32), floor: (u32, u32)) -> bool {
    revision >= floor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(model: &str) -> (Family, bool) {
        let choice = family_for_model(model);
        (choice.family, choice.exact)
    }

    #[test]
    fn claude_lines_and_revisions() {
        for (model, expected) in [
            ("anthropic/claude-opus-5-5", Family::ClaudeV5),
            ("claude-opus-5", Family::ClaudeV5),
            ("claude-opus-4-7", Family::ClaudeV47),
            ("claude-opus-4-9-20261001", Family::ClaudeV47),
            ("claude-opus-4-6", Family::ClaudeV3),
            ("claude-sonnet-5-5", Family::ClaudeV5Sonnet),
            ("claude-fable-5-1", Family::ClaudeV5Sonnet),
            ("claude-sonnet-4-5-20250929", Family::ClaudeV3),
            ("claude-haiku-4-5", Family::ClaudeV3),
            ("claude-3-5-sonnet-20241022", Family::ClaudeV3),
        ] {
            assert_eq!(family(model), (expected, true), "{model}");
        }
    }

    #[test]
    fn approximations_are_not_exact() {
        assert_eq!(family("claude-haiku-5-5"), (Family::ClaudeV5Sonnet, false));
        assert_eq!(family("claude-instant-1"), (Family::ClaudeV3, false));
    }

    #[test]
    fn other_known_families() {
        for (model, expected) in [
            ("openai/gpt-6.1-sol", Family::O200k),
            ("gpt-5.5", Family::O200k),
            ("gpt-6-luna", Family::O200k),
            ("deepseek/deepseek-flash", Family::DeepSeekV3),
            ("deepseek-v4-pro", Family::DeepSeekV3),
            ("qwen3.6-coder", Family::Qwen3),
            ("kimi-k2.5", Family::KimiK2),
            ("glm-5.1", Family::Glm5),
        ] {
            assert_eq!(family(model), (expected, true), "{model}");
        }
    }

    #[test]
    fn unknown_or_older_models_count_with_o200k_marked_inexact() {
        for model in [
            "gemini-3-pro",
            "gpt-4o",
            "qwen3-coder",
            "glm-4.6",
            "grok-4.7",
            "",
        ] {
            assert_eq!(family(model), (Family::O200k, false), "{model}");
        }
    }

    #[test]
    fn names_round_trip() {
        for family in Family::ALL {
            assert_eq!(Family::from_name(family.name()), Some(family));
            assert_eq!(
                Family::from_token_family(&family.token_family()),
                Some((family, family.revision()))
            );
        }
        assert_eq!(Family::from_name("claude"), None);
        assert_eq!(Family::from_token_family("claude-v5"), None);
        assert_eq!(Family::from_token_family("claude-v5/x"), None);
    }
}
