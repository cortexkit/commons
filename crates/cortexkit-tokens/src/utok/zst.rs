//! cortexkit-tokens: zstd decoding in pure Rust.
//!
//! Upstream decodes the embedded tables with the `zstd` crate, which compiles
//! the C library. A runner built for WebAssembly has no C toolchain, and the
//! tables only ever need decoding, so this uses `ruzstd`, a pure-Rust
//! decoder. Both decode a valid frame to the same bytes.

use std::io::Read;

/// Decode one embedded zstd frame. The tables are compiled in, so a failure
/// is a build defect, never bad input.
pub(crate) fn decode_all(data: &[u8]) -> Vec<u8> {
    let mut source = data;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(&mut source)
        .expect("utoken: embedded zstd frame header is valid");
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .expect("utoken: embedded zstd frame decodes");
    out
}
