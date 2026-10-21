//! A pure checker for chunked returned files.
//!
//! [`ReturnAssembler`] applies the `return_chunk` and `return_manifest` rules
//! so that a caller and a producer's tests check a stream the same way. It
//! does no IO: the caller writes the bytes each accepted chunk yields, for
//! example to a temporary file, and places a file only after [`finish`]
//! reports it as verified.
//!
//! [`finish`]: ReturnAssembler::finish

use crate::types::{
    ReturnChunk, ReturnManifest, ReturnManifestOutcome, ReturnPathsFailureCause,
    RETURN_CHUNK_MAX_BYTES,
};
use sha2::{Digest, Sha256};
use std::fmt;

/// One requested path after [`ReturnAssembler::finish`] verified the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerifiedReturn {
    /// Position of the path in the request's `return_paths`.
    pub path_index: u32,
    /// The requested path.
    pub path: String,
    /// [`ReturnManifestOutcome::Returned`] means the bytes received for this
    /// path match the manifest's size and SHA-256, so the caller may place
    /// the file. [`ReturnManifestOutcome::Failed`] means the caller discards
    /// any bytes received for it.
    pub outcome: ReturnManifestOutcome,
}

impl VerifiedReturn {
    /// Read the position of the path in the request's `return_paths`.
    pub fn path_index(&self) -> u32 {
        self.path_index
    }

    /// Read the requested path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Read whether the file was returned and verified, or failed.
    pub fn outcome(&self) -> &ReturnManifestOutcome {
        &self.outcome
    }

    /// Whether the file was returned and its bytes verified.
    pub fn is_returned(&self) -> bool {
        matches!(self.outcome, ReturnManifestOutcome::Returned { .. })
    }
}

/// Why a chunked returned-file stream was refused. The caller discards every
/// byte received for the job's returned files.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReturnStreamError {
    /// A chunk named a path index past the end of the requested paths.
    PathIndexOutOfRange {
        path_index: u32,
        offset: u64,
        path_count: usize,
    },
    /// A chunk went back to an earlier path, so paths were interleaved or out
    /// of ascending order.
    PathOutOfOrder {
        path_index: u32,
        offset: u64,
        /// The path that received the previous chunk.
        current_path_index: u32,
    },
    /// A chunk carried no bytes.
    EmptyChunk { path_index: u32, offset: u64 },
    /// A chunk carried more than [`RETURN_CHUNK_MAX_BYTES`] bytes.
    ChunkTooLarge {
        path_index: u32,
        offset: u64,
        len: usize,
    },
    /// A chunk's offset was not the number of bytes already received for its
    /// path: a gap when larger, an overlap when smaller.
    NonContiguous {
        path_index: u32,
        offset: u64,
        expected_offset: u64,
    },
    /// A chunk would take the total received bytes past the cap.
    OverCap {
        path_index: u32,
        offset: u64,
        /// The total the chunk would have produced.
        total_bytes: u64,
        cap_bytes: u64,
    },
    /// The manifest's entry count differs from the number of requested paths.
    /// `path_index` is the first position present in only one of the two.
    EntryCountMismatch {
        path_index: u32,
        expected: usize,
        actual: usize,
    },
    /// A manifest entry did not echo its requested path exactly.
    PathNotEchoed {
        path_index: u32,
        expected: String,
        actual: String,
    },
    /// A returned entry's size differs from the bytes received for the path.
    SizeMismatch {
        path_index: u32,
        size: u64,
        received: u64,
    },
    /// A returned entry's SHA-256 differs from the digest of the received bytes.
    DigestMismatch {
        path_index: u32,
        expected: String,
        actual: String,
    },
    /// A failed entry had chunks although its cause cannot follow chunks. Only
    /// `read_error`, and causes this version does not recognise, may.
    ChunksForFailedPath {
        path_index: u32,
        cause: ReturnPathsFailureCause,
        received: u64,
    },
    /// The manifest's `total_bytes` differs from the sum of its returned sizes.
    /// This is the only error that concerns no single path.
    TotalMismatch {
        total_bytes: u64,
        returned_bytes: u64,
    },
}

