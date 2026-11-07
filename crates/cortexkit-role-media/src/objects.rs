use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};

use crate::{CheckError, CheckResult, CHUNK_MAX, MESSAGE_MAX};

wire_struct!(#[serde(deny_unknown_fields)] MediaValue { mime: String, bytes: u64, sha256: String } optional {
    data: String => with_data, object_id: String => with_object_id,
    width: u64 => with_width, height: u64 => with_height, duration_ms: u64 => with_duration_ms,
});
wire_struct!(OutputObject {
    output_object: MediaValue
});
wire_struct!(
    #[serde(deny_unknown_fields)]
    UploadBeginRequest {
        mime: String,
        bytes: u64,
        sha256: String
    }
);
wire_struct!(UploadBeginAnswer { upload_id: String });
wire_struct!(
    #[serde(deny_unknown_fields)]
    UploadAppendRequest {
        upload_id: String,
        offset: u64,
        data: String
    }
);
wire_struct!(UploadAppendAnswer { received: u64 });
wire_struct!(
    #[serde(deny_unknown_fields)]
    UploadStatusRequest { upload_id: String }
);
wire_struct!(
    #[serde(deny_unknown_fields)]
    UploadCommitRequest { upload_id: String }
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UploadState {
    Open,
    Committed,
    Gone,
}

wire_struct!(UploadStatusAnswer {
    state: UploadState,
    received: u64
});
wire_struct!(UploadCommitAnswer {
    state: UploadState,
    sha256: String,
    mime: String,
    bytes: u64
});
wire_struct!(
    #[serde(deny_unknown_fields)]
    ReadRequest {
        object_id: String,
        offset: u64,
        length: u64
    }
);
wire_struct!(ReadAnswer { data: String });

pub fn sha256_is_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn verify_object(bytes: &[u8], expected_bytes: u64, sha256: &str) -> CheckResult {
    if bytes.len() as u64 != expected_bytes {
        return Err(CheckError::new("upload_size_mismatch"));
    }
    if !sha256_is_valid(sha256) || format!("{:x}", Sha256::digest(bytes)) != sha256 {
        return Err(CheckError::new("upload_sha256_mismatch"));
    }
    Ok(())
}

pub fn check_media(value: &MediaValue) -> CheckResult {
    if value.data.is_some() == value.object_id.is_some() {
        return Err(CheckError::new("invalid_params").with_field("data"));
    }
    if !sha256_is_valid(&value.sha256) {
        return Err(CheckError::new("invalid_params").with_field("sha256"));
    }
    if let Some(data) = &value.data {
        let bytes = decode_chunk(data, "invalid_params")?;
        if value.bytes > CHUNK_MAX {
            return Err(CheckError::new("invalid_params").with_field("data"));
        }
        verify_object(&bytes, value.bytes, &value.sha256)?;
    }
    Ok(())
}

pub fn check_object_facts(value: &MediaValue, stored: &MediaValue, field: &str) -> CheckResult {
    if value.mime != stored.mime || value.bytes != stored.bytes || value.sha256 != stored.sha256 {
        return Err(CheckError::new("object_mismatch").with_field(field));
    }
    Ok(())
}

fn decode_chunk(data: &str, oversize_code: &str) -> Result<Vec<u8>, CheckError> {
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| CheckError::new("invalid_params").with_field("data"))?;
    if bytes.len() as u64 > CHUNK_MAX {
        return Err(CheckError::new(oversize_code).with_field("data"));
    }
    Ok(bytes)
}

pub fn check_upload_append(
    request: &UploadAppendRequest,
    received: u64,
) -> Result<Vec<u8>, CheckError> {
    let bytes = decode_chunk(&request.data, "upload_chunk_too_large")?;
    if request.offset != received {
        return Err(CheckError::new("upload_offset_mismatch"));
    }
    Ok(bytes)
}

pub fn check_read(request: &ReadRequest, bytes: u64) -> CheckResult {
    if !(1..=CHUNK_MAX).contains(&request.length) {
        return Err(CheckError::new("invalid_params").with_field("length"));
    }
    if request.offset > bytes {
        return Err(CheckError::new("invalid_params").with_field("offset"));
    }
    Ok(())
}

/// Paths are relative to params. The largest decoded inline payload comes first.
pub fn inline_data_paths_largest_first(params: &serde_json::Value) -> Vec<String> {
    fn visit(value: &serde_json::Value, path: String, found: &mut Vec<(u64, String)>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(data) = map.get("data").and_then(|v| v.as_str()) {
                    let size = STANDARD
                        .decode(data)
                        .map_or(data.len() as u64, |b| b.len() as u64);
                    found.push((
                        size,
                        if path.is_empty() {
                            "data".into()
                        } else {
                            format!("{path}.data")
                        },
                    ));
                }
                for (key, value) in map {
                    visit(
                        value,
                        if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{path}.{key}")
                        },
                        found,
                    );
                }
            }
            serde_json::Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    visit(value, format!("{path}[{index}]"), found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    visit(params, String::new(), &mut found);
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, path)| path).collect()
}

pub fn check_message_budget(params: &serde_json::Value) -> CheckResult {
    if crate::encoded_len(params)? > MESSAGE_MAX {
        return Err(CheckError::new("invalid_params").with_field(
            inline_data_paths_largest_first(params)
                .into_iter()
                .next()
                .unwrap_or_else(|| "params".into()),
        ));
    }
    Ok(())
}
