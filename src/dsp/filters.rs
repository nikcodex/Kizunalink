/// Filter chain - orchestrates all audio filters in the correct order.
/// Matches Lavalink's filter chain:
/// Volume -> Equalizer -> Karaoke -> Timescale -> Tremolo -> Vibrato ->
/// Distortion -> Rotation -> ChannelMix -> LowPass
///
/// The chain exposes a streaming API: feed arbitrary-size interleaved stereo
/// chunks into [`FilterChain::process`]; returned audio may differ in length
/// when Timescale is active. This is the single code path used both by the
/// live KizunaVoice pipeline and by offline verification.
use super::channel_mix::ChannelMix;
use super::distortion::Distortion;
use super::equalizer::Equalizer;
use super::karaoke::Karaoke;
use super::lowpass::LowPass;
use super::rotation::Rotation;
use super::timescale::Timescale;
use super::tremolo::Tremolo;
use super::vibrato::Vibrato;
use crate::models::filters::Filters;

#[derive(Clone, Copy, PartialEq)]
struct TimescaleSignature(f64, f64, f64);

pub struct FilterChain {
    pub volume: f64,
    equalizer: Equalizer,
    karaoke: Option<Karaoke>,
    timescale: Option<Timescale>,
    tremolo: Option<Tremolo>,
    vibrato: Option<Vibrato>,
    distortion: Option<Distortion>,
    rotation: Option<Rotation>,
    channel_mix: Option<ChannelMix>,
    low_pass: Option<LowPass>,

    sample_rate: f64,
    last_timescale_sig: TimescaleSignature,
}

