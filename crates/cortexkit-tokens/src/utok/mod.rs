//! Universal offline tokenizer.
//!
//! Exact token counting (and, where meaningful, encoding) for six model
//! families, with all vocabulary data zstd-compressed and embedded in the
//! binary. No network, no files, no external tokenizer runtimes.
//!
//! | [`Encoding`] | family | mechanism |
//! |---|---|---|
//! | `O200kBase` | GPT-4o / o1 / GPT-5+ | byte-level BPE (tiktoken ranks) |
//! | `Cl100kBase` | GPT-3.5 / GPT-4 | byte-level BPE |
//! | `ClaudeV3` / `ClaudeV47` / `ClaudeV5` / `ClaudeV5Sonnet` | Claude generations | ctok count reconstruction (count-only) |
//! | `Qwen3` | Qwen 3.5 / 3.6 / 3.8 (248k vocab) | byte-level BPE + NFC |
//! | `DeepSeekV3` | `DeepSeek` V3 … V4 | byte-level BPE, 3-stage split chain |
//! | `KimiK2` | Kimi K2 … K3 | byte-level BPE (tiktoken ranks) |
//! | `Glm5` | GLM-5.x (exact), GLM-4.x (near-exact) | byte-level BPE, `ignore_merges` |
//!
//! Semantics are `encode_ordinary`: plain content, no special tokens, no
//! chat-template frame. This matches budget-estimation use where fragments
//! are summed.
//!
//! Input is encoding-generic ([`Utf`]): `&str`/`String` (UTF-8), `&[u16]`
//! (UTF-16, e.g. a JS string over napi) and `&[u32]` (UTF-32) tokenize
//! natively in their own code units — no UTF-8 transcode, no scratch
//! buffer. The pre-tokenizer scans a codepoint [`Cursor`], and rank tables
//! lazily expand per-flavor lookup views (`bpe.rs`). Valid text yields
//! flavor-invariant ids/counts; malformed units decode permissively as
//! U+FFFD (matching a lossy JS crossing).

mod bpe;
// cortexkit-tokens: each family's data is behind a cargo feature, so a
// runner can build only the families it serves; upstream's Jev family is
// removed (its vocabulary was measured from a live API and no model maps to
// it here).
#[cfg(feature = "claude")]
mod claude;
mod pretoken;
mod scan;
mod tables;
// cortexkit-tokens: stable stand-in for upstream's `xutf` (see the module).
mod ucd;
mod utf;
// cortexkit-tokens: pure-Rust stand-in for upstream's `zstd` (see the module).
mod zst;
pub use self::{
    bpe::RankTable,
    utf::{Cursor, Unit, Utf},
};

/// A tokenizer family. Copy-cheap; all state is in lazily-initialized
/// process-wide tables (first use pays one zstd decode of ~0.5–1 MB).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Encoding {
    /// GPT-4o / o1 / GPT-5 (`OpenAI` default).
    O200kBase,
    /// GPT-3.5 / GPT-4 / older `OpenAI`.
    Cl100kBase,
    /// Claude 3 through Opus 4.6 (and every non-opus Claude < 5).
    ClaudeV3,
    /// Claude Opus 4.7–4.9.
    ClaudeV47,
    /// Claude Opus 5+.
    ClaudeV5,
    /// Claude Sonnet/Fable 5+ (non-opus 5-series frame variant).
    ClaudeV5Sonnet,
    /// Qwen 3.5 / 3.6 / 3.8 (248,044-token vocabulary, NFC input).
    Qwen3,
    /// `DeepSeek` V3 / V3.1 / V3.2 / R1 / V4 (identical base BPE).
    DeepSeekV3,
    /// Kimi K2 / K2.5 / K3 (163,584-token base vocabulary).
    KimiK2,
    /// GLM-5 (154,820-token vocabulary; ID-preserving superset of GLM-4.x).
    Glm5,
}

impl Encoding {
    /// Exact content token count of `text` (no specials, no message frame),
    /// in any UTF flavor.
    pub fn count<T: Utf + ?Sized>(self, text: &T) -> u32 {
        // cortexkit-tokens: the Claude arms are gated with their module; a
        // family whose feature is off never reaches here (`crate::count`
        // checks first).
        match self {
            #[cfg(feature = "claude")]
            Self::ClaudeV3 => claude::content_token_count(text.units(), claude::Family::V3),
            #[cfg(feature = "claude")]
            Self::ClaudeV47 => claude::content_token_count(text.units(), claude::Family::V47),
            #[cfg(feature = "claude")]
            Self::ClaudeV5 => claude::content_token_count(text.units(), claude::Family::V5),
            #[cfg(feature = "claude")]
            Self::ClaudeV5Sonnet => {
                claude::content_token_count(text.units(), claude::Family::V5Sonnet)
            }
            _ => tables::bpe_for(self).count(text.units()),
        }
    }

    /// Token ids for `text`, in any UTF flavor.
    ///
    /// `None` for the Claude families: they are reconstructed from counts,
    /// not boundaries, so no id sequence exists.
    pub fn encode<T: Utf + ?Sized>(self, text: &T) -> Option<Vec<u32>> {
        match self {
            Self::ClaudeV3 | Self::ClaudeV47 | Self::ClaudeV5 | Self::ClaudeV5Sonnet => None,
            _ => Some(tables::bpe_for(self).encode(text.units())),
        }
    }
}

// cortexkit-tokens: each family's tests run only when its data is built in.
// The OpenAI module tests o200k and cl100k together, so it needs `cl100k`.
#[cfg(test)]
#[path = "tests"]
mod tests {
    #[cfg(feature = "claude")]
    #[path = "claude.rs"]
    mod claude;
    #[cfg(feature = "deepseek")]
    #[path = "deepseek.rs"]
    mod deepseek;
    #[cfg(feature = "glm")]
    #[path = "glm.rs"]
    mod glm;
    #[cfg(feature = "kimi")]
    #[path = "kimi.rs"]
    mod kimi;
    #[cfg(feature = "cl100k")]
    #[path = "openai.rs"]
    mod openai;
    #[cfg(feature = "qwen")]
    #[path = "qwen.rs"]
    mod qwen;
    #[path = "rank_table.rs"]
    mod rank_table;
}
