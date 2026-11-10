use std::collections::BTreeSet;

use crate::{bias::BiasLimit, CheckError, CheckResult, MESSAGE_MAX};

open_enum!(Group {
    Jobs => "jobs", UsageFeed => "usage_feed", StreamTranscribe => "stream_transcribe",
    Realtime => "realtime", BiasPacks => "bias_packs", VoiceFront => "voice-front/v1",
});
open_enum!(Kind {
    Transcribe => "transcribe", Synthesize => "synthesize", Image => "image", Video => "video",
    StreamTranscribe => "stream_transcribe", Realtime => "realtime",
});
open_enum!(LatencyClass { Realtime => "realtime", Interactive => "interactive", Batch => "batch" });

impl Kind {
    pub fn group(&self) -> Option<Group> {
        match self {
            Self::Transcribe | Self::Synthesize | Self::Image | Self::Video => Some(Group::Jobs),
            Self::StreamTranscribe => Some(Group::StreamTranscribe),
            Self::Realtime => Some(Group::Realtime),
            Self::Unknown(_) => None,
        }
    }
}

wire_struct!(Engine {
    kind: Kind, provider: String, model: String, local: bool,
    formats: Vec<String>, latency_class: LatencyClass,
} optional {
    reference_formats: Vec<String> => with_reference_formats,
    languages: Vec<String> => with_languages,
    voices: Vec<String> => with_voices,
    sizes: Vec<String> => with_sizes,
    resolutions: Vec<String> => with_resolutions,
    qualities: Vec<String> => with_qualities,
    max_input_bytes: u64 => with_max_input_bytes,
    max_duration_ms: u64 => with_max_duration_ms,
    bias_limit: BiasLimit => with_bias_limit,
});

wire_struct!(Describe { groups: Vec<Group>, engines: Vec<Engine> } optional {
    jobs_survive_restart: bool => with_jobs_survive_restart,
    token_max_ttl_ms: u64 => with_token_max_ttl_ms,
    call_silence_timeout_s: u64 => with_call_silence_timeout_s,
});

pub fn check_describe(describe: &Describe) -> CheckResult {
    check_describe_value(
        &serde_json::to_value(describe).map_err(|_| CheckError::new("describe_too_large"))?,
    )
}

/// Check raw facts too, so a wrong JSON type gets the group-fact problem code.
pub fn check_describe_value(value: &serde_json::Value) -> CheckResult {
    if crate::encoded_len(value)? > MESSAGE_MAX {
        return Err(CheckError::new("describe_too_large"));
    }
    let groups: Vec<Group> =
        serde_json::from_value(value.get("groups").cloned().unwrap_or_default())
            .map_err(|_| CheckError::new("invalid_params").with_field("groups"))?;
    let has = |group: &Group| groups.contains(group);
    if has(&Group::StreamTranscribe) && !has(&Group::Jobs) {
        return Err(CheckError::new("stream_without_jobs"));
    }
    if has(&Group::VoiceFront) && !has(&Group::Realtime) {
        return Err(CheckError::new("voice_front_without_realtime"));
    }
    for (group, fact, boolean) in [
        (Group::Jobs, "jobs_survive_restart", true),
        (Group::Realtime, "token_max_ttl_ms", false),
        (Group::Realtime, "call_silence_timeout_s", false),
    ] {
        if has(&group)
            && !value.get(fact).is_some_and(|v| {
                if boolean {
                    v.is_boolean()
                } else {
                    v.as_u64().is_some_and(|n| n >= 1)
                }
            })
        {
            return Err(CheckError::new("group_fact_invalid").with_field(fact));
        }
    }
    if value
        .get("engines")
        .and_then(|v| v.as_array())
        .is_some_and(|engines| {
            engines.iter().any(|engine| {
                engine.get("bias_limit").is_some_and(|limit| {
                    !limit
                        .get("max")
                        .and_then(|v| v.as_u64())
                        .is_some_and(|n| n >= 1)
                })
            })
        })
    {
        return Err(CheckError::new("bias_limit_invalid"));
    }
    let describe: Describe =
        serde_json::from_value(value.clone()).map_err(|_| CheckError::new("invalid_params"))?;
    let mut keys = BTreeSet::new();
    for engine in &describe.engines {
        if let Some(group) = engine.kind.group() {
            if !has(&group) {
                return Err(CheckError::new("engine_without_group"));
            }
            if !engine.local && !has(&Group::UsageFeed) {
                return Err(CheckError::new("usage_without_usage_feed"));
            }
        }
        if !keys.insert((&engine.kind, &engine.provider, &engine.model)) {
            return Err(CheckError::new("duplicate_engine"));
        }
        let lists = [
            &engine.reference_formats,
            &engine.languages,
            &engine.voices,
            &engine.sizes,
            &engine.resolutions,
            &engine.qualities,
        ];
        if std::iter::once(&engine.provider)
            .chain(std::iter::once(&engine.model))
            .chain(engine.formats.iter())
            .chain(lists.into_iter().flatten().flatten())
            .any(|value| value.chars().count() > 256)
        {
            return Err(CheckError::new("fact_too_long"));
        }
        if engine
            .bias_limit
            .as_ref()
            .is_some_and(|limit| limit.max == 0)
        {
            return Err(CheckError::new("bias_limit_invalid"));
        }
        if engine.formats.is_empty() {
            return Err(CheckError::new("formats_empty"));
        }
        if engine.kind == Kind::Realtime && engine.local {
            return Err(CheckError::new("local_realtime_engine"));
        }
    }
    Ok(())
}

pub fn language_matches(declared: &str, requested: &str) -> bool {
    let declared = declared.to_ascii_lowercase();
    let requested = requested.to_ascii_lowercase();
    requested == declared
        || requested
            .strip_prefix(&declared)
            .is_some_and(|tail| tail.starts_with('-'))
}

pub fn mime_matches(declared: &str, requested: &str) -> bool {
    fn parts(mime: &str) -> Option<(String, std::collections::BTreeMap<String, String>)> {
        let mut parts = mime.split(';');
        let essence = parts.next()?.trim().to_ascii_lowercase();
        if !essence.contains('/') {
            return None;
        }
        let mut params = std::collections::BTreeMap::new();
        for part in parts {
            let (key, value) = part.split_once('=')?;
            params.insert(
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"').to_string(),
            );
        }
        Some((essence, params))
    }
    match (parts(declared), parts(requested)) {
        (Some((d, dp)), Some((r, rp))) => {
            d == r && dp.iter().all(|(key, value)| rp.get(key) == Some(value))
        }
        _ => false,
    }
}
