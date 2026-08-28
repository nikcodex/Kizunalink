/// Tremolo filter - amplitude modulation at low frequency

#[derive(Debug)]
pub struct Tremolo {
    frequency: f64,
    depth: f64,
    sample_rate: f64,
    phase: f64,
}

impl Tremolo {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            frequency: 2.0,
            depth: 0.5,
            sample_rate,
            phase: 0.0,
        }
    }

    /// Set tremolo frequency (Hz)
    pub fn set_frequency(&mut self, frequency: f64) {
        self.frequency = frequency.clamp(0.1, 20.0);
    }

    /// Set tremolo depth (0.0 to 1.0)
    pub fn set_depth(&mut self, depth: f64) {
        self.depth = depth.clamp(0.0, 1.0);
    }

    /// Process a single sample
    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        let modulation = 1.0 - self.depth * (self.phase * 2.0 * std::f64::consts::PI).sin();
        self.phase += self.frequency / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        input * modulation
    }

    /// Process stereo interleaved buffer
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for chunk in buffer.chunks_exact_mut(2) {
            let modulation = 1.0 - self.depth * (self.phase * 2.0 * std::f64::consts::PI).sin();
            let mod_f32 = modulation as f32;
            chunk[0] *= mod_f32;
            chunk[1] *= mod_f32;
            self.phase += self.frequency / self.sample_rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }

    /// Reset filter state
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}

impl Default for Tremolo {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tremolo_no_modulation() {
        let mut trem = Tremolo::new(48000.0);
        trem.set_depth(0.0); // No modulation

        let sample_rate = 48000.0;
        let input: Vec<f32> = (0..1000)
            .map(|i| {
                let t = i as f64 / sample_rate;
                (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32
            })
            .collect();

        let mut output = input.clone();
        trem.process_buffer(&mut output);

        // With depth=0, output should match input
        for (i, (o, e)) in output.iter().zip(input.iter()).enumerate() {
            assert!(
                (o - e).abs() < 0.001,
                "Sample {} differs: {} vs {}",
                i,
                o,
                e
            );
        }
    }

    #[test]
    fn test_tremolo_modulation() {
        let mut trem = Tremolo::new(48000.0);
        trem.set_depth(0.5);
        trem.set_frequency(2.0);

        let sample_rate = 48000.0;
        let input: Vec<f32> = vec![1.0; 1000]; // DC signal

        let mut output = input;
        trem.process_buffer(&mut output);

        // Output should vary (not all 1.0)
        let max = output.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let min = output.iter().cloned().fold(f32::INFINITY, f32::min);

        assert!(max > min, "Tremolo should create amplitude variation");
        assert!(max <= 1.0, "Output should not exceed input level");
    }
}
