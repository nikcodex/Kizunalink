/// Rotation filter - rotates the stereo field using an amplitude-preserving
/// rotation matrix, matching lavaplayer's RotationFilter:
///   L' = L*cos(a) - R*sin(a)
///   R' = R*cos(a) + L*sin(a)

#[derive(Debug)]
pub struct Rotation {
    rotation_hz: f64,
    sample_rate: f64,
    phase: f64,
}

impl Rotation {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            rotation_hz: 0.0,
            sample_rate,
            phase: 0.0,
        }
    }

    /// Set rotation speed in Hz. 0 disables.
    pub fn set_rotation_hz(&mut self, hz: f64) {
        if !hz.is_finite() || hz <= 0.0 {
            self.rotation_hz = 0.0;
            return;
        }
        self.rotation_hz = hz.clamp(0.0, 10.0);
    }

    /// Process stereo interleaved buffer in place.
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        if self.rotation_hz <= 0.0
            || self.rotation_hz.abs() < 1e-6
            || !self.sample_rate.is_finite()
            || self.sample_rate <= 0.0
        {
            return;
        }
        for chunk in buffer.as_chunks_mut::<2>().0 {
            let l = chunk[0] as f64;
            let r = chunk[1] as f64;

            let angle = 2.0 * std::f64::consts::PI * self.phase;
            let (sin_a, cos_a) = angle.sin_cos();

            let new_l = l * cos_a - r * sin_a;
            let new_r = r * cos_a + l * sin_a;

            chunk[0] = new_l as f32;
            chunk[1] = new_r as f32;

            self.phase += self.rotation_hz / self.sample_rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}

impl Default for Rotation {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rotation_preserves_energy() {
        let mut rot = Rotation::new(48000.0);
        rot.set_rotation_hz(1.0);

        // Hard-panned signal: only left channel active
        let mut buf = Vec::new();
        for i in 0..96000 {
            let t = i as f64 / 48000.0;
            let s = (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32;
            buf.push(s);
            buf.push(0.0);
        }

        let input_energy: f32 = buf.iter().map(|s| s * s).sum();
        rot.process_buffer(&mut buf);
        let output_energy: f32 = buf.iter().map(|s| s * s).sum();

        let ratio = output_energy / input_energy;
        assert!(
            (ratio - 1.0).abs() < 0.01,
            "rotation must preserve total energy, got ratio {}",
            ratio
        );
    }

    #[test]
    fn test_rotation_swaps_channels_over_time() {
        let mut rot = Rotation::new(48000.0);
        rot.set_rotation_hz(0.5); // full cycle every 2 seconds

        let mut buf = Vec::new();
        for i in 0..96000 {
            let t = i as f64 / 48000.0;
            let s = 0.8f32; // DC on left only
            let _ = t;
            buf.push(s);
            buf.push(0.0);
        }
        rot.process_buffer(&mut buf);

        // At phase ~0.25s of a 2s cycle => 45 degrees => both channels ~0.56
        let mid_left = buf[12000 * 2];
        let mid_right = buf[12000 * 2 + 1];
        assert!(
            mid_left.abs() > 0.3 && mid_right.abs() > 0.3,
            "at quarter cycle energy should be split across channels (L={} R={})",
            mid_left,
            mid_right
        );
    }

    #[test]
    fn test_rotation_zero_hz_is_noop() {
        let mut rot = Rotation::new(48000.0);
        rot.set_rotation_hz(0.0);

        let mut buf = vec![0.5f32; 200];
        let input = buf.clone();
        rot.process_buffer(&mut buf);
        assert_eq!(buf, input, "0 Hz rotation should not modify samples");
    }

    #[test]
    fn test_rotation_near_zero_hz_is_noop() {
        let mut rot = Rotation::new(48000.0);
        rot.set_rotation_hz(1e-7);

        let mut buf = vec![0.5f32; 200];
        let input = buf.clone();
        rot.process_buffer(&mut buf);
        assert_eq!(
            buf, input,
            "Near-zero Hz rotation should not modify samples"
        );
    }

    #[test]
    fn test_rotation_handles_nan_and_inf() {
        let mut rot = Rotation::new(48000.0);
        rot.set_rotation_hz(f64::NAN);
        assert_eq!(rot.rotation_hz, 0.0);

        rot.set_rotation_hz(f64::INFINITY);
        assert_eq!(rot.rotation_hz, 0.0);

        rot.set_rotation_hz(-5.0);
        assert_eq!(rot.rotation_hz, 0.0);
    }

    #[test]
    fn test_rotation_odd_length_buffer() {
        let mut rot = Rotation::new(48000.0);
        rot.set_rotation_hz(1.0);
        let mut buf = vec![0.5f32; 199];
        rot.process_buffer(&mut buf);
        assert_eq!(buf.len(), 199);
    }
}
