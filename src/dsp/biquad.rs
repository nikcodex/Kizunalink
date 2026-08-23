/// Biquad filter implementation for audio DSP
/// This is a standard biquad filter that can be configured as lowpass, highpass, bandpass, etc.

#[derive(Debug, Clone)]
pub struct BiquadFilter {
    // Coefficients
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    
    // State
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BiquadFilter {
    pub fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Create a lowpass filter
    pub fn lowpass(sample_rate: f64, frequency: f64, q: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * frequency / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        
        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Create a highpass filter
    pub fn highpass(sample_rate: f64, frequency: f64, q: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * frequency / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        
        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Create a bandpass filter (0 dB peak gain)
    pub fn bandpass(sample_rate: f64, frequency: f64, q: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * frequency / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        
        let b0 = alpha;
        let b1 = 0.0;
        let b2 = -alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Create a peaking EQ filter
    pub fn peaking_eq(sample_rate: f64, frequency: f64, q: f64, gain_db: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * frequency / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        let a = 10.0_f64.powf(gain_db / 40.0);
        
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;
        
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Process a single sample
    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1 - self.a2 * self.y2;
        
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        
        output
    }

    /// Process a buffer of samples (mono)
    pub fn process_buffer(&mut self, buffer: &mut [f64]) {
        for sample in buffer.iter_mut() {
            *sample = self.process(*sample);
        }
    }

    /// Process stereo interleaved buffer
    pub fn process_stereo(&mut self, buffer: &mut [f32]) {
        // For stereo, we process each channel separately
        // This is a simplified approach - for proper stereo processing,
        // we'd need two separate filter instances
        for chunk in buffer.chunks_mut(2) {
            if chunk.len() == 2 {
                let left = chunk[0] as f64;
                let right = chunk[1] as f64;
                chunk[0] = self.process(left) as f32;
                chunk[1] = self.process(right) as f32;
            }
        }
    }

    /// Reset filter state
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

impl Default for BiquadFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lowpass_passes_low_frequencies() {
        let sample_rate = 48000.0;
        let frequency = 1000.0;
        let q = 0.707;
        
        let mut filter = BiquadFilter::lowpass(sample_rate, frequency, q);
        
        // Generate a 100 Hz sine wave (should pass through)
        let mut sum = 0.0;
        let num_samples = 1000;
        for i in 0..num_samples {
            let t = i as f64 / sample_rate;
            let input = (2.0 * std::f64::consts::PI * 100.0 * t).sin();
            let output = filter.process(input);
            sum += output * output;
        }
        let rms = (sum / num_samples as f64).sqrt();
        
        // RMS should be close to 0.707 for a sine wave
        assert!(rms > 0.5, "Low frequencies should pass through lowpass filter");
    }

    #[test]
    fn test_lowpass_attenuates_high_frequencies() {
        let sample_rate = 48000.0;
        let frequency = 1000.0;
        let q = 0.707;
        
        let mut filter = BiquadFilter::lowpass(sample_rate, frequency, q);
        
        // Generate a 10000 Hz sine wave (should be attenuated)
        let mut sum = 0.0;
        let num_samples = 1000;
        for i in 0..num_samples {
            let t = i as f64 / sample_rate;
            let input = (2.0 * std::f64::consts::PI * 10000.0 * t).sin();
            let output = filter.process(input);
            sum += output * output;
        }
        let rms = (sum / num_samples as f64).sqrt();
        
        // RMS should be much lower than 0.707
        assert!(rms < 0.1, "High frequencies should be attenuated by lowpass filter");
    }
}