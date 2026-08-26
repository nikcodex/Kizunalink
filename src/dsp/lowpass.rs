/// Low-pass filter - attenuates high frequency content.
/// `smoothing` maps to cutoff: cutoff = sample_rate / (2 * smoothing),
/// so higher smoothing = duller sound (lavaplayer-style behavior).

use super::biquad::BiquadFilter;

#[derive(Debug)]
pub struct LowPass {
    filter_l: BiquadFilter,
    filter_r: BiquadFilter,
    smoothing: f64,
    sample_rate: f64,
}

impl LowPass {
    pub fn new(sample_rate: f64) -> Self {
        let mut lp = Self {
            filter_l: BiquadFilter::new(),
            filter_r: BiquadFilter::new(),
            smoothing: 20.0,
            sample_rate,
        };
        lp.rebuild();
        lp
    }

    fn rebuild(&mut self) {
        let cutoff = (self.sample_rate / (2.0 * self.smoothing))
            .clamp(50.0, self.sample_rate / 2.0 * 0.95);
        self.filter_l = BiquadFilter::lowpass(self.sample_rate, cutoff, 0.707);
        self.filter_r = BiquadFilter::lowpass(self.sample_rate, cutoff, 0.707);
    }

    /// Set smoothing factor. Lavalink range 1..=100.
    pub fn set_smoothing(&mut self, smoothing: f64) {
        self.smoothing = smoothing.clamp(1.0, 100.0);
        self.rebuild();
    }

    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        self.filter_l.process(input)
    }

    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for chunk in buffer.chunks_mut(2) {
            chunk[0] = self.filter_l.process(chunk[0] as f64) as f32;
            if chunk.len() == 2 {
                chunk[1] = self.filter_r.process(chunk[1] as f64) as f32;
            }
        }
    }

    pub fn reset(&mut self) {
        self.filter_l.reset();
        self.filter_r.reset();
    }
}

impl Default for LowPass {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_energy(buf: &[f32], freq: f64, sample_rate: f64) -> f32 {
        let n = buf.len() as f64;
        let k = 2.0 * std::f64::consts::PI * freq / sample_rate;
        let (mut s_prev, mut s_prev2) = (0.0f64, 0.0f64);
        for &s in buf {
            let s0 = s as f64 + 2.0 * k.cos() * s_prev - s_prev2;
            s_prev2 = s_prev;
            s_prev = s0;
        }
        let coeff = k.cos();
        ((s_prev2.powi(2) + s_prev.powi(2) - 2.0 * coeff * s_prev * s_prev2) / n).sqrt() as f32
    }

    #[test]
    fn test_lowpass_attenuates_high_keeps_low() {
        let mut lp = LowPass::new(48000.0);
        lp.set_smoothing(20.0); // cutoff ~1200 Hz

        // Mixed 100 Hz + 10 kHz
        let mut buf: Vec<f32> = (0..48000)
            .map(|i| {
                let t = i as f64 / 48000.0;
                let low = (2.0 * std::f64::consts::PI * 100.0 * t).sin();
                let high = (2.0 * std::f64::consts::PI * 10000.0 * t).sin();
                (low + high) as f32 / 2.0
            })
            .collect();

        lp.process_buffer(&mut buf);

        let low_amp = tone_energy(&buf, 100.0, 48000.0);
        let high_amp = tone_energy(&buf, 10000.0, 48000.0);

        assert!(low_amp > 0.4, "low freq should pass through (amp={})", low_amp);
        assert!(
            high_amp < low_amp * 0.25,
            "high freq should be strongly attenuated (high={} low={})",
            high_amp,
            low_amp
        );
    }
}
