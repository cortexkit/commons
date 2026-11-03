//! An estimate of what one image costs the model that reads it.
//!
//! Providers bill an image by its pixel size, each with its own formula. The
//! four formulas ([`Rule`]) and the per-model numbers are ported from Oh My
//! Pi's model catalog (`compat/image-tokenization.ts` and the
//! `image-tokenization` rules beside it), which records which ones were
//! checked against real billing. [`rule_for`] uses a recognized model's rule
//! when it has one. Otherwise it estimates with GPT-5.5-style sizing for
//! [`Api::OpenAi`], a fixed 1,120 tokens for [`Api::Google`], and Claude's
//! high-resolution sizing for [`Api::Other`]. These fallbacks are
//! assumptions, not billing rules or upper bounds: an OpenAI-style rule can
//! bill a large image far above Claude's cap.

use crate::family::{Family, family_for_model};

/// An image's pixel size, as the runner holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// OpenAI's `detail` level on an image; `None` is the provider default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    Low,
    High,
    Original,
}

/// One OpenAI `detail` level's sizing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatchSizing {
    /// The longest side after the image is fitted.
    pub max_edge: u32,
    /// The patch budget it is resized to fit, if any.
    pub patch_budget: Option<u32>,
}

/// A model line's image billing rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rule {
    /// OpenAI: 32px patches times a per-model multiplier, sized per `detail`.
    OpenAiPatch {
        multiplier: f64,
        low: PatchSizing,
        high: PatchSizing,
        original: PatchSizing,
        /// The level an omitted `detail` (or `auto`) sizes like.
        auto_original: bool,
    },
    /// Anthropic: 28px patches after the API's resize to fit an edge limit
    /// and a patch budget.
    AnthropicPatch { max_edge: u32, max_tokens: u32 },
    /// DeepSeek: ViT patches merged `downsample`² into one token, plus a
    /// newline token per row and two delimiters.
    DeepSeekPatch {
        patch_size: u32,
        downsample: u32,
        max_tokens: u32,
        min_pixels: u32,
    },
    /// A fixed budget per image whatever its size (Gemini 3).
    Fixed { tokens: u32 },
}

/// The wire API a request travels on, as far as image billing tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    OpenAi,
    Google,
    /// Anthropic Messages, and every other API.
    Other,
}

const fn sizing(max_edge: u32, patch_budget: Option<u32>) -> PatchSizing {
    PatchSizing {
        max_edge,
        patch_budget,
    }
}

/// GPT-5.5's rule, used as the estimate for an OpenAI-compatible API when
/// the model has no rule of its own.
const OPENAI_WIRE: Rule = Rule::OpenAiPatch {
    multiplier: 1.2,
    low: sizing(512, None),
    high: sizing(2048, Some(2500)),
    original: sizing(6000, Some(10000)),
    auto_original: true,
};
/// GPT-5.6 onward (checked against billing through 6.1).
const OPENAI_5_6: Rule = Rule::OpenAiPatch {
    multiplier: 1.2,
    low: sizing(512, None),
    high: sizing(2048, Some(2500)),
    original: sizing(65535, None),
    auto_original: true,
};
const OPENAI_5_4: Rule = Rule::OpenAiPatch {
    multiplier: 1.2,
    low: sizing(2048, Some(6144)),
    high: sizing(2048, Some(2500)),
    original: sizing(6000, Some(10000)),
    auto_original: false,
};
const OPENAI_5_2: Rule = Rule::OpenAiPatch {
    multiplier: 1.2,
    low: sizing(2048, Some(6144)),
    high: sizing(2048, Some(6144)),
    original: sizing(2048, Some(6144)),
    auto_original: false,
};
/// gpt-6-astra's limits differ from the rest of its revision.
const OPENAI_ASTRA: Rule = Rule::OpenAiPatch {
    multiplier: 1.2,
    low: sizing(512, None),
    high: sizing(65535, Some(2500)),
    original: sizing(65535, None),
    auto_original: true,
};
/// Claude from Opus 4.7 on, and the estimate for any other API when the
/// model has no rule of its own.
const CLAUDE_HIGH: Rule = Rule::AnthropicPatch {
    max_edge: 2576,
    max_tokens: 4784,
};
const CLAUDE_STANDARD: Rule = Rule::AnthropicPatch {
    max_edge: 1568,
    max_tokens: 1568,
};
const GEMINI: Rule = Rule::Fixed { tokens: 1120 };
const DEEPSEEK_FLASH_4_1: Rule = Rule::DeepSeekPatch {
    patch_size: 14,
    downsample: 3,
    max_tokens: 1024,
    min_pixels: 295_936,
};

