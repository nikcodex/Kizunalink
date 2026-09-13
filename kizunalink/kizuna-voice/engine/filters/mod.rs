// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod biquad;
pub mod channel_mix;
pub mod chorus;
pub mod compressor;
pub mod delay_line;
pub mod distortion;
pub mod echo;
pub mod equalizer;
pub mod flanger;
pub mod high_pass;
pub mod karaoke;
pub mod lfo;
pub mod low_pass;
pub mod normalization;
pub mod phaser;
pub mod phonograph;
pub mod reverb;
pub mod rotation;
pub mod spatial;
pub mod timescale;
pub mod tremolo;
pub mod vibrato;
pub mod volume;

use crate::{
    config::FiltersConfig,
    discord::player::{EqBand, Filters},
};

/// Declares every filter exactly once. The macro expands to:
/// 1. The `ConcreteFilter` enum variants
/// 2. `process()` and `reset()` match arms
/// 3. `validate_filters()` checks
/// 4. `FilterChain::from_config()` construction
///
/// Syntax per entry:
///   (VariantName, config_field, filter_module::Type, constructor_expr, api_name)
///
/// For boxed filters, use `Box<module::Type>` as the third arg.
macro_rules! define_filters {
    ($(
        ($variant:ident, $config_field:ident, $filter_ty:ty, $api_name:expr)
    ),* $(,)?) => {

        // ── ConcreteFilter enum ──────────────────────────────────────────
        pub enum ConcreteFilter {
            $($variant($filter_ty),)*
        }

        impl ConcreteFilter {
            #[inline(always)]
            pub fn process(&mut self, samples: &mut [i16]) {
                match self {
                    $(Self::$variant(f) => f.process(samples),)*
                }
            }

            pub fn reset(&mut self) {
                match self {
                    $(Self::$variant(f) => f.reset(),)*
                }
            }
        }

        // ── validate_filters ─────────────────────────────────────────────
        pub fn validate_filters(filters: &Filters, config: &FiltersConfig) -> Vec<&'static str> {
            let mut invalid = Vec::new();
            $(if filters.$config_field.is_some() && !config.$config_field {
                invalid.push($api_name);
            })*
            invalid
        }

    };
}

// Register all filters: (Variant, config_field, Type, API name)
define_filters! {
    (Volume,        volume,        volume::VolumeFilter,                              "volume"),
    (Equalizer,     equalizer,     Box<equalizer::EqualizerFilter>,                   "equalizer"),
    (Karaoke,       karaoke,       karaoke::KaraokeFilter,                            "karaoke"),
    (Tremolo,       tremolo,       tremolo::TremoloFilter,                            "tremolo"),
    (Vibrato,       vibrato,       vibrato::VibratoFilter,                            "vibrato"),
    (Rotation,      rotation,      rotation::RotationFilter,                          "rotation"),
    (Distortion,    distortion,    distortion::DistortionFilter,                      "distortion"),
    (ChannelMix,    channel_mix,   channel_mix::ChannelMixFilter,                     "channelMix"),
    (LowPass,       low_pass,      low_pass::LowPassFilter,                           "lowPass"),
    (Echo,          echo,          echo::EchoFilter,                                  "echo"),
    (HighPass,      high_pass,     high_pass::HighPassFilter,                         "highPass"),
    (Normalization, normalization, normalization::NormalizationFilter,                "normalization"),
    (Chorus,        chorus,        chorus::ChorusFilter,                              "chorus"),
    (Compressor,    compressor,    compressor::CompressorFilter,                      "compressor"),
    (Flanger,       flanger,       flanger::FlangerFilter,                            "flanger"),
    (Phaser,        phaser,        phaser::PhaserFilter,                              "phaser"),
    (Phonograph,    phonograph,    Box<phonograph::PhonographFilter>,                 "phonograph"),
    (Reverb,        reverb,        reverb::ReverbFilter,                              "reverb"),
    (Spatial,       spatial,       spatial::SpatialFilter,                            "spatial"),
}