impl ReturnStreamError {
    /// A stable snake_case name for the error, for logs and test vectors.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PathIndexOutOfRange { .. } => "path_index_out_of_range",
            Self::PathOutOfOrder { .. } => "path_out_of_order",
            Self::EmptyChunk { .. } => "empty_chunk",
            Self::ChunkTooLarge { .. } => "chunk_too_large",
            Self::NonContiguous { .. } => "non_contiguous",
            Self::OverCap { .. } => "over_cap",
            Self::EntryCountMismatch { .. } => "entry_count_mismatch",
            Self::PathNotEchoed { .. } => "path_not_echoed",
            Self::SizeMismatch { .. } => "size_mismatch",
            Self::DigestMismatch { .. } => "digest_mismatch",
            Self::ChunksForFailedPath { .. } => "chunks_for_failed_path",
            Self::TotalMismatch { .. } => "total_mismatch",
        }
    }

    /// The path the error concerns; `None` only for [`Self::TotalMismatch`].
    pub fn path_index(&self) -> Option<u32> {
        match self {
            Self::PathIndexOutOfRange { path_index, .. }
            | Self::PathOutOfOrder { path_index, .. }
            | Self::EmptyChunk { path_index, .. }
            | Self::ChunkTooLarge { path_index, .. }
            | Self::NonContiguous { path_index, .. }
            | Self::OverCap { path_index, .. }
            | Self::EntryCountMismatch { path_index, .. }
            | Self::PathNotEchoed { path_index, .. }
            | Self::SizeMismatch { path_index, .. }
            | Self::DigestMismatch { path_index, .. }
            | Self::ChunksForFailedPath { path_index, .. } => Some(*path_index),
            Self::TotalMismatch { .. } => None,
        }
    }

    /// The offset of the refused chunk, for errors raised by a chunk.
    pub fn offset(&self) -> Option<u64> {
        match self {
            Self::PathIndexOutOfRange { offset, .. }
            | Self::PathOutOfOrder { offset, .. }
            | Self::EmptyChunk { offset, .. }
            | Self::ChunkTooLarge { offset, .. }
            | Self::NonContiguous { offset, .. }
            | Self::OverCap { offset, .. } => Some(*offset),
            _ => None,
        }
    }
}

impl fmt::Display for ReturnStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathIndexOutOfRange {
                path_index,
                offset,
                path_count,
            } => write!(
                f,
                "return_chunk path_index {path_index} at offset {offset} is out of range for {path_count} requested paths"
            ),
            Self::PathOutOfOrder {
                path_index,
                offset,
                current_path_index,
            } => write!(
                f,
                "return_chunk for path_index {path_index} at offset {offset} returns to an earlier path after path_index {current_path_index}"
            ),
            Self::EmptyChunk { path_index, offset } => write!(
                f,
                "return_chunk for path_index {path_index} at offset {offset} carries no bytes"
            ),
            Self::ChunkTooLarge {
                path_index,
                offset,
                len,
            } => write!(
                f,
                "return_chunk for path_index {path_index} at offset {offset} carries {len} bytes, more than {RETURN_CHUNK_MAX_BYTES}"
            ),
            Self::NonContiguous {
                path_index,
                offset,
                expected_offset,
            } => write!(
                f,
                "return_chunk for path_index {path_index} starts at offset {offset}, expected {expected_offset}"
            ),
            Self::OverCap {
                path_index,
                offset,
                total_bytes,
                cap_bytes,
            } => write!(
                f,
                "return_chunk for path_index {path_index} at offset {offset} takes the total to {total_bytes} bytes, past the cap of {cap_bytes}"
            ),
            Self::EntryCountMismatch {
                path_index,
                expected,
                actual,
            } => write!(
                f,
                "return_manifest has {actual} entries for {expected} requested paths; path_index {path_index} has no pair"
            ),
            Self::PathNotEchoed {
                path_index,
                expected,
                actual,
            } => write!(
                f,
                "return_manifest entry {path_index} names {actual:?}, not the requested path {expected:?}"
            ),
            Self::SizeMismatch {
                path_index,
                size,
                received,
            } => write!(
                f,
                "return_manifest entry {path_index} declares {size} bytes, but {received} were received"
            ),
            Self::DigestMismatch {
                path_index,
                expected,
                actual,
            } => write!(
                f,
                "return_manifest entry {path_index} declares sha256 {expected}, but the received bytes hash to {actual}"
            ),
            Self::ChunksForFailedPath {
                path_index,
                cause,
                received,
            } => write!(
                f,
                "return_manifest entry {path_index} failed with cause {}, which cannot follow the {received} bytes received",
                String::from(cause.clone())
            ),
            Self::TotalMismatch {
                total_bytes,
                returned_bytes,
            } => write!(
                f,
                "return_manifest total_bytes is {total_bytes}, but its returned entries total {returned_bytes}"
            ),
        }
    }
}

impl std::error::Error for ReturnStreamError {}

/// Checks a chunked returned-file stream for one job.
///
/// Feed every `return_chunk` record to [`Self::accept_chunk`] in stream
/// order, then the `return_manifest` to [`Self::finish`]. On any error, the
/// caller discards every byte received for the job's returned files.
#[derive(Debug, Clone)]
pub struct ReturnAssembler {
    return_paths: Vec<String>,
    cap_bytes: u64,
    /// Bytes received so far for each requested path.
    received: Vec<u64>,
    /// Running digest for each path; `None` until its first chunk arrives.
    hashers: Vec<Option<Sha256>>,
    /// The path that received the most recent chunk.
    current: Option<usize>,
    /// Bytes received across all paths.
    total_bytes: u64,
}