/// The image rule to estimate with for `model` read over `api`: the model's
/// own rule when it is recognized, else `api`'s fallback (see the module
/// docs).
pub fn rule_for(model: &str, api: Api) -> Rule {
    let id = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    model_rule(&id).unwrap_or(match api {
        Api::OpenAi => OPENAI_WIRE,
        Api::Google => GEMINI,
        Api::Other => CLAUDE_HIGH,
    })
}

fn model_rule(id: &str) -> Option<Rule> {
    // The image tier follows the Claude line, whether or not its text
    // tokenizer is an approximation (Haiku 5.5 counts with Sonnet's).
    match family_for_model(id).family {
        // Claude lines before 4.7 bill the standard tier.
        Family::ClaudeV3 => return Some(CLAUDE_STANDARD),
        Family::ClaudeV47 | Family::ClaudeV5 | Family::ClaudeV5Sonnet => return Some(CLAUDE_HIGH),
        _ => {}
    }
    if id.contains("gpt-6-astra") {
        return Some(OPENAI_ASTRA);
    }
    if let Some(revision) = revision(id, "gpt") {
        return match revision {
            (5, 2..=3) => Some(OPENAI_5_2),
            (5, 4) => Some(OPENAI_5_4),
            (5, 5) => Some(OPENAI_WIRE),
            (major, minor) if (major, minor) >= (5, 6) && major < 10 => Some(OPENAI_5_6),
            _ => None,
        };
    }
    if let Some(revision) = revision(id, "gemini") {
        return (revision.0 >= 3).then_some(GEMINI);
    }
    if id.contains("deepseek") && id.contains("flash") && id.contains("4.1") {
        return Some(DEEPSEEK_FLASH_4_1);
    }
    None
}

fn revision(id: &str, name: &str) -> Option<(u32, u32)> {
    let start = id.find(name)? + name.len();
    let rest = id[start..].trim_start_matches(['-', '_', '.']);
    let mut numbers = rest
        .split(|c: char| !c.is_ascii_digit())
        .take(2)
        .map(|part| part.parse::<u32>().ok());
    let major = numbers.next()??;
    let minor = numbers.next().flatten().unwrap_or(0);
    Some((major, minor))
}

/// Estimated input tokens for one image of `size` under `rule`. Only the
/// OpenAI rule reads `detail`.
pub fn tokens(rule: Rule, size: Size, detail: Option<Detail>) -> u64 {
    match rule {
        Rule::Fixed { tokens } => u64::from(tokens),
        Rule::OpenAiPatch {
            multiplier,
            low,
            high,
            original,
            auto_original,
        } => {
            let sizing = match detail {
                Some(Detail::Low) => low,
                Some(Detail::High) => high,
                Some(Detail::Original) => original,
                None if auto_original => original,
                None => high,
            };
            (openai_patches(size, sizing) as f64 * multiplier).ceil() as u64
        }
        Rule::AnthropicPatch {
            max_edge,
            max_tokens,
        } => anthropic_tokens(size, max_edge, max_tokens),
        Rule::DeepSeekPatch {
            patch_size,
            downsample,
            max_tokens,
            min_pixels,
        } => deepseek_tokens(size, patch_size, downsample, max_tokens, min_pixels),
    }
}

/// Scale `size` down (never up) so its longest side is at most `max_edge`.
fn fit_long_edge(size: Size, max_edge: u32) -> (f64, f64) {
    let (width, height) = (f64::from(size.width), f64::from(size.height));
    let longest = width.max(height);
    if longest <= f64::from(max_edge) {
        return (width, height);
    }
    let scale = f64::from(max_edge) / longest;
    (
        (width * scale).round().max(1.0),
        (height * scale).round().max(1.0),
    )
}

