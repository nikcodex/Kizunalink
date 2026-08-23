//! Test-only helpers: WAV read/write and DSP analysis (Goertzel, envelopes).
//! Used by the offline filter verification suite.

#![cfg(test)]

use std::f64::consts::PI;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

/// Write interleaved f32 samples as a 16-bit PCM WAV file.
pub fn write_wav_i16(path: &Path, samples: &[f32], sample_rate: u32, channels: u16) {
    let data_len = samples.len() * 2;
    let file = File::create(path).expect("create wav");
    let mut w = BufWriter::new(file);

    let _ = w.write_all(b"RIFF");
    let _ = w.write_all(&(36 + data_len as u32).to_le_bytes());
    let _ = w.write_all(b"WAVE");
    let _ = w.write_all(b"fmt ");
    let _ = w.write_all(&16u32.to_le_bytes());
    let _ = w.write_all(&1u16.to_le_bytes()); // PCM
    let _ = w.write_all(&channels.to_le_bytes());
    let _ = w.write_all(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * channels as u32 * 2;
    let _ = w.write_all(&byte_rate.to_le_bytes());
    let block_align = channels * 2;
    let _ = w.write_all(&block_align.to_le_bytes());
    let _ = w.write_all(&16u16.to_le_bytes());
    let _ = w.write_all(b"data");
    let _ = w.write_all(&(data_len as u32).to_le_bytes());

    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * 32767.0) as i16;
        let _ = w.write_all(&v.to_le_bytes());
    }
}

/// Read a 16-bit PCM WAV file into interleaved f32 samples.
pub fn read_wav_i16(path: &Path) -> Result<(Vec<f32>, u32, u16), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut r = BufReader::new(file);

    let mut riff = [0u8; 4];
    r.read_exact(&mut riff).map_err(|e| e.to_string())?;
    if &riff != b"RIFF" {
        return Err("not RIFF".into());
    }
    let mut size_buf = [0u8; 4];
    r.read_exact(&mut size_buf).map_err(|e| e.to_string())?;
    let mut wave = [0u8; 4];
    r.read_exact(&mut wave).map_err(|e| e.to_string())?;
    if &wave != b"WAVE" {
        return Err("not WAVE".into());
    }

    let mut sample_rate = 48000u32;
    let mut channels = 2u16;

    loop {
        let mut chunk_id = [0u8; 4];
        if r.read_exact(&mut chunk_id).is_err() {
            break;
        }
        r.read_exact(&mut size_buf).map_err(|e| e.to_string())?;
        let chunk_size = u32::from_le_bytes(size_buf) as usize;

        match &chunk_id {
            b"fmt " => {
                let mut fmt = vec![0u8; chunk_size];
                r.read_exact(&mut fmt).map_err(|e| e.to_string())?;
                channels = u16::from_le_bytes([fmt[2], fmt[3]]);
                sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            }
            b"data" => {
                let mut data = vec![0u8; chunk_size];
                r.read_exact(&mut data).map_err(|e| e.to_string())?;
                let samples: Vec<f32> = data
                    .chunks(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32767.0)
                    .collect();
                return Ok((samples, sample_rate, channels));
            }
            _ => {
                // skip unknown chunks
                let mut sink = vec![0u8; chunk_size];
                r.read_exact(&mut sink).map_err(|e| e.to_string())?;
            }
        }
    }
    Err("no data chunk".into())
}

/// Goertzel single-bin amplitude estimate for a real signal.
pub fn goertzel(samples: &[f32], freq: f64, sample_rate: f64) -> f64 {
    let n = samples.len();
    if n == 0 {
        return 0.0;
    }
    let k = 2.0 * PI * freq / sample_rate;
    let coeff = 2.0 * k.cos();
    let (mut s_prev, mut s_prev2) = (0.0f64, 0.0f64);
    for &s in samples {
        let s0 = s as f64 + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s0;
    }
    let power = s_prev2 * s_prev2 + s_prev * s_prev - coeff * s_prev * s_prev2;
    (power / (n as f64 / 2.0)).sqrt()
}

/// RMS level of a slice.
pub fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt()
}

/// RMS envelope in fixed windows (returns per-window RMS).
pub fn rms_envelope(samples: &[f32], window_samples: usize) -> Vec<f64> {
    samples
        .chunks(window_samples)
        .filter(|c| !c.is_empty())
        .map(rms)
        .collect()
}

/// Dominant zero-crossing frequency estimate (works for near-sinusoids).
pub fn zero_crossing_freq(samples: &[f32], sample_rate: f64) -> f64 {
    let mut crossings = 0usize;
    for pair in samples.windows(2) {
        if pair[0] <= 0.0 && pair[1] > 0.0 {
            crossings += 1;
        }
    }
    crossings as f64 * sample_rate / samples.len() as f64
}

/// Generate the standard multi-tone test signal: chord of 100 Hz + 1 kHz +
/// 10 kHz at equal amplitude, stereo identical channels.
pub fn generate_chord(seconds: f64, sample_rate: f64) -> Vec<f32> {
    let frames = (seconds * sample_rate) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    let fade_n = (0.01 * sample_rate) as usize; // 10ms fade-in/out to avoid clicks

    for i in 0..frames {
        let t = i as f64 / sample_rate;
        let mut v = 0.30 * (2.0 * PI * 100.0 * t).sin()
            + 0.30 * (2.0 * PI * 1000.0 * t).sin()
            + 0.30 * (2.0 * PI * 10000.0 * t).sin();

        // fade edges
        let fade_in = (i as f64 / fade_n as f64).min(1.0);
        let fade_out = ((frames - i) as f64 / fade_n as f64).min(1.0);
        v *= fade_in * fade_out;

        out.push(v as f32);
        out.push(v as f32);
    }
    out
}