pub trait AudioFilter: Send {
    fn process(&mut self, samples: &mut [i16]);
    fn is_enabled(&self) -> bool;
    fn reset(&mut self);
}

pub struct FilterChain {
    filters: Vec<ConcreteFilter>,
    timescale: Option<timescale::TimescaleFilter>,
    timescale_buffer: Vec<i16>,
}

impl FilterChain {
    pub fn from_config(config: &Filters) -> Self {
        let mut filters = Vec::new();

        if let Some(vol) = config.volume {
            let f = volume::VolumeFilter::new(vol);
            if f.is_enabled() {
                filters.push(ConcreteFilter::Volume(f));
            }
        }

        if let Some(ref bands) = config.equalizer {
            let band_tuples: Vec<(u8, f32)> =
                bands.iter().map(|b: &EqBand| (b.band, b.gain)).collect();
            let f = equalizer::EqualizerFilter::new(&band_tuples);
            if f.is_enabled() {
                filters.push(ConcreteFilter::Equalizer(Box::new(f)));
            }
        }

        if let Some(ref k) = config.karaoke {
            let f = karaoke::KaraokeFilter::new(
                k.level.unwrap_or(1.0),
                k.mono_level.unwrap_or(1.0),
                k.filter_band.unwrap_or(220.0),
                k.filter_width.unwrap_or(100.0),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Karaoke(f));
            }
        }

        if let Some(ref t) = config.tremolo {
            let f = tremolo::TremoloFilter::new(t.frequency.unwrap_or(2.0), t.depth.unwrap_or(0.5));
            if f.is_enabled() {
                filters.push(ConcreteFilter::Tremolo(f));
            }
        }

        if let Some(ref v) = config.vibrato {
            let f = vibrato::VibratoFilter::new(v.frequency.unwrap_or(2.0), v.depth.unwrap_or(0.5));
            if f.is_enabled() {
                filters.push(ConcreteFilter::Vibrato(f));
            }
        }

        if let Some(ref r) = config.rotation {
            let f = rotation::RotationFilter::new(r.rotation_hz.unwrap_or(0.0));
            if f.is_enabled() {
                filters.push(ConcreteFilter::Rotation(f));
            }
        }

