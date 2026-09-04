/// 15-band equalizer implementation
/// Each band is a peaking EQ filter at specific frequencies
use super::biquad::BiquadFilter;

const NUM_BANDS: usize = 15;

// Standard 15-band EQ frequencies (in Hz) matching lavaplayer
const BAND_FREQUENCIES: [f64; NUM_BANDS] = [
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0,
    10000.0, 16000.0,
];

#[derive(Debug)]
pub struct Equalizer {
    bands_l: [BiquadFilter; NUM_BANDS],
    bands_r: [BiquadFilter; NUM_BANDS],
    gains: [f32; NUM_BANDS],
    sample_rate: f64,
}

impl Equalizer {
    pub fn new(sample_rate: f64) -> Self {
        let mut bands_l = std::array::from_fn(|_| BiquadFilter::new());
        let mut bands_r = std::array::from_fn(|_| BiquadFilter::new());

        for (i, (bl, br)) in bands_l.iter_mut().zip(bands_r.iter_mut()).enumerate() {
            *bl = BiquadFilter::peaking_eq(sample_rate, BAND_FREQUENCIES[i], 0.707, 0.0);
            *br = BiquadFilter::peaking_eq(sample_rate, BAND_FREQUENCIES[i], 0.707, 0.0);
        }

        Self {
            bands_l,
            bands_r,
            gains: [0.0; NUM_BANDS],
            sample_rate,
        }
    }

    /// Set gain for a specific band (0-14)
    /// gain_db: -0.5 to 1.0 (matching lavaplayer's range)
    pub fn set_band(&mut self, band: usize, gain_db: f32) {
        if band < NUM_BANDS {
            self.gains[band] = gain_db;
            self.bands_l[band].update_peaking_eq(
                self.sample_rate,
                BAND_FREQUENCIES[band],
                0.707,
                gain_db as f64,
            );
            self.bands_r[band].update_peaking_eq(
                self.sample_rate,
                BAND_FREQUENCIES[band],
                0.707,
                gain_db as f64,
            );
        }
    }

    /// Set gains for all bands from a slice
    pub fn set_gains(&mut self, gains: &[f32]) {
        for (i, &gain) in gains.iter().enumerate().take(NUM_BANDS) {
            self.set_band(i, gain);
        }
    }

    /// Process a single sample (mono, uses left channel filters)
    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        let mut output = input;
        for band in &mut self.bands_l {
            output = band.process(output);
        }
        output
    }

    /// Process a buffer of interleaved stereo samples
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for chunk in buffer.as_chunks_mut::<2>().0 {
            let mut left = chunk[0] as f64;
            let mut right = chunk[1] as f64;
            for (bl, br) in self.bands_l.iter_mut().zip(self.bands_r.iter_mut()) {
                left = bl.process(left);
                right = br.process(right);
            }
            chunk[0] = left as f32;
            chunk[1] = right as f32;
        }
    }

    /// Reset all filter states
    pub fn reset(&mut self) {
        for (bl, br) in self.bands_l.iter_mut().zip(self.bands_r.iter_mut()) {
            bl.reset();
            br.reset();
        }
    }
}

impl Default for Equalizer {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eq_default_no_change() {
        let mut eq = Equalizer::new(48000.0);

        // Generate a test signal
        let sample_rate = 48000.0;
        let mut output_sum = 0.0;
        let input_sum = (0..1000)
            .map(|i| {
                let t = i as f64 / sample_rate;
                (2.0 * std::f64::consts::PI * 1000.0 * t).sin()
            })
            .sum::<f64>();

        for i in 0..1000 {
            let t = i as f64 / sample_rate;
            let input = (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
            let output = eq.process(input);
            output_sum += output;
        }

        // With default gains (0 dB), output should be close to input
        let ratio = output_sum / input_sum;
        assert!(
            ratio > 0.9 && ratio < 1.1,
            "Default EQ should not significantly change signal"
        );
    }

    #[test]
    fn test_eq_boost_band() {
        let mut eq = Equalizer::new(48000.0);

        // Boost 1kHz band
        eq.set_band(8, 1.0); // 1kHz is band 8

        // Generate 1kHz signal
        let sample_rate = 48000.0;
        let mut input_rms = 0.0;
        let mut output_rms = 0.0;

        for i in 0..1000 {
            let t = i as f64 / sample_rate;
            let input = (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
            let output = eq.process(input);
            input_rms += input * input;
            output_rms += output * output;
        }

        input_rms = (input_rms / 1000.0).sqrt();
        output_rms = (output_rms / 1000.0).sqrt();

        // Output should be louder than input
        assert!(
            output_rms > input_rms,
            "Boosted band should increase signal level"
        );
    }
}
