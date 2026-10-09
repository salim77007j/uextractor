//! Audio extraction: detect embedded OGG/Opus/FLAC/RIFF payloads in SoundWave bulk data,
//! convert ADPCM to WAV, and emit WAV/OGG files.

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum AudioKind {
    Ogg,
    Flac,
    Wav,
    Adpcm,
    Unknown,
}

pub fn detect(data: &[u8]) -> AudioKind {
    if data.len() >= 4 && &data[0..4] == b"OggS" {
        return AudioKind::Ogg;
    }
    if data.len() >= 4 && &data[0..4] == b"fLaC" {
        return AudioKind::Flac;
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return AudioKind::Wav;
    }
    // XWMA / ADPCM heuristics
    AudioKind::Unknown
}

/// Find the first audio stream signature in a blob (used when properties precede the data).
pub fn find_audio_stream(data: &[u8]) -> Option<(usize, AudioKind)> {
    let mut i = 0;
    while i + 4 <= data.len() {
        if &data[i..i + 4] == b"OggS" {
            return Some((i, AudioKind::Ogg));
        }
        if &data[i..i + 4] == b"fLaC" {
            return Some((i, AudioKind::Flac));
        }
        if i + 12 <= data.len() && &data[i..i + 4] == b"RIFF" && &data[i + 8..i + 12] == b"WAVE" {
            return Some((i, AudioKind::Wav));
        }
        i += 4;
    }
    None
}

/// Simple IMA ADPCM block decoder (4-bit nibbles, standard layout).
pub fn decode_ima_adpcm(data: &[u8], channels: usize, block_size: usize) -> Result<(Vec<i16>, u32)> {
    if channels == 0 || channels > 2 || block_size < 8 {
        return Err(Error::asset("bad adpcm params"));
    }
    let samples_per_block = (block_size - 4 * channels) * 2 / channels + 1;
    let mut out = Vec::with_capacity(data.len() / block_size * samples_per_block * channels);
    let mut predictor = [0i32; 2];
    let mut step_index = [0i32; 2];
    let step_table: [i32; 89] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88,
        97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724,
        796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026,
        4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500,
        20350, 22385, 24623, 27086, 29794, 32767,
    ];
    let mut pos = 0;
    let mut sample_count = 0u32;
    while pos + 4 * channels <= data.len() {
        for ch in 0..channels {
            predictor[ch] = i32::from_le_bytes(data[pos + ch * 4..pos + ch * 4 + 4].try_into().unwrap());
            step_index[ch] = data[pos + ch * 4 + 4].min(88) as i32;
        }
        pos += 4 * channels + channels; // header + reserved byte per channel
        let nibble_count = block_size.saturating_sub(4 * channels + channels) * 2;
        let mut block_samples: Vec<i16> = Vec::with_capacity(samples_per_block * channels);
        for i in 0..nibble_count {
            let ch = i % channels;
            let byte_idx = pos + i / channels;
            let Some(&b) = data.get(byte_idx) else { break };
            let nibble = if (i / channels) % 2 == 0 { b & 0x0f } else { b >> 4 };
            let step = step_table[step_index[ch] as usize];
            let diff = (((nibble & 7) as i32 * step) >> 2) + if nibble & 8 != 0 { step } else { 0 };
            predictor[ch] = (predictor[ch] + if nibble & 4 != 0 { -diff } else { diff }).clamp(-32768, 32767);
            step_index[ch] = (step_index[ch] + STEP_ADJ[nibble as usize & 7]).clamp(0, 88);
            block_samples.push(predictor[ch] as i16);
        }
        out.extend_from_slice(&block_samples);
        pos += block_size.saturating_sub(4 * channels + channels) / channels + 4 * channels + channels - 4 * channels;
        sample_count += block_samples.len() as u32 / channels as u32;
        // advance to next block boundary conservatively
        pos = ((pos / block_size) + 1) * block_size;
    }
    Ok((out, sample_count))
}

const STEP_ADJ: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// Write a RIFF/WAVE file (PCM16).
pub fn write_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Extract audio payload: returns (extension, bytes).
pub fn extract_payload(data: &[u8]) -> Result<(String, Vec<u8>)> {
    match detect(data) {
        AudioKind::Ogg => Ok(("ogg".into(), data.to_vec())),
        AudioKind::Flac => Ok(("flac".into(), data.to_vec())),
        AudioKind::Wav => Ok(("wav".into(), data.to_vec())),
        _ => {
            if let Some((off, kind)) = find_audio_stream(data) {
                let ext = match kind {
                    AudioKind::Ogg => "ogg",
                    AudioKind::Flac => "flac",
                    AudioKind::Wav => "wav",
                    _ => "bin",
                };
                return Ok((ext.into(), data[off..].to_vec()));
            }
            Err(Error::unsupported("no recognizable audio stream found"))
        }
    }
}
