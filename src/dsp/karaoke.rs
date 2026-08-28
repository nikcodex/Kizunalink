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
    bp_left: BiquadFilter,
    bp_right: BiquadFilter,
}

impl Karaoke {
    pub fn new(sample_rate: f64) -> Self {
        let mut k = Self {
            level: 1.0,
            mono_level: 1.0,
            filter_band: 220.0,
            filter_width: 100.0,
            sample_rate,
            bp_left: BiquadFilter::new(),
            bp_right: BiquadFilter::new(),
        };
        k.rebuild();
        k
    }

    fn rebuild(&mut self) {
        let band = self.filter_band.clamp(20.0, self.sample_rate / 2.0 * 0.95);
        let width = self.filter_width.max(10.0);
        let q = band / width;

        self.bp_left = BiquadFilter::bandpass(self.sample_rate, band, q);
        self.bp_right = BiquadFilter::bandpass(self.sample_rate, band, q);
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
    ///
    /// Standard karaoke vocal removal: subtract band-passed center content
    /// from each channel, scaled by `level`. `mono_level` blends between
    /// the original signal (0.0) and the center-removed signal (1.0).
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for chunk in buffer.chunks_exact_mut(2) {
            let l = chunk[0] as f64;
            let r = chunk[1] as f64;

            let center = (l + r) / 2.0;

            // Band-pass the center content
            let bp_l = self.bp_left.process(center);
            let bp_r = self.bp_right.process(center);

            // Subtract band-passed center from each channel, scaled by level
            let removed_l = l - bp_l * self.level;
            let removed_r = r - bp_r * self.level;

            // Blend between original and center-removed
            chunk[0] = (l * (1.0 - self.mono_level) + removed_l * self.mono_level) as f32;
            chunk[1] = (r * (1.0 - self.mono_level) + removed_r * self.mono_level) as f32;
        }
    }

    /// Reset filter states.
    pub fn reset(&mut self) {
        self.bp_left.reset();
        self.bp_right.reset();
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