const OPENAI_PATCH_PX: f64 = 32.0;

/// <https://developers.openai.com/api/docs/guides/images-vision#patch-based-image-tokenization>
fn openai_patches(size: Size, sizing: PatchSizing) -> u64 {
    let (width, height) = fit_long_edge(size, sizing.max_edge);
    let patches = (width / OPENAI_PATCH_PX).ceil() * (height / OPENAI_PATCH_PX).ceil();
    let Some(budget) = sizing.patch_budget.map(f64::from) else {
        return patches as u64;
    };
    if patches <= budget {
        return patches as u64;
    }
    let shrink = ((OPENAI_PATCH_PX * OPENAI_PATCH_PX * budget) / (width * height)).sqrt();
    let scaled_w = width * shrink / OPENAI_PATCH_PX;
    let scaled_h = height * shrink / OPENAI_PATCH_PX;
    let adjusted = shrink * (scaled_w.floor() / scaled_w).min(scaled_h.floor() / scaled_h);
    let resized_w = (width * adjusted).floor();
    let resized_h = (height * adjusted).floor();
    if resized_w <= 0.0 || resized_h <= 0.0 {
        return budget as u64;
    }
    budget.min((resized_w / OPENAI_PATCH_PX).ceil() * (resized_h / OPENAI_PATCH_PX).ceil()) as u64
}

const ANTHROPIC_PATCH_PX: u64 = 28;

/// Round half to even, like the API's resize and Python's `round`.
fn round_ties_to_even(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor != 0.5 {
        return value.round();
    }
    if floor % 2.0 == 0.0 {
        floor
    } else {
        floor + 1.0
    }
}

