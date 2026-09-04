/// Timescale filter - speed/pitch/rate control.
///
/// Semantics (matching lavaplayer's TimescaleFilter):
///   - `speed` s: duration /= s AND pitch *= s   (varispeed)
///   - `rate`  rho: duration /= rho, pitch unchanged  (tempo / time-stretch)
///   - `pitch` p: pitch *= p, duration unchanged      (pitch-shift)
///
/// Implementation:
///   1. rubato sinc resampler with output/input ratio r = 1 / (pitch * speed)
///      -> duration x r, pitch x (1/r)
///   2. WSOLA time-stretch with factor alpha = rate / pitch
///      -> duration / alpha, pitch unchanged
///
/// Net: duration / (speed * rate), pitch * (pitch * speed). Correct for any
/// combination, including defaults (all 1.0 => bit-identical passthrough).
use super::wsola::Wsola;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

const CHUNK_FRAMES: usize = 1024;

pub struct Timescale {
    pub speed: f64,
    pub pitch: f64,
    pub rate: f64,

    _sample_rate: f64,

    // rubato resampler; None when ratio == 1.0
    resampler: Option<SincFixedIn<f32>>,
    // deinterleaved input accumulation for fixed-size resampler chunks
    res_in: [Vec<f32>; 2],
    res_out_pending: [Vec<f32>; 2],

    wsola: Wsola,
    // interleaved output not yet consumed
    out_queue: Vec<f32>,
}

impl Timescale {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            speed: 1.0,
            pitch: 1.0,
            rate: 1.0,
            _sample_rate: sample_rate,
            resampler: None,
            res_in: [Vec::new(), Vec::new()],
            res_out_pending: [Vec::new(), Vec::new()],
            wsola: Wsola::new(sample_rate),
            out_queue: Vec::new(),
        }
    }

    pub fn set_speed(&mut self, v: f64) {
        self.speed = v.clamp(0.25, 4.0);
    }

    pub fn set_pitch(&mut self, v: f64) {
        self.pitch = v.clamp(0.25, 4.0);
    }

    pub fn set_rate(&mut self, v: f64) {
        self.rate = v.clamp(0.25, 4.0);
    }

    /// Rebuild DSP graph after parameter changes. Must be called once after
    /// constructing/configuring, before processing.
    pub fn prepare(&mut self) {
        let ratio = 1.0 / (self.pitch * self.speed);

        self.resampler = if (ratio - 1.0).abs() > 1e-6 {
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                oversampling_factor: 256,
                interpolation: SincInterpolationType::Cubic,
                window: WindowFunction::BlackmanHarris,
            };
            Some(
                SincFixedIn::<f32>::new(ratio, 4.0, params, CHUNK_FRAMES, 2)
                    .expect("valid resampler parameters"),
            )
        } else {
            None
        };

        self.wsola.set_alpha(self.rate / self.pitch);
    }

    /// Reset all internal state (keeps parameters).
    pub fn reset(&mut self) {
        self.res_in = [Vec::new(), Vec::new()];
        self.res_out_pending = [Vec::new(), Vec::new()];
        self.out_queue.clear();
        self.wsola.reset();
        let prepared = self.resampler.is_some();
        if prepared {
            self.prepare();
        }
    }

    /// Push an arbitrary-size interleaved stereo chunk through the filter.
    /// Returns processed interleaved audio; length may differ from input when
    /// speed/rate/pitch are active. May return empty while buffering.
    pub fn push(&mut self, interleaved: &[f32]) -> Vec<f32> {
        let mut stage: Vec<f32> = interleaved.to_vec();

        // Stage 1: rubato resampling
        if let Some(resampler) = self.resampler.as_mut() {
            let mut out = Vec::with_capacity(stage.len());
            for chunk in stage.chunks(2) {
                self.res_in[0].push(chunk[0]);
                self.res_in[1].push(*chunk.get(1).unwrap_or(&0.0));
            }

            while self.res_in[0].len() >= CHUNK_FRAMES {
                let wave_in = vec![
                    self.res_in[0].drain(..CHUNK_FRAMES).collect::<Vec<f32>>(),
                    self.res_in[1].drain(..CHUNK_FRAMES).collect::<Vec<f32>>(),
                ];
                match resampler.process(&wave_in, None) {
                    Ok(mut wave_out) => {
                        self.res_out_pending[0].append(&mut wave_out[0]);
                        self.res_out_pending[1].append(&mut wave_out[1]);
                    }
                    Err(e) => {
                        tracing::warn!("resampler error: {}", e);
                        break;
                    }
                }
            }

            // Drain pending resampled frames into interleaved stage
            let n = self.res_out_pending[0].len();
            out.reserve(n * 2);
            for i in 0..n {
                out.push(self.res_out_pending[0][i]);
                out.push(self.res_out_pending[1][i]);
            }
            self.res_out_pending[0].clear();
            self.res_out_pending[1].clear();

            stage = out;
        }

        // Stage 2: WSOLA time-stretch
        if (self.rate / self.pitch - 1.0).abs() > 1e-6 {
            self.wsola.push(&stage);
            self.out_queue.reserve(stage.len());
            self.wsola.drain(&mut self.out_queue);
            std::mem::take(&mut self.out_queue)
        } else {
            stage
        }
    }

    /// Flush end-of-stream: drains resampler/WSOLA tails.
    pub fn flush(&mut self) -> Vec<f32> {
        let mut out = Vec::new();

        // Flush remaining partial chunk and Sinc delay line
        if let Some(resampler) = self.resampler.as_mut() {
            while !self.res_in[0].is_empty() {
                self.res_in[0].resize(CHUNK_FRAMES, 0.0);
                self.res_in[1].resize(CHUNK_FRAMES, 0.0);

                let wave_in = vec![
                    self.res_in[0].drain(..CHUNK_FRAMES).collect::<Vec<f32>>(),
                    self.res_in[1].drain(..CHUNK_FRAMES).collect::<Vec<f32>>(),
                ];
                if let Ok(wave_out) = resampler.process(&wave_in, None) {
                    self.res_out_pending[0].extend_from_slice(&wave_out[0]);
                    self.res_out_pending[1].extend_from_slice(&wave_out[1]);
                }
            }

            // A couple of chunks of pure silence to flush the sinc delay line
            let silence = vec![vec![0.0f32; CHUNK_FRAMES], vec![0.0f32; CHUNK_FRAMES]];
            for _ in 0..3 {
                if let Ok(wave_out) = resampler.process(&silence, None) {
                    self.res_out_pending[0].extend_from_slice(&wave_out[0]);
                    self.res_out_pending[1].extend_from_slice(&wave_out[1]);
                }
            }
            for i in 0..self.res_out_pending[0].len() {
                self.wsola
                    .push(&[self.res_out_pending[0][i], self.res_out_pending[1][i]]);
            }
            self.res_out_pending[0].clear();
            self.res_out_pending[1].clear();
        }

        self.wsola.flush();
        let mut drained = Vec::new();
        self.wsola.drain(&mut drained);
        out.extend_from_slice(&drained);
        out
    }
}
