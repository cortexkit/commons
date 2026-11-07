use std::collections::BTreeMap;

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::{
    errors::{check_failure, Failure},
    CheckError, CheckResult, USAGE_RECORD_MAX,
};

open_enum!(ChargeBasis { Vendor => "vendor", Local => "local" });
open_enum!(AuthMethod { ApiKey => "api_key", Oauth => "oauth" });
open_enum!(Unit { Second => "second", Character => "character", Image => "image", Request => "request", Token => "token" });
open_enum!(MeasurementRole { InputAudio => "input_audio", OutputAudio => "output_audio", CachedInputAudio => "cached_input_audio", Input => "input", Output => "output" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UsageStatus {
    Done,
    Failed,
    Cancelled,
    Ended,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Measure {
    Billed,
    Observed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Source {
    VendorReply,
    ProviderMeasured,
}

wire_struct!(Measurement {
    role: MeasurementRole,
    unit: Unit,
    quantity: String,
    measure: Measure,
    source: Source
});

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct UsageRecord {
    pub record_id: String,
    pub principal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub charge_basis: ChargeBasis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<AuthMethod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_request_id: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub status: UsageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    pub selectors: BTreeMap<String, String>,
    pub measurements: Vec<Measurement>,
}

macro_rules! record_builder {
    ($($field:ident: $ty:ty => $with:ident),* $(,)?) => {
        $(pub fn $with(mut self, value: $ty) -> Self { self.$field = Some(value); self })*
    };
}

impl UsageRecord {
    pub fn new(
        record_id: String,
        principal: String,
        provider: String,
        model: String,
        charge_basis: ChargeBasis,
        status: UsageStatus,
    ) -> Self {
        Self {
            record_id,
            principal,
            provider,
            model,
            charge_basis,
            status,
            job_id: None,
            stream_id: None,
            call_id: None,
            decision_id: None,
            credential_ref: None,
            auth_method: None,
            vendor_request_id: None,
            started_at_ms: 0,
            finished_at_ms: 0,
            failure: None,
            selectors: BTreeMap::new(),
            measurements: Vec::new(),
        }
    }
    record_builder! {
        job_id: String => with_job_id, stream_id: String => with_stream_id,
        call_id: String => with_call_id, decision_id: String => with_decision_id,
        credential_ref: String => with_credential_ref, auth_method: AuthMethod => with_auth_method,
        vendor_request_id: String => with_vendor_request_id, failure: Failure => with_failure,
    }
    pub fn with_times(mut self, started_at_ms: u64, finished_at_ms: u64) -> Self {
        self.started_at_ms = started_at_ms;
        self.finished_at_ms = finished_at_ms;
        self
    }
    pub fn with_selectors(mut self, value: BTreeMap<String, String>) -> Self {
        self.selectors = value;
        self
    }
    pub fn with_measurements(mut self, value: Vec<Measurement>) -> Self {
        self.measurements = value;
        self
    }
}

pub fn decimal_is_valid(value: &str) -> bool {
    let (whole, fraction) = value
        .split_once('.')
        .map_or((value, None), |(w, f)| (w, Some(f)));
    !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && (whole == "0" || !whole.starts_with('0'))
        && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

pub fn bounded_vendor_request_id(value: &str) -> String {
    if value.chars().count() <= 256 {
        value.to_owned()
    } else {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }
}

pub fn check_usage_record(record: &UsageRecord) -> CheckResult {
    if crate::encoded_len(record)? >= USAGE_RECORD_MAX {
        return Err(CheckError::new("usage_record_too_large"));
    }
    if [&record.job_id, &record.stream_id, &record.call_id]
        .iter()
        .filter(|id| id.is_some())
        .count()
        != 1
    {
        return Err(CheckError::new("invalid_usage_record").with_field("work_id"));
    }
    if (record.status == UsageStatus::Failed) != record.failure.is_some() {
        return Err(CheckError::new("invalid_usage_record").with_field("failure"));
    }
    if record.charge_basis == ChargeBasis::Local
        && (record.credential_ref.is_some()
            || record.auth_method.is_some()
            || record.vendor_request_id.is_some())
    {
        return Err(CheckError::new("invalid_usage_record").with_field("charge_basis"));
    }
    if record.selectors.len() > 16 || record.measurements.len() > 16 {
        return Err(CheckError::new("invalid_usage_record").with_field("entries"));
    }
    if let Some(failure) = &record.failure {
        check_failure(failure)?;
    }
    for measurement in &record.measurements {
        if !decimal_is_valid(&measurement.quantity) {
            return Err(CheckError::new("invalid_usage_record").with_field("quantity"));
        }
    }
    // Inspect the serialized tree so future open enum values and nested keys
    // receive the same scalar-value bound as ordinary string members.
    fn bounded(value: &serde_json::Value, path: &str) -> CheckResult {
        match value {
            serde_json::Value::String(text)
                if text.chars().count()
                    > if path == "failure.message" {
                        1_024
                    } else {
                        256
                    } =>
            {
                Err(CheckError::new("invalid_usage_record").with_field(path))
            }
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    if key.chars().count() > 256 {
                        return Err(CheckError::new("invalid_usage_record").with_field(path));
                    }
                    let path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    bounded(value, &path)?;
                }
                Ok(())
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    bounded(value, path)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    bounded(
        &serde_json::to_value(record).map_err(|_| CheckError::new("invalid_usage_record"))?,
        "",
    )
}

wire_struct!(#[serde(deny_unknown_fields)] UsageReadRequest { limit: u64 } optional { after_seq: u64 => with_after_seq });
wire_struct!(RawEntry { bytes_b64: String, reason: String } optional { bytes: u64 => with_bytes, sha256: String => with_sha256 });
wire_struct!(UsageEntry { seq: u64 } optional { record: UsageRecord => with_record, raw: RawEntry => with_raw });
wire_struct!(UsageReadAnswer { entries: Vec<UsageEntry> } optional { last_seq: u64 => with_last_seq });
wire_struct!(
    #[serde(deny_unknown_fields)]
    UsageAckRequest { through_seq: u64 }
);
wire_struct!(UsageAckAnswer {
    acked_through_seq: u64
});

pub fn check_usage_entry(entry: &UsageEntry) -> CheckResult {
    if entry.seq == 0 || entry.record.is_some() == entry.raw.is_some() {
        return Err(CheckError::new("invalid_usage_entry"));
    }
    if let Some(record) = &entry.record {
        check_usage_record(record)?;
    }
    if let Some(raw) = &entry.raw {
        if raw.reason == "oversize" {
            if !raw.bytes_b64.is_empty()
                || !raw.bytes.is_some_and(|n| n > USAGE_RECORD_MAX as u64)
                || !raw
                    .sha256
                    .as_deref()
                    .is_some_and(crate::objects::sha256_is_valid)
            {
                return Err(CheckError::new("invalid_usage_entry"));
            }
        } else {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&raw.bytes_b64)
                .map_err(|_| CheckError::new("invalid_usage_entry"))?;
            if bytes.len() > USAGE_RECORD_MAX || raw.bytes.is_some() || raw.sha256.is_some() {
                return Err(CheckError::new("invalid_usage_entry"));
            }
        }
    }
    Ok(())
}

pub fn check_usage_read(request: &UsageReadRequest) -> CheckResult {
    if !(1..=1_000).contains(&request.limit) {
        return Err(CheckError::new("invalid_params").with_field("limit"));
    }
    Ok(())
}

pub fn check_usage_answer(answer: &UsageReadAnswer) -> CheckResult {
    let mut previous = 0;
    for entry in &answer.entries {
        check_usage_entry(entry)?;
        if entry.seq <= previous {
            return Err(CheckError::new("invalid_usage_entry"));
        }
        previous = entry.seq;
    }
    if answer.last_seq != answer.entries.last().map(|entry| entry.seq)
        || crate::encoded_len(answer)? > crate::MESSAGE_MAX
    {
        return Err(CheckError::new("invalid_usage_answer"));
    }
    Ok(())
}

pub fn usage_ack(
    through_seq: u64,
    watermark: u64,
    highest_written: u64,
) -> Result<UsageAckAnswer, CheckError> {
    if through_seq > highest_written {
        return Err(CheckError::new("invalid_params").with_field("through_seq"));
    }
    Ok(UsageAckAnswer::new(watermark.max(through_seq)))
}
