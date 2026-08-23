/// Karaoke filter - attenuates band-limited center-channel content.
/// Center = (L+R)/2 is band-passed around filterBand with the given width,
/// then subtracted from both channels scaled by `level`.

use super::biquad::BiquadFilter;

#[derive(Debug)]
pub struct Karaoke {
    level: f64,
    mono_level: f64,
    filter_band: f64,
    filter_width: f64,

    sample_rate: f64,
    hp_left: BiquadFilter,
    lp_left: BiquadFilter,
    hp_right: BiquadFilter,
    lp_right: BiquadFilter,
}

impl Karaoke {
    pub fn new(sample_rate: f64) -> Self {
        let mut k = Self {
            level: 1.0,
            mono_level: 1.0,
            filter_band: 220.0,
            filter_width: 100.0,
            sample_rate,
            hp_left: BiquadFilter::new(),
            lp_left: BiquadFilter::new(),
            hp_right: BiquadFilter::new(),
            lp_right: BiquadFilter::new(),
        };
        k.rebuild();
        k
    }

    fn rebuild(&mut self) {
        // Band-pass = highpass(band - width/2) cascaded with lowpass(band + width/2)
        let lo = (self.filter_band - self.filter_width / 2.0).max(20.0);
        let hi = self.filter_band + self.filter_width / 2.0;
        let nyquist = self.sample_rate / 2.0;
        let hi = hi.min(nyquist * 0.95);

        self.hp_left = BiquadFilter::highpass(self.sample_rate, lo, 0.707);
        self.lp_left = BiquadFilter::lowpass(self.sample_rate, hi, 0.707);
        self.hp_right = BiquadFilter::highpass(self.sample_rate, lo, 0.707);
        self.lp_right = BiquadFilter::lowpass(self.sample_rate, hi, 0.707);
    }

    pub fn set_level(&mut self, level: f64) {
        self.level = level.clamp(0.0, 1.0);
    }

    pub fn set_mono_level(&mut self, mono_level: f64) {
        self.mono_level = mono_level.clamp(0.0, 1.0);
    }

    pub fn set_filter_band(&mut self, band: f64) {
        self.filter_band = band.clamp(20.0, 20000.0);
        self.rebuild();
    }

    pub fn set_filter_width(&mut self, width: f64) {
        self.filter_width = width.max(0.0);
        self.rebuild();
    }

    /// Process stereo interleaved buffer in place.
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for chunk in buffer.chunks_exact_mut(2) {
            let l = chunk[0] as f64;
            let r = chunk[1] as f64;

            let center = (l + r) / 2.0;

            // Band-passed center on each channel path (keeps per-channel phase)
            let bp_l = self.lp_left.process(self.hp_left.process(center));
            let bp_r = self.lp_right.process(self.hp_right.process(center));

            chunk[0] =
                (l * (1.0 - self.mono_level) + bp_l + (center - bp_l) * self.level) as f32;
            chunk[1] =
                (r * (1.0 - self.mono_level) + bp_r + (center - bp_r) * self.level) as f32;
        }
    }

    /// Reset filter states.
    pub fn reset(&mut self) {
        self.hp_left.reset();
        self.lp_left.reset();
        self.hp_right.reset();
        self.lp_right.reset();
    }
}

impl Default for Karaoke {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_karaoke_attenuates_center_content() {
        let mut karaoke = Karaoke::new(48000.0);
        karaoke.set_level(1.0);

        // Identical 220 Hz tone in both channels => fully centered
        let mut buf = Vec::new();
        for i in 0..48000 {
            let t = i as f64 / 48000.0;
            let s = (2.0 * std::f64::consts::PI * 220.0 * t).sin() as f32;
            buf.push(s);
            buf.push(s);
        }

        let input_energy: f32 = buf.iter().map(|s| s * s).sum();
        karaoke.process_buffer(&mut buf);
        let output_energy: f32 = buf.iter().map(|s| s * s).sum();

        assert!(
            output_energy < input_energy * 0.5,
            "karaoke should strongly attenuate centered content (in={} out={})",
            input_energy,
            output_energy
        );
    }

    #[test]
    fn test_karaoke_preserves_stereo_content() {
        let mut karaoke = Karaoke::new(48000.0);
        karaoke.set_level(1.0);

        // Anti-phase content (L = -R) has zero center; must pass through untouched
        let mut buf = Vec::new();
        for i in 0..48000 {
            let t = i as f64 / 48000.0;
            let s = (2.0 * std::f64::consts::PI * 440.0 * t).sin() as f32;
            buf.push(s);
            buf.push(-s);
        }

        let input_energy: f32 = buf.iter().map(|s| s * s).sum();
        karaoke.process_buffer(&mut buf);
        let output_energy: f32 = buf.iter().map(|s| s * s).sum();

        let ratio = output_energy / input_energy;
        assert!(
            ratio > 0.9,
            "anti-phase (side) content should be preserved, energy ratio {}",
            ratio
        );
    }
}
