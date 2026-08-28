/// Vibrato filter - frequency modulation via a modulated delay line
/// Read pointer oscillates around a base delay, producing pitch modulation.

#[derive(Debug)]
pub struct Vibrato {
    frequency: f64,
    depth: f64,
    sample_rate: f64,
    phase: f64,
    // Circular delay buffer (stereo interleaved)
    buffer: Vec<f32>,
    write_pos: usize,
    // Base delay in samples and modulation amplitude in samples
    base_delay: f64,
    delay_depth: f64,
}

const MAX_DELAY_SAMPLES: usize = 4096;

impl Vibrato {
    pub fn new(sample_rate: f64) -> Self {
        let base_ms = 5.0;
        let base_delay = base_ms * sample_rate / 1000.0;
        let delay_depth = 2.0 * sample_rate / 1000.0; // +/- 2ms swing
        Self {
            frequency: 2.0,
            depth: 0.5,
            sample_rate,
            phase: 0.0,
            buffer: vec![0.0; MAX_DELAY_SAMPLES * 2],
            write_pos: 0,
            base_delay,
            delay_depth,
        }
    }

    /// Set vibrato frequency (Hz). Lavalink range 0.1..14.
    pub fn set_frequency(&mut self, frequency: f64) {
        self.frequency = frequency.clamp(0.1, 14.0);
    }

    /// Set vibrato depth (0.0..1.0).
    pub fn set_depth(&mut self, depth: f64) {
        self.depth = depth.clamp(0.0, 1.0);
    }

    #[inline]
    fn read_delayed(&self, channel: usize, delay_samples: f64) -> f32 {
        // delay_samples is in frames; buffer is stereo interleaved (stride 2)
        let delay_in_buf = delay_samples * 2.0;
        let read_pos = self.write_pos as f64 - delay_in_buf;
        let len = self.buffer.len() as f64;
        let rp = if read_pos < 0.0 {
            read_pos + len
        } else {
            read_pos
        };

        // Ensure we land on even indices (frame boundaries) before adding channel offset
        let frame_pos = rp / 2.0;
        let frame_idx0 = frame_pos.floor() as usize;
        let frame_idx1 = frame_idx0 + 1;
        let frac = (frame_pos - frame_pos.floor()) as f32;

        let buf_len_frames = self.buffer.len() / 2;
        let idx0 = (frame_idx0 % buf_len_frames) * 2 + channel;
        let idx1 = (frame_idx1 % buf_len_frames) * 2 + channel;

        let s0 = self.buffer[idx0];
        let s1 = self.buffer[idx1];
        s0 + (s1 - s0) * frac
    }

    /// Process stereo interleaved buffer in place.
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for chunk in buffer.chunks_exact_mut(2) {
            let l = chunk[0];
            let r = chunk[1];

            // Write current input into delay line
            self.buffer[self.write_pos] = l;
            self.buffer[self.write_pos + 1] = r;

            // Modulated delay
            let lfo = (2.0 * std::f64::consts::PI * self.phase).sin();
            let delay = self.base_delay + self.delay_depth * self.depth * ((lfo + 1.0) / 2.0);

            chunk[0] = self.read_delayed(0, delay);
            chunk[1] = self.read_delayed(1, delay);

            self.write_pos = (self.write_pos + 2) % self.buffer.len();
            self.phase += self.frequency / self.sample_rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.buffer.iter_mut().for_each(|s| *s = 0.0);
        self.write_pos = 0;
        self.phase = 0.0;
    }
}

impl Default for Vibrato {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vibrato_no_modulation_passthrough_after_delay() {
        let mut vib = Vibrato::new(48000.0);
        vib.set_depth(0.0);

        // Feed DC signal; after the base delay elapses output should equal input
        let input = vec![0.75f32; 48000]; // 1 second of DC
        let mut buf = vec![0.0f32; 96000];
        for i in 0..input.len() {
            buf[i * 2] = input[i];
            buf[i * 2 + 1] = input[i];
        }
        vib.process_buffer(&mut buf);

        // After ~6ms (base delay) the output must be the delayed DC = same value
        let tail = &buf[96000 - 200..];
        for s in tail {
            assert!(
                (s - 0.75).abs() < 1e-3,
                "expected passthrough after delay, got {}",
                s
            );
        }
    }

    #[test]
    fn test_vibrato_modulates_pitch() {
        let mut vib = Vibrato::new(48000.0);
        vib.set_depth(0.9);
        vib.set_frequency(5.0);

        // 440 Hz sine, measure zero-crossing intervals vary => pitch modulation
        let mut buf = Vec::with_capacity(96000 * 2);
        for i in 0..96000 {
            let t = i as f64 / 48000.0;
            let s = (2.0 * std::f64::consts::PI * 440.0 * t).sin() as f32;
            buf.push(s);
            buf.push(s);
        }
        vib.process_buffer(&mut buf);

        // Collect zero crossing intervals on left channel
        let mut intervals = Vec::new();
        let mut last_cross = 0usize;
        for i in 1..buf.len() / 2 {
            let a = buf[(i - 1) * 2];
            let b = buf[i * 2];
            if a <= 0.0 && b > 0.0 {
                intervals.push(i - last_cross);
                last_cross = i;
            }
        }
        assert!(intervals.len() > 20, "not enough cycles detected");
        let min = *intervals.iter().min().unwrap();
        let max = *intervals.iter().max().unwrap();
        assert!(
            max as f32 / min as f32 > 1.05,
            "zero-crossing intervals should vary under vibrato (min={} max={})",
            min,
            max
        );
    }
}