impl FilterChain {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            volume: 1.0,
            equalizer: Equalizer::new(sample_rate),
            karaoke: None,
            timescale: None,
            tremolo: None,
            vibrato: None,
            distortion: None,
            rotation: None,
            channel_mix: None,
            low_pass: None,
            sample_rate,
            last_timescale_sig: TimescaleSignature(1.0, 1.0, 1.0),
        }
    }

    /// Update output volume multiplier directly.
    pub fn set_volume(&mut self, vol: f32) {
        if vol.is_finite() && vol >= 0.0 {
            self.volume = (vol as f64).clamp(0.0, 10.0);
        }
    }

    /// Update the chain from Lavalink filter settings.
    ///
    /// Returns `true` when the change is *structural* (Timescale graph changed),
    /// meaning an active playback stream should be rebuilt to apply cleanly.
    /// All other changes are applied live on the next processed samples.
    pub fn update_from_lavalink(&mut self, filters: &Filters) -> bool {
        self.volume = filters.volume.unwrap_or(1.0).clamp(0.0, 5.0) as f64;

        if let Some(ref bands) = filters.equalizer {
            for band in bands {
                if band.band >= 0 && (band.band as usize) < 15 {
                    self.equalizer.set_band(band.band as usize, band.gain);
                }
            }
        }

        let new_ts_sig = filters
            .timescale
            .as_ref()
            .map(|t| TimescaleSignature(t.speed, t.pitch, t.rate));

        self.karaoke = filters.karaoke.as_ref().map(|k| {
            let mut karaoke = Karaoke::new(self.sample_rate);
            karaoke.set_level(k.level as f64);
            karaoke.set_mono_level(k.mono_level as f64);
            karaoke.set_filter_band(k.filter_band as f64);
            karaoke.set_filter_width(k.filter_width as f64);
            karaoke
        });

        let structural = match (&self.timescale, new_ts_sig) {
            (None, None) => false,
            (Some(_), Some(sig)) => {
                if sig != self.last_timescale_sig {
                    // rebuild timescale with fresh params
                    let ts = Self::build_timescale(filters, self.sample_rate);
                    self.timescale = Some(ts);
                    true
                } else {
                    false
                }
            }
            (_, Some(sig)) => {
                self.timescale = Some(Self::build_timescale(filters, self.sample_rate));
                self.last_timescale_sig = sig;
                true
            }
            (Some(_), None) => {
                self.timescale = None;
                self.last_timescale_sig = TimescaleSignature(1.0, 1.0, 1.0);
                true
            }
        };
        if let Some(sig) = new_ts_sig {
            self.last_timescale_sig = sig;
        }

        self.tremolo = filters.tremolo.as_ref().map(|t| {
            let mut tremolo = Tremolo::new(self.sample_rate);
            tremolo.set_frequency(t.frequency as f64);
            tremolo.set_depth(t.depth as f64);
            tremolo
        });

        self.vibrato = filters.vibrato.as_ref().map(|v| {
            let mut vibrato = Vibrato::new(self.sample_rate);
            vibrato.set_frequency(v.frequency as f64);
            vibrato.set_depth(v.depth as f64);
            vibrato
        });

        self.distortion = filters.distortion.as_ref().map(|d| {
            let mut dist = Distortion::new();
            dist.set_sin_offset(d.sin_offset as f64);
            dist.set_sin_scale(d.sin_scale as f64);
            dist.set_cos_offset(d.cos_offset as f64);
            dist.set_cos_scale(d.cos_scale as f64);
            dist.set_tan_offset(d.tan_offset as f64);
            dist.set_tan_scale(d.tan_scale as f64);
            dist.set_offset(d.offset as f64);
            dist.set_scale(d.scale as f64);
            dist
        });

        self.rotation = filters.rotation.as_ref().map(|r| {
            let mut rotation = Rotation::new(self.sample_rate);
            rotation.set_rotation_hz(r.rotation_hz);
            rotation
        });

        self.channel_mix = filters.channel_mix.as_ref().map(|cm| {
            let mut mix = ChannelMix::new();
            mix.set_left_to_left(cm.left_to_left as f64);
            mix.set_left_to_right(cm.left_to_right as f64);
            mix.set_right_to_left(cm.right_to_left as f64);
            mix.set_right_to_right(cm.right_to_right as f64);
            mix
        });

        self.low_pass = filters.low_pass.as_ref().map(|lp| {
            let mut low = LowPass::new(self.sample_rate);
            low.set_smoothing(lp.smoothing as f64);
            low
        });

        structural
    }

    fn build_timescale(filters: &Filters, sample_rate: f64) -> Timescale {
        let mut ts = Timescale::new(sample_rate);
        if let Some(t) = &filters.timescale {
            ts.set_speed(t.speed);
            ts.set_pitch(t.pitch);
            ts.set_rate(t.rate);
        }
        ts.prepare();
        ts
    }

    /// Process one arbitrary-size interleaved stereo chunk.
    /// Returns filtered audio; length can differ from input when Timescale is
    /// active (may be empty while buffering).
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        let mut work: Vec<f32> = input.to_vec();

        // Length-preserving stages first (per-sample / per-frame transforms)
        if (self.volume - 1.0).abs() > 1e-4 {
            let vol = self.volume as f32;
            for s in work.iter_mut() {
                *s *= vol;
            }
        }

        self.equalizer.process_buffer(&mut work);

        if let Some(ref mut karaoke) = self.karaoke {
            karaoke.process_buffer(&mut work);
        }

        // Timescale: variable-length stage
        let mut work = match self.timescale.as_mut() {
            Some(ts) => ts.push(&work),
            None => work,
        };

        if work.is_empty() {
            return work;
        }

        if let Some(ref mut tremolo) = self.tremolo {
            tremolo.process_buffer(&mut work);
        }
        if let Some(ref mut vibrato) = self.vibrato {
            vibrato.process_buffer(&mut work);
        }
        if let Some(ref mut distortion) = self.distortion {
            distortion.process_buffer(&mut work);
        }
        if let Some(ref mut rotation) = self.rotation {
            rotation.process_buffer(&mut work);
        }
        if let Some(ref mut channel_mix) = self.channel_mix {
            channel_mix.process_buffer(&mut work);
        }
        if let Some(ref mut low_pass) = self.low_pass {
            low_pass.process_buffer(&mut work);
        }

        // Audiophile Soft-Knee Peak Limiting (Anti-Clipping Headroom Protection)
        // When heavy EQ, Bass Boost, or volume > 1.0 is applied, digital audio samples can exceed +/-1.0.
        // Instead of hard-clamping which causes harsh buzzing square waves, apply smooth tanh saturation
        // for peaks above 0.95 (approx -0.45 dBFS).
        for sample in work.iter_mut() {
            let abs_val = sample.abs();
            if abs_val > 0.95 {
                let sign = sample.signum();
                let compressed = 0.95 + 0.05 * ((abs_val - 0.95) / 0.05).tanh();
                *sample = sign * compressed.min(1.0);
            }
        }

        work
    }

    /// Flush internal buffering at end of stream.
    pub fn flush(&mut self) -> Vec<f32> {
        match self.timescale.as_mut() {
            Some(ts) => ts.flush(),
            None => Vec::new(),
        }
    }

    /// Duration scaling factor currently in effect (source-seconds per wall-second).
    /// Used for position reporting while Timescale is active.
    pub fn duration_factor(&self) -> f64 {
        match &self.timescale {
            Some(ts) => ts.speed * ts.rate,
            None => 1.0,
        }
    }

    /// True when any DSP beyond plain passthrough is configured.
    pub fn is_active(&self) -> bool {
        (self.volume - 1.0).abs() > 1e-4
            || self.timescale.is_some()
            || self.karaoke.is_some()
            || self.tremolo.is_some()
            || self.vibrato.is_some()
            || self.distortion.is_some()
            || self.rotation.is_some()
            || self.channel_mix.is_some()
            || self.low_pass.is_some()
    }

    /// Reset transient filter state (keeps parameters).
    pub fn reset(&mut self) {
        self.equalizer.reset();
        if let Some(ref mut k) = self.karaoke {
            k.reset();
        }
        if let Some(ref mut t) = self.timescale {
            t.reset();
        }
        if let Some(ref mut t) = self.tremolo {
            t.reset();
        }
        if let Some(ref mut v) = self.vibrato {
            v.reset();
        }
        if let Some(ref mut r) = self.rotation {
            r.reset();
        }
        if let Some(ref mut lp) = self.low_pass {
            lp.reset();
        }
    }
}

impl Default for FilterChain {
    fn default() -> Self {
        Self::new(48000.0)
    }
}
