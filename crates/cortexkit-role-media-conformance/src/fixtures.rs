//! Generated audio only: fixture transcripts are supplied by scripted engines.

use sha2::{Digest, Sha256};

/// Audio fixture with an exact duration at 16 kHz, mono, signed 16-bit PCM.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum AudioFixture {
    /// One second of digital silence.
    Silence,
    /// One second of a 1 kHz tone at amplitude 8,000.
    Tone,
    /// Half a second of xorshift32 noise, seeded with 0x12345678.
    Noise,
}

impl AudioFixture {
    pub fn duration_ms(self) -> u64 {
        match self {
            Self::Silence | Self::Tone => 1_000,
            Self::Noise => 500,
        }
    }

    /// Uses an integer table rather than platform-dependent trigonometry.
    pub fn wav(self) -> Vec<u8> {
        let samples = self.duration_ms() as u32 * 16;
        let bytes = samples * 2;
        let mut wav = Vec::with_capacity(bytes as usize + 44);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(bytes + 36).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&16_000_u32.to_le_bytes());
        wav.extend_from_slice(&32_000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&bytes.to_le_bytes());
        let tone: [i16; 16] = [
            0, 3061, 5657, 7391, 8000, 7391, 5657, 3061, 0, -3061, -5657, -7391, -8000, -7391,
            -5657, -3061,
        ];
        let mut seed = 0x1234_5678_u32;
        for i in 0..samples {
            let sample = match self {
                Self::Silence => 0,
                Self::Tone => tone[i as usize % tone.len()],
                Self::Noise => {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed >> 16) as i16
                }
            };
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
