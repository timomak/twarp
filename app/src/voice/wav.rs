//! Minimal RIFF/WAV encoder for the one shape the STT upload needs:
//! 16 kHz mono s16le (twarp 17). Hand-rolled to avoid a dependency for a
//! fixed 44-byte header.

pub const STT_SAMPLE_RATE: u32 = 16_000;

/// Encode mono s16le samples as a WAV file at `sample_rate`.
pub fn encode_mono_s16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = sample_rate * 2; // mono * 16-bit
    let mut out = Vec::with_capacity(44 + data_len as usize);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Downmix interleaved f32 frames to mono and linearly resample `from_rate` →
/// `to_rate`, clamping into i16. Linear interpolation is plenty for speech
/// headed to a transcription model.
pub fn downmix_resample_to_s16(
    interleaved: &[f32],
    channels: u16,
    from_rate: u32,
    to_rate: u32,
) -> Vec<i16> {
    let channels = channels.max(1) as usize;
    let frames = interleaved.len() / channels;
    if frames == 0 || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    let mono: Vec<f32> = (0..frames)
        .map(|frame| {
            let start = frame * channels;
            interleaved[start..start + channels].iter().sum::<f32>() / channels as f32
        })
        .collect();

    let out_len = ((frames as u64 * to_rate as u64) / from_rate as u64) as usize;
    let mut out = Vec::with_capacity(out_len);
    let step = from_rate as f64 / to_rate as f64;
    for i in 0..out_len {
        let pos = i as f64 * step;
        let base = pos as usize;
        let frac = (pos - base as f64) as f32;
        let a = mono[base.min(frames - 1)];
        let b = mono[(base + 1).min(frames - 1)];
        let sample = a + (b - a) * frac;
        out.push((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_shape() {
        let wav = encode_mono_s16(&[0, 1000, -1000], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        // PCM, mono, 16 kHz, 32 kB/s, 16-bit.
        assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1);
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(
            u32::from_le_bytes([wav[28], wav[29], wav[30], wav[31]]),
            32_000
        );
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 6);
        assert_eq!(wav.len(), 44 + 6);
        // Samples are little-endian in order.
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), 1000);
        assert_eq!(i16::from_le_bytes([wav[48], wav[49]]), -1000);
    }

    #[test]
    fn resample_halves_length_and_downmixes() {
        // Stereo 32 kHz → mono 16 kHz: 8 frames in, 4 samples out.
        let interleaved: Vec<f32> = (0..16)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let out = downmix_resample_to_s16(&interleaved, 2, 32_000, 16_000);
        assert_eq!(out.len(), 4);
        // L=0.5, R=-0.5 downmixes to silence.
        assert!(out.iter().all(|&s| s == 0));
    }

    #[test]
    fn resample_identity_preserves_amplitude() {
        let interleaved = vec![0.5f32; 100];
        let out = downmix_resample_to_s16(&interleaved, 1, 16_000, 16_000);
        assert_eq!(out.len(), 100);
        assert!((out[50] as f32 / i16::MAX as f32 - 0.5).abs() < 0.001);
    }

    #[test]
    fn resample_empty_input() {
        assert!(downmix_resample_to_s16(&[], 2, 48_000, 16_000).is_empty());
    }
}
