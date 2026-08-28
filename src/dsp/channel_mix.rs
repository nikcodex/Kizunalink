/// Channel mix filter - mixes left/right channels with configurable coefficients
/// Useful for mono conversion, stereo width control, etc.

#[derive(Debug)]
pub struct ChannelMix {
    left_to_left: f64,
    left_to_right: f64,
    right_to_left: f64,
    right_to_right: f64,
}

impl ChannelMix {
    pub fn new() -> Self {
        Self {
            left_to_left: 1.0,
            left_to_right: 0.0,
            right_to_left: 0.0,
            right_to_right: 1.0,
        }
    }

    pub fn set_left_to_left(&mut self, value: f64) {
        self.left_to_left = value;
    }
    pub fn set_left_to_right(&mut self, value: f64) {
        self.left_to_right = value;
    }
    pub fn set_right_to_left(&mut self, value: f64) {
        self.right_to_left = value;
    }
    pub fn set_right_to_right(&mut self, value: f64) {
        self.right_to_right = value;
    }

    /// Process stereo interleaved buffer
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for chunk in buffer.chunks_mut(2) {
            if chunk.len() == 2 {
                let left = chunk[0] as f64;
                let right = chunk[1] as f64;

                let new_left = left * self.left_to_left + right * self.right_to_left;
                let new_right = left * self.left_to_right + right * self.right_to_right;

                chunk[0] = new_left as f32;
                chunk[1] = new_right as f32;
            }
        }
    }
}

impl Default for ChannelMix {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_mix_passthrough() {
        let mut mix = ChannelMix::new(); // Default is passthrough

        let sample_rate = 48000.0;
        let mut buffer: Vec<f32> = (0..1000)
            .map(|i| {
                let t = i as f64 / sample_rate;
                (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32
            })
            .collect();

        let input = buffer.clone();
        mix.process_buffer(&mut buffer);

        // Default mix should preserve signal
        let max_diff = buffer
            .iter()
            .zip(input.iter())
            .map(|(o, i)| (o - i).abs())
            .fold(0.0f32, f32::max);

        assert!(max_diff < 0.001, "Default mix should passthrough signal");
    }

    #[test]
    fn test_channel_mix_to_mono() {
        let mut mix = ChannelMix::new();
        mix.set_left_to_left(0.5);
        mix.set_left_to_right(0.5);
        mix.set_right_to_left(0.5);
        mix.set_right_to_right(0.5);

        let sample_rate = 48000.0;
        let mut buffer: Vec<f32> = Vec::new();
        for i in 0..1000 {
            let t = i as f64 / sample_rate;
            let left = (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32;
            let right = (2.0 * std::f64::consts::PI * 2000.0 * t).sin() as f32;
            buffer.push(left);
            buffer.push(right);
        }

        mix.process_buffer(&mut buffer);

        // Check that left and right channels are now identical (mono)
        for chunk in buffer.chunks(2) {
            if chunk.len() == 2 {
                assert!(
                    (chunk[0] - chunk[1]).abs() < 0.001,
                    "Mono mix should make channels identical"
                );
            }
        }
    }
}