/// Anthropic visual tokens after the API's resize: the largest
/// aspect-preserving size whose 28px-padded edges fit `max_edge` and whose
/// patch count fits `max_tokens`, found by bisecting the long edge, as the
/// reference implementation does:
/// <https://platform.claude.com/docs/en/build-with-claude/vision-coordinates#resize-your-image-before-uploading>
fn anthropic_tokens(size: Size, max_edge: u32, max_tokens: u32) -> u64 {
    let long = u64::from(size.width.max(size.height)).max(1);
    let short_side = u64::from(size.width.min(size.height)).max(1);
    let aspect = long as f64 / short_side as f64;
    let patches = |l: u64, s: u64| l.div_ceil(ANTHROPIC_PATCH_PX) * s.div_ceil(ANTHROPIC_PATCH_PX);
    let fits = |l: u64, s: u64| {
        l.div_ceil(ANTHROPIC_PATCH_PX) * ANTHROPIC_PATCH_PX <= u64::from(max_edge)
            && s.div_ceil(ANTHROPIC_PATCH_PX) * ANTHROPIC_PATCH_PX <= u64::from(max_edge)
            && patches(l, s) <= u64::from(max_tokens)
    };
    let short = |l: u64| (round_ties_to_even(l as f64 / aspect) as u64).max(1);
    if fits(long, short_side) {
        return patches(long, short_side);
    }
    let (mut lo, mut hi) = (1, long);
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if fits(mid, short(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    patches(lo, short(lo))
}

/// DeepSeek visual tokens: the image is upscaled to `min_pixels`, padded to
/// whole patches, and when its token grid exceeds `max_tokens` shrunk to the
/// largest aspect-preserving grid that fits. Port of `plan_image_grid` in the
/// reference processor:
/// <https://huggingface.co/deepseek-ai/DeepSeek-V4.1-Flash/blob/main/inference/image_processor.py>
fn deepseek_tokens(
    size: Size,
    patch: u32,
    downsample: u32,
    max_tokens: u32,
    min_pixels: u32,
) -> u64 {
    let (patch, downsample) = (f64::from(patch), f64::from(downsample));
    let max_tokens = f64::from(max_tokens);
    let cell = patch * downsample;
    let span = |height: f64, width: f64| {
        let rows = ((height / patch).floor() / downsample).ceil();
        let cols = ((width / patch).floor() / downsample).ceil();
        rows * (cols + 1.0) + 2.0
    };
    let (mut width, mut height) = (f64::from(size.width.max(1)), f64::from(size.height.max(1)));
    if width * height < f64::from(min_pixels) {
        let scale = (f64::from(min_pixels) / (width * height)).sqrt();
        width = (width * scale).trunc();
        height = (height * scale).trunc();
    }
    let tokens = span(
        (height / patch).ceil() * patch,
        (width / patch).ceil() * patch,
    );
    if tokens <= max_tokens {
        return tokens as u64;
    }
    let aspect = height / width;
    let max_cols = ((max_tokens - 2.0) / aspect + 0.25).sqrt() - 0.5;
    let max_rows = max_cols * aspect;
    if max_cols < 1.0 {
        return span(((max_tokens - 2.0) / 2.0).floor() * cell, cell) as u64;
    }
    if max_rows < 1.0 {
        return span(cell, (max_tokens - 3.0) * cell) as u64;
    }
    let beta = (max_cols.floor() * cell / width).min(max_rows.floor() * cell / height);
    span(
        ((height * beta) / patch).floor() * patch,
        ((width * beta) / patch).floor() * patch,
    ) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    #[test]
    fn claude_counts_patches_and_resizes_past_its_limits() {
        // 1000x1000 fits the high tier as is: 36 x 36 patches of 28px.
        assert_eq!(tokens(CLAUDE_HIGH, size(1000, 1000), None), 36 * 36);
        // A large image is shrunk until its patches fit the budget.
        let big = tokens(CLAUDE_HIGH, size(8000, 6000), None);
        assert!(big <= 4784 && big > 4000, "{big}");
        assert!(tokens(CLAUDE_STANDARD, size(8000, 6000), None) <= 1568);
    }

    #[test]
    fn openai_counts_patches_with_its_multiplier_per_detail() {
        // 1024x1024 at `original`: 32 x 32 patches, times 1.2.
        assert_eq!(
            tokens(OPENAI_5_6, size(1024, 1024), None),
            (1024.0_f64 * 1.2).ceil() as u64
        );
        // `low` fits the long edge to 512 first: 16 x 16 patches.
        assert_eq!(
            tokens(OPENAI_5_6, size(1024, 1024), Some(Detail::Low)),
            (256.0_f64 * 1.2).ceil() as u64
        );
        // `high` holds a 2,500-patch budget.
        assert!(
            tokens(OPENAI_5_6, size(4000, 4000), Some(Detail::High))
                <= (2500.0_f64 * 1.2).ceil() as u64
        );
    }

    #[test]
    fn fixed_and_deepseek_rules() {
        assert_eq!(tokens(GEMINI, size(10, 10), None), 1120);
        let small = tokens(DEEPSEEK_FLASH_4_1, size(256, 256), None);
        let large = tokens(DEEPSEEK_FLASH_4_1, size(4000, 4000), None);
        assert!(
            small > 0 && large <= 1024 && large >= small,
            "{small} {large}"
        );
    }

    #[test]
    fn rules_by_model_and_api() {
        assert_eq!(
            rule_for("anthropic/claude-opus-5-5", Api::Other),
            CLAUDE_HIGH
        );
        assert_eq!(rule_for("claude-sonnet-4-5", Api::Other), CLAUDE_STANDARD);
        assert_eq!(rule_for("openai/gpt-6.1-sol", Api::OpenAi), OPENAI_5_6);
        assert_eq!(rule_for("gpt-6-astra", Api::OpenAi), OPENAI_ASTRA);
        assert_eq!(rule_for("gpt-5.4", Api::OpenAi), OPENAI_5_4);
        assert_eq!(rule_for("gemini-3-pro", Api::Google), GEMINI);
        // A model without a rule is estimated with its API's fallback, and
        // an unknown API with Claude's high tier.
        assert_eq!(rule_for("mystery-model", Api::OpenAi), OPENAI_WIRE);
        assert_eq!(rule_for("mystery-model", Api::Google), GEMINI);
        assert_eq!(rule_for("mystery-model", Api::Other), CLAUDE_HIGH);
    }
}
