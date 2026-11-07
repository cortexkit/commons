use crate::{
    bias::{Bias, BiasApplied},
    describe::Kind,
    errors::{check_failure, Failure},
    objects::{check_media, MediaValue},
    CheckError, CheckResult,
};

open_enum!(JobStatus { Queued => "queued", Running => "running", Done => "done", Failed => "failed", Cancelled => "cancelled" });

wire_struct!(#[serde(deny_unknown_fields)] SubmitRequest { client_job_id: String, kind: Kind, input: JobInput, model: String } optional {
    decision_id: String => with_decision_id, deadline_ms: u64 => with_deadline_ms,
});
wire_struct!(SubmitAnswer { job_id: String });
wire_struct!(
    #[serde(deny_unknown_fields)]
    StatusRequest { job_id: String }
);
wire_struct!(
    #[serde(deny_unknown_fields)]
    ResultRequest { job_id: String }
);
wire_struct!(
    #[serde(deny_unknown_fields)]
    CancelRequest { job_id: String }
);
wire_struct!(StatusAnswer { status: JobStatus } optional { progress: f64 => with_progress, failure: Failure => with_failure });
wire_struct!(CancelAnswer { status: JobStatus });
wire_struct!(#[serde(deny_unknown_fields)] TranscribeInput { audio: MediaValue } optional { language: String => with_language, bias: Bias => with_bias });
wire_struct!(
    #[serde(deny_unknown_fields)]
    SynthesizeInput {
        text: String,
        voice: String,
        format: String
    }
);
wire_struct!(#[serde(deny_unknown_fields)] ImageInput { prompt: String, size: String } optional {
    quality: String => with_quality, format: String => with_format, references: Vec<MediaValue> => with_references,
});
wire_struct!(#[serde(deny_unknown_fields)] VideoInput { prompt: String, duration_s: u64 } optional {
    resolution: String => with_resolution, format: String => with_format, references: Vec<MediaValue> => with_references,
});
wire_struct!(Word { text: String, start_ms: u64, end_ms: u64 } optional { confidence: f64 => with_confidence });
wire_struct!(TranscribeOutput { text: String, words: Vec<Word>, no_speech: bool } optional {
    language: String => with_language, bias_applied: BiasApplied => with_bias_applied,
});
wire_struct!(SynthesizeOutput { audio: MediaValue });
wire_struct!(ImageOutput { images: Vec<MediaValue> });
wire_struct!(VideoOutput { video: MediaValue });

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum JobInput {
    Transcribe(TranscribeInput),
    Synthesize(SynthesizeInput),
    Image(ImageInput),
    Video(VideoInput),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum JobOutput {
    Transcribe(TranscribeOutput),
    Synthesize(SynthesizeOutput),
    Image(ImageOutput),
    Video(VideoOutput),
    Object(crate::objects::OutputObject),
}

pub fn check_status(answer: &StatusAnswer) -> CheckResult {
    if (answer.status == JobStatus::Failed) != answer.failure.is_some() {
        return Err(CheckError::new("invalid_status").with_field("failure"));
    }
    if answer
        .progress
        .is_some_and(|p| !p.is_finite() || !(0.0..=1.0).contains(&p))
    {
        return Err(CheckError::new("invalid_status").with_field("progress"));
    }
    if let Some(failure) = &answer.failure {
        check_failure(failure)?;
    }
    Ok(())
}

pub fn check_transcribe_output(output: &TranscribeOutput) -> CheckResult {
    if output.no_speech {
        if !output.text.is_empty() || !output.words.is_empty() {
            return Err(CheckError::new("engine_empty_result"));
        }
    } else if output.text.trim().is_empty() {
        return Err(CheckError::new("engine_empty_result"));
    }
    for word in &output.words {
        if word
            .confidence
            .is_some_and(|p| !p.is_finite() || !(0.0..=1.0).contains(&p))
        {
            return Err(CheckError::new("invalid_output").with_field("words.confidence"));
        }
    }
    Ok(())
}

pub fn check_synthesize_output(output: &SynthesizeOutput) -> CheckResult {
    check_nonempty(&output.audio)
}
fn check_nonempty(value: &MediaValue) -> CheckResult {
    if value.bytes == 0 {
        return Err(CheckError::new("engine_empty_result"));
    }
    check_media(value)
}
pub fn check_image_output(output: &ImageOutput) -> CheckResult {
    if output.images.is_empty() {
        return Err(CheckError::new("engine_empty_result"));
    }
    for image in &output.images {
        check_nonempty(image)?;
        if image.width.is_none() || image.height.is_none() {
            return Err(CheckError::new("invalid_output").with_field("images"));
        }
    }
    Ok(())
}
pub fn check_video_output(output: &VideoOutput) -> CheckResult {
    check_nonempty(&output.video)?;
    if output.video.width.is_none()
        || output.video.height.is_none()
        || output.video.duration_ms.is_none()
    {
        return Err(CheckError::new("invalid_output").with_field("video"));
    }
    Ok(())
}