impl ReturnAssembler {
    /// Start checking a stream for the request's `return_paths` under the
    /// effective total byte cap.
    pub fn new(return_paths: &[String], cap_bytes: u64) -> Self {
        Self {
            return_paths: return_paths.to_vec(),
            cap_bytes,
            received: vec![0; return_paths.len()],
            hashers: vec![None; return_paths.len()],
            current: None,
            total_bytes: 0,
        }
    }

    /// Check one chunk and return its bytes for the caller to write at
    /// `chunk.offset` of the file at `chunk.path_index`. A refused chunk
    /// leaves the assembler unchanged.
    pub fn accept_chunk<'c>(
        &mut self,
        chunk: &'c ReturnChunk,
    ) -> Result<&'c [u8], ReturnStreamError> {
        let path_index = chunk.path_index;
        let offset = chunk.offset;
        let data = chunk.data.0.as_slice();
        let index = usize::try_from(path_index)
            .ok()
            .filter(|index| *index < self.return_paths.len())
            .ok_or(ReturnStreamError::PathIndexOutOfRange {
                path_index,
                offset,
                path_count: self.return_paths.len(),
            })?;
        if let Some(current) = self.current.filter(|current| index < *current) {
            return Err(ReturnStreamError::PathOutOfOrder {
                path_index,
                offset,
                current_path_index: index_u32(current),
            });
        }
        if data.is_empty() {
            return Err(ReturnStreamError::EmptyChunk { path_index, offset });
        }
        if data.len() > RETURN_CHUNK_MAX_BYTES {
            return Err(ReturnStreamError::ChunkTooLarge {
                path_index,
                offset,
                len: data.len(),
            });
        }
        let expected_offset = self.received[index];
        if offset != expected_offset {
            return Err(ReturnStreamError::NonContiguous {
                path_index,
                offset,
                expected_offset,
            });
        }
        // The length is at most RETURN_CHUNK_MAX_BYTES, so it fits in a u64.
        let len = data.len() as u64;
        let total_bytes = self.total_bytes.saturating_add(len);
        if total_bytes > self.cap_bytes {
            return Err(ReturnStreamError::OverCap {
                path_index,
                offset,
                total_bytes,
                cap_bytes: self.cap_bytes,
            });
        }
        self.current = Some(index);
        self.received[index] = expected_offset + len;
        self.total_bytes = total_bytes;
        self.hashers[index]
            .get_or_insert_with(Sha256::new)
            .update(data);
        Ok(data)
    }

    /// Check the manifest against the requested paths and the received bytes.
    /// Returns one [`VerifiedReturn`] per requested path, in request order.
    pub fn finish(
        self,
        manifest: &ReturnManifest,
    ) -> Result<Vec<VerifiedReturn>, ReturnStreamError> {
        let expected = self.return_paths.len();
        let actual = manifest.files.len();
        if actual != expected {
            return Err(ReturnStreamError::EntryCountMismatch {
                path_index: index_u32(expected.min(actual)),
                expected,
                actual,
            });
        }
        let mut verified = Vec::with_capacity(expected);
        let mut returned_bytes: u64 = 0;
        let paths = self.return_paths.into_iter();
        let progress = self.received.into_iter().zip(self.hashers);
        for (index, ((path, (received, hasher)), entry)) in
            paths.zip(progress).zip(&manifest.files).enumerate()
        {
            let path_index = index_u32(index);
            if entry.path != path {
                return Err(ReturnStreamError::PathNotEchoed {
                    path_index,
                    expected: path,
                    actual: entry.path.clone(),
                });
            }
            match &entry.outcome {
                ReturnManifestOutcome::Returned { size, sha256 } => {
                    if *size != received {
                        return Err(ReturnStreamError::SizeMismatch {
                            path_index,
                            size: *size,
                            received,
                        });
                    }
                    let digest = format!("{:x}", hasher.unwrap_or_default().finalize());
                    if digest != *sha256 {
                        return Err(ReturnStreamError::DigestMismatch {
                            path_index,
                            expected: sha256.clone(),
                            actual: digest,
                        });
                    }
                    returned_bytes = returned_bytes.saturating_add(*size);
                }
                ReturnManifestOutcome::Failed { cause, .. } => {
                    // A newer runner may define a cause that follows chunks,
                    // so an unrecognised cause discards them without an error.
                    let may_follow_chunks = matches!(
                        cause,
                        ReturnPathsFailureCause::ReadError | ReturnPathsFailureCause::Unknown(_)
                    );
                    if received > 0 && !may_follow_chunks {
                        return Err(ReturnStreamError::ChunksForFailedPath {
                            path_index,
                            cause: cause.clone(),
                            received,
                        });
                    }
                }
            }
            verified.push(VerifiedReturn {
                path_index,
                path,
                outcome: entry.outcome.clone(),
            });
        }
        if manifest.total_bytes != returned_bytes {
            return Err(ReturnStreamError::TotalMismatch {
                total_bytes: manifest.total_bytes,
                returned_bytes,
            });
        }
        Ok(verified)
    }
}

/// Path indexes are `u32` on the wire; a list longer than that cannot be
/// addressed, so larger positions saturate.
fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}