        if let Some(ref d) = config.distortion {
            let f = distortion::DistortionFilter::new(
                d.sin_offset.unwrap_or(0.0),
                d.sin_scale.unwrap_or(1.0),
                d.cos_offset.unwrap_or(0.0),
                d.cos_scale.unwrap_or(1.0),
                d.tan_offset.unwrap_or(0.0),
                d.tan_scale.unwrap_or(1.0),
                d.offset.unwrap_or(0.0),
                d.scale.unwrap_or(1.0),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Distortion(f));
            }
        }

        if let Some(ref cm) = config.channel_mix {
            let f = channel_mix::ChannelMixFilter::new(
                cm.left_to_left.unwrap_or(1.0),
                cm.left_to_right.unwrap_or(0.0),
                cm.right_to_left.unwrap_or(0.0),
                cm.right_to_right.unwrap_or(1.0),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::ChannelMix(f));
            }
        }

        if let Some(ref lp) = config.low_pass {
            let f = low_pass::LowPassFilter::new(lp.smoothing.unwrap_or(20.0));
            if f.is_enabled() {
                filters.push(ConcreteFilter::LowPass(f));
            }
        }

        if let Some(ref e) = config.echo {
            let f = echo::EchoFilter::new(e.echo_length.unwrap_or(1.0), e.decay.unwrap_or(0.5));
            if f.is_enabled() {
                filters.push(ConcreteFilter::Echo(f));
            }
        }

        if let Some(ref hp) = config.high_pass {
            let f = high_pass::HighPassFilter::new(
                hp.cutoff_frequency.unwrap_or(200),
                hp.boost_factor.unwrap_or(1.0),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::HighPass(f));
            }
        }

        if let Some(ref n) = config.normalization {
            let f = normalization::NormalizationFilter::new(
                n.max_amplitude.unwrap_or(1.0),
                n.adaptive.unwrap_or(true),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Normalization(f));
            }
        }

        if let Some(ref c) = config.chorus {
            let f = chorus::ChorusFilter::new(
                c.rate.unwrap_or(1.5),
                c.depth.unwrap_or(1.0),
                c.delay.unwrap_or(2.0),
                c.mix.unwrap_or(0.5),
                c.feedback.unwrap_or(0.5),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Chorus(f));
            }
        }

        if let Some(ref c) = config.compressor {
            let f = compressor::CompressorFilter::new(
                c.threshold.unwrap_or(-10.0),
                c.ratio.unwrap_or(2.0),
                c.attack.unwrap_or(5.0),
                c.release.unwrap_or(50.0),
                c.makeup_gain.unwrap_or(0.0),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Compressor(f));
            }
        }

        if let Some(ref fl) = config.flanger {
            let f = flanger::FlangerFilter::new(
                fl.rate.unwrap_or(0.2),
                fl.depth.unwrap_or(1.0),
                fl.feedback.unwrap_or(0.5),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Flanger(f));
            }
        }

        if let Some(ref p) = config.phaser {
            let f = phaser::PhaserFilter::new(
                p.stages.unwrap_or(4),
                p.rate.unwrap_or(0.0),
                p.depth.unwrap_or(1.0),
                p.feedback.unwrap_or(0.0),
                p.mix.unwrap_or(0.5),
                p.min_frequency.unwrap_or(100.0),
                p.max_frequency.unwrap_or(2500.0),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Phaser(f));
            }
        }

        if let Some(ref ph) = config.phonograph {
            let f = phonograph::PhonographFilter::new(
                ph.frequency.unwrap_or(0.8),
                ph.depth.unwrap_or(0.25),
                ph.crackle.unwrap_or(0.18),
                ph.flutter.unwrap_or(0.18),
                ph.room.unwrap_or(0.22),
                ph.mic_agc.unwrap_or(0.25),
                ph.drive.unwrap_or(0.25),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Phonograph(Box::new(f)));
            }
        }

        if let Some(ref r) = config.reverb {
            let f = reverb::ReverbFilter::new(
                r.mix.unwrap_or(0.0),
                r.room_size.unwrap_or(0.5),
                r.damping.unwrap_or(0.5),
                r.width.unwrap_or(1.0),
            );
            if f.is_enabled() {
                filters.push(ConcreteFilter::Reverb(f));
            }
        }

        if let Some(ref s) = config.spatial {
            let f = spatial::SpatialFilter::new(s.rate.unwrap_or(0.0), s.depth.unwrap_or(0.0));
            if f.is_enabled() {
                filters.push(ConcreteFilter::Spatial(f));
            }
        }

        let timescale = config.timescale.as_ref().and_then(|t| {
            let f = timescale::TimescaleFilter::new(
                t.speed.unwrap_or(1.0),
                t.pitch.unwrap_or(1.0),
                t.rate.unwrap_or(1.0),
            );
            if f.is_enabled() { Some(f) } else { None }
        });

        Self {
            filters,
            timescale,
            timescale_buffer: Vec::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        !self.filters.is_empty() || self.timescale.is_some()
    }

    pub fn process(&mut self, samples: &mut [i16]) {
        for filter in self.filters.iter_mut() {
            filter.process(samples);
        }

        if let Some(ref mut ts) = self.timescale {
            let resampled = ts.process_resample(samples);
            self.timescale_buffer.extend_from_slice(&resampled);

            // B05: Unbounded growth guard — drain oldest (front) to stay within limit.
            // Stereo alignment is preserved by rounding excess to even.
            const MAX_TS_SAMPLES: usize = 1920 * 1024;
            if self.timescale_buffer.len() > MAX_TS_SAMPLES {
                let excess = self.timescale_buffer.len() - MAX_TS_SAMPLES;
                let excess = excess - (excess % 2);
                if excess > 0 {
                    self.timescale_buffer.drain(..excess);
                }
            }
        }
    }

    pub fn fill_frame(&mut self, output: &mut [i16]) -> bool {
        if self.timescale.is_none() {
            return false;
        }

        if self.timescale_buffer.len() >= output.len() {
            output.copy_from_slice(&self.timescale_buffer[..output.len()]);
            self.timescale_buffer.drain(..output.len());
            true
        } else {
            false
        }
    }

    pub fn has_timescale(&self) -> bool {
        self.timescale.is_some()
    }

    pub fn reset(&mut self) {
        for filter in self.filters.iter_mut() {
            filter.reset();
        }
        if let Some(ref mut ts) = self.timescale {
            ts.reset();
        }
        self.timescale_buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::player::state::Filters;

    #[test]
    fn filter_chain_default_is_inactive() {
        let chain = FilterChain::from_config(&Filters::default());
        assert!(!chain.is_active());
        assert!(chain.filters.is_empty());
        assert!(chain.timescale.is_none());
    }

    #[test]
    fn filter_chain_with_volume() {
        let mut filters = Filters::default();
        filters.volume = Some(50.0);
        let chain = FilterChain::from_config(&filters);
        assert!(chain.is_active());
        assert_eq!(chain.filters.len(), 1);
    }

    #[test]
    fn filter_chain_process_passthrough() {
        let mut chain = FilterChain::from_config(&Filters::default());
        let mut samples = [100i16, 200, 300, 400];
        let original = samples;
        chain.process(&mut samples);
        // No filters active, samples unchanged
        assert_eq!(samples, original);
    }

    #[test]
    fn filter_chain_reset() {
        let mut filters = Filters::default();
        filters.volume = Some(100.0);
        let mut chain = FilterChain::from_config(&filters);
        chain.reset();
        // After reset, process should still work
        let mut samples = [100i16, 200];
        chain.process(&mut samples);
    }

    /// B22: re-verify the timescale frame-exchange contract.
    ///
    /// The chain keeps its own `timescale_buffer`: `process()` must NOT swap the caller's
    /// in-place samples for resampled audio; the only way out is `fill_frame`, which must
    /// drain the front of the buffer exactly `output.len()` samples, in order, and must not
    /// consume anything when the buffer is short.
    #[test]
    fn timescale_frame_exchange_contract() {
        use crate::discord::player::state::TimescaleFilter;

        let mut filters = Filters::default();
        filters.timescale = Some(TimescaleFilter {
            speed: Some(1.5),
            pitch: Some(1.0),
            rate: Some(1.0),
        });
        let mut chain = FilterChain::from_config(&filters);
        assert!(chain.is_active());
        assert!(chain.has_timescale());

        let input: Vec<i16> = (0..960).map(|i| ((i * 31) % 2000) as i16 - 1000).collect();
        let mut samples = input.clone();
        chain.process(&mut samples);
        assert_eq!(samples, input, "process() must not replace caller samples in-place");
        assert!(
            !chain.timescale_buffer.is_empty(),
            "speed=1.5 over 960 frames must produce output"
        );

        let produced = chain.timescale_buffer.clone();
        let mut frame8 = [0i16; 8];
        assert!(chain.fill_frame(&mut frame8), "8 samples must be available");
        assert_eq!(&frame8[..], &produced[..8]);

        let mut frame16 = [0i16; 16];
        assert!(chain.fill_frame(&mut frame16), "next 16 must be available");
        assert_eq!(&frame16[..], &produced[8..24], "drain must continue in order");
        assert_eq!(chain.timescale_buffer.len(), produced.len() - 24);

        // Short buffer => no partial drain, returns false.
        let before = chain.timescale_buffer.len();
        let mut too_big = vec![0i16; before + 8];
        assert!(!chain.fill_frame(&mut too_big));
        assert_eq!(chain.timescale_buffer.len(), before, "failed fill must not consume");

        // reset() empties the exchange buffer.
        chain.reset();
        assert!(chain.timescale_buffer.is_empty());
    }

    #[test]
    fn validate_filters_default_all_valid() {
        let filters = Filters::default();
        let config = FiltersConfig::default();
        let invalid = validate_filters(&filters, &config);
        // No filters set, nothing should be invalid
        assert!(invalid.is_empty());
    }
}
