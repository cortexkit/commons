use std::collections::BTreeMap;

use crate::{describe::Kind, CheckError, CheckResult, MESSAGE_MAX};

open_enum!(BiasUnit { Token => "token", Phrase => "phrase" });
wire_struct!(BiasLimit {
    unit: BiasUnit,
    max: u64
});
wire_struct!(
    #[serde(deny_unknown_fields)]
    BiasTerm {
        text: String,
        weight: f64
    }
);
wire_struct!(#[serde(deny_unknown_fields)] Bias { terms: Vec<BiasTerm> } optional { prompt: String => with_prompt });
wire_struct!(BiasApplied {
    terms_used: usize,
    terms_dropped: usize,
    prompt_used: bool
});

/// First pass only: tokenizers must recount concatenated text, separators included.
pub fn trim_bias(term_costs: &[u64], prompt_cost: Option<u64>, max: u64) -> BiasApplied {
    let mut remaining = max;
    let mut used = 0;
    for cost in term_costs {
        if *cost > remaining {
            break;
        }
        remaining -= cost;
        used += 1;
    }
    BiasApplied::new(
        used,
        term_costs.len() - used,
        prompt_cost.is_some_and(|cost| cost <= remaining),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PackPurpose {
    Bias,
    Front,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaPrivacy {
    LocalOnly,
    RemoteAllowed,
}

wire_struct!(PrivacyRule {
    media: MediaPrivacy,
    bias_remote: bool
});
pub type Privacy = BTreeMap<Kind, PrivacyRule>;

pub fn privacy_for(privacy: &Privacy, kind: &Kind) -> PrivacyRule {
    privacy
        .get(kind)
        .cloned()
        .unwrap_or_else(|| PrivacyRule::new(MediaPrivacy::LocalOnly, false))
}

/// Callers must check both media policy and bias permission before remote use.
pub fn check_privacy(
    privacy: &Privacy,
    kind: &Kind,
    remote: bool,
    sends_bias: bool,
) -> CheckResult {
    let rule = privacy_for(privacy, kind);
    if remote && (rule.media == MediaPrivacy::LocalOnly || (sends_bias && !rule.bias_remote)) {
        return Err(CheckError::new("privacy_denied"));
    }
    Ok(())
}

wire_struct!(#[serde(deny_unknown_fields)] ContextRequest { agent_id: String, purpose: PackPurpose } optional {
    if_version: String => with_if_version,
});
wire_struct!(UnchangedPack {
    unchanged: bool,
    version: String
});
wire_struct!(BiasPack { version: String, terms: Vec<BiasTerm>, built_at_ms: u64, privacy: Privacy } optional { prompt: String => with_prompt });
wire_struct!(FrontPack {
    version: String, persona: String, memories: Vec<serde_json::Value>, board: serde_json::Value,
    recent: serde_json::Value, built_at_ms: u64, privacy: Privacy,
});

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum ContextAnswer {
    Unchanged(UnchangedPack),
    Bias(BiasPack),
    Front(FrontPack),
}

pub fn check_bias(bias: &Bias) -> CheckResult {
    for term in &bias.terms {
        if !(1..=256).contains(&term.text.chars().count())
            || !term.weight.is_finite()
            || !(0.0..=1.0).contains(&term.weight)
        {
            return Err(CheckError::new("invalid_params").with_field("terms"));
        }
    }
    Ok(())
}

pub fn check_pack(pack: &ContextAnswer) -> CheckResult {
    if crate::encoded_len(pack)? > MESSAGE_MAX {
        return Err(CheckError::new("pack_too_large"));
    }
    match pack {
        ContextAnswer::Unchanged(value) if !value.unchanged => {
            Err(CheckError::new("invalid_pack").with_field("unchanged"))
        }
        ContextAnswer::Bias(value) => check_bias(&Bias::new(value.terms.clone())),
        ContextAnswer::Front(value) if !value.memories.is_empty() => {
            Err(CheckError::new("invalid_pack").with_field("memories"))
        }
        _ => Ok(()),
    }
}
