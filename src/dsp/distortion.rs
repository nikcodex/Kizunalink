/// Distortion filter - waveshaping via mixed sin/cos/tan transfer functions.
/// Matches lavaplayer's DistortionFilter parameter set.
/// tan() arguments are wrapped into (-pi/2, pi/2) to avoid asymptote blowups.

#[derive(Debug)]
pub struct Distortion {
    sin_offset: f64,
    sin_scale: f64,
    cos_offset: f64,
    cos_scale: f64,
    tan_offset: f64,
    tan_scale: f64,
    offset: f64,
    scale: f64,
}

impl Distortion {
    pub fn new() -> Self {
        Self {
            sin_offset: 0.0,
            sin_scale: 1.0,
            cos_offset: 0.0,
            cos_scale: 1.0,
            tan_offset: 0.0,
            tan_scale: 1.0,
            offset: 0.0,
            scale: 1.0,
        }
    }

    pub fn set_sin_offset(&mut self, v: f64) {
        self.sin_offset = v;
    }
    pub fn set_sin_scale(&mut self, v: f64) {
        self.sin_scale = v;
    }
    pub fn set_cos_offset(&mut self, v: f64) {
        self.cos_offset = v;
    }
    pub fn set_cos_scale(&mut self, v: f64) {
        self.cos_scale = v;
    }
    pub fn set_tan_offset(&mut self, v: f64) {
        self.tan_offset = v;
    }
    pub fn set_tan_scale(&mut self, v: f64) {
        self.tan_scale = v;
    }
    pub fn set_offset(&mut self, v: f64) {
        self.offset = v;
    }
    pub fn set_scale(&mut self, v: f64) {
        self.scale = v;
    }

    /// Wrap x into (-pi/2, pi/2) so tan stays finite.
    #[inline]
    fn wrapped_tan(x: f64) -> f64 {
        let half_pi = std::f64::consts::FRAC_PI_2;
        let period = std::f64::consts::PI;
        let mut m = (x + half_pi).rem_euclid(period) - half_pi;
        // keep strictly inside to avoid tan(inf) at exact boundary
        if m >= half_pi {
            m -= period;
        }
        if m <= -half_pi {
            m += period;
        }
        m.tan()
    }

    #[inline]
    fn unit(x: f64, scale: f64, offset: f64) -> f64 {
        // normalize output of each shaping function into roughly [-1, 1]
        (x * scale + offset).clamp(-4.0, 4.0) / 4.0
    }

    /// Process a single sample.
    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        let x = input * self.scale + self.offset;

        let sin_part = Self::unit(x.sin(), self.sin_scale, self.sin_offset);
        let cos_part = Self::unit(x.cos(), self.cos_scale, self.cos_offset);
        let tan_part = Self::unit(Self::wrapped_tan(x), self.tan_scale, self.tan_offset);

        (sin_part + cos_part + tan_part) / 3.0
    }

    /// Process stereo interleaved buffer in place.
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process(*sample as f64) as f32;
        }
    }
}

impl Default for Distortion {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrapped_tan_stays_finite() {
        for i in -10000..10000 {
            let x = i as f64 / 1000.0;
            let t = Distortion::wrapped_tan(x);
            assert!(t.is_finite(), "wrapped_tan({}) must be finite", x);
            assert!(
                t.abs() < 1e6,
                "wrapped_tan({}) should not explode near asymptotes",
                x
            );
        }
    }

    #[test]
    fn test_default_params_output_bounded() {
        let mut dist = Distortion::new();

        let mut buf: Vec<f32> = (0..48000)
            .map(|i| {
                let t = i as f64 / 48000.0;
                (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32
            })
            .collect();
        dist.process_buffer(&mut buf);

        assert!(
            buf.iter().all(|s| s.is_finite() && s.abs() <= 2.0),
            "distortion output must stay bounded"
        );
    }

    #[test]
    fn test_distortion_adds_harmonics() {
        let mut dist = Distortion::new();
        dist.set_scale(8.0); // heavy drive

        // Pure 1kHz sine in, harmonic energy at 2k/3k should appear
        let mut buf: Vec<f32> = (0..48000)
            .map(|i| {
                let t = i as f64 / 48000.0;
                0.9 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32
            })
            .collect();
        dist.process_buffer(&mut buf);

        let goertzel_at = |freq: f64| -> f32 {
            let n = buf.len() as f64;
            let k = 2.0 * std::f64::consts::PI * freq / 48000.0;
            let (mut s_prev, mut s_prev2) = (0.0f64, 0.0f64);
            for &s in &buf {
                let s0 = s as f64 + 2.0 * k.cos() * s_prev - s_prev2;
                s_prev2 = s_prev;
                s_prev = s0;
            }
            let coeff = k.cos();
            ((s_prev2.powi(2) + s_prev.powi(2) - 2.0 * coeff * s_prev * s_prev2) / n).sqrt() as f32
        };

        let fundamental = goertzel_at(1000.0);
        let h3 = goertzel_at(3000.0);
        assert!(
            h3 > fundamental * 0.01 && h3 > 0.001,
            "expected 3rd harmonic from distortion (fund={} h3={})",
            fundamental,
            h3
        );
    }
}
