// The retained legacy frame constructors refer to their deprecated types.
#![allow(deprecated)]

use crate::{
    bias::{Bias, BiasApplied},
    errors::{check_failure, Failure},
    jobs::Word,
    objects::OutputObject,
    CheckError, CheckResult,
};

pub const STREAM_TRANSCRIBE_OP: &str = "media.stream_transcribe";
pub const STREAM_AUDIO_OP: &str = "media.stream_audio";
pub const STREAM_AUDIO_WINDOW: u64 = 8;

wire_struct!(#[serde(deny_unknown_fields)] StreamRequest { stream_id: String, model: String, format: String } optional {
    decision_id: String => with_decision_id, language: String => with_language, bias: Bias => with_bias,
});
wire_struct!(
    #[serde(deny_unknown_fields)]
    StreamAudioRequest {
        stream_id: String,
        seq: u64,
        data: String
    }
);
wire_struct!(
    #[serde(deny_unknown_fields)]
    StreamEndRequest {
        stream_id: String,
        seq: u64,
        end: bool
    }
);
wire_struct!(StreamAudioAnswer {});
impl Default for StreamAudioAnswer {
    fn default() -> Self {
        Self::new()
    }
}

wire_struct!(
    #[deprecated(
        since = "0.1.8",
        note = "Use StreamAudioRequest with media.stream_audio"
    )]
    #[serde(deny_unknown_fields)]
    AudioFrame {
        seq: u64,
        data: String
    }
);
wire_struct!(
    #[deprecated(since = "0.1.8", note = "Use StreamEndRequest with media.stream_audio")]
    #[serde(deny_unknown_fields)]
    EndFrame {
        seq: u64,
        end: bool
    }
);
wire_struct!(Segment { segment: u64, text: String, words: Vec<Word> });

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamEvent {
    Partial {
        segment: u64,
        text: String,
    },
    Final {
        segment: u64,
        text: String,
        words: Vec<Word>,
    },
}
impl StreamEvent {
    pub fn partial(segment: u64, text: String) -> Self {
        Self::Partial { segment, text }
    }
    pub fn final_segment(value: Segment) -> Self {
        Self::Final {
            segment: value.segment,
            text: value.text,
            words: value.words,
        }
    }
}

wire_struct!(StreamSuccess { segments: Vec<Segment>, no_speech: bool } optional { bias_applied: BiasApplied => with_bias_applied });
wire_struct!(StreamFailure { segments: Vec<Segment>, failure: Failure });

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum StreamAnswer {
    Success(StreamSuccess),
    Failure(StreamFailure),
    Object(OutputObject),
}

pub fn check_stream_answer(answer: &StreamAnswer) -> CheckResult {
    if crate::encoded_len(answer)? > crate::MESSAGE_MAX {
        return Err(CheckError::new("output_too_large"));
    }
    let segments = match answer {
        StreamAnswer::Success(value) => {
            if value.no_speech != value.segments.is_empty() {
                return Err(CheckError::new("engine_empty_result"));
            }
            &value.segments
        }
        StreamAnswer::Failure(value) => {
            check_failure(&value.failure)?;
            &value.segments
        }
        StreamAnswer::Object(value) => return crate::objects::check_media(&value.output_object),
    };
    let mut previous = None;
    for segment in segments {
        if segment.text.trim().is_empty() || previous.is_some_and(|p| segment.segment <= p) {
            return Err(CheckError::new("invalid_output").with_field("segments"));
        }
        previous = Some(segment.segment);
    }
    Ok(())
}
