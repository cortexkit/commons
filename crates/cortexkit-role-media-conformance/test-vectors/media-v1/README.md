# Generated fixtures

All WAV files are 16 kHz mono signed 16-bit little-endian PCM with a 44-byte WAV
header. `silence.wav` is exactly 1,000 ms of zeros. `tone.wav` is exactly 1,000 ms
of a 1 kHz sine, amplitude 8,000, rounded to the fixed 16-sample integer table
in `src/fixtures.rs`. `noise.wav` is exactly 500 ms of xorshift32 output from seed
`0x12345678`, using its upper 16 bits as each signed sample.

Regenerate with `cargo run -p cortexkit-role-media-conformance --example generate_audio`.
The generator also writes `SHA256SUMS`. Tests compare generated and committed
bytes and verify these independently pinned hashes. There is no recorded speech;
the subject's scripted engine supplies each expected transcript.
