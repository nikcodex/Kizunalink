// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use super::{AudioFilter, lfo::Lfo};

/// Tremolo filter.
pub struct TremoloFilter {
    lfo: Lfo,
}

impl TremoloFilter {
    pub fn new(frequency: f32, depth: f32) -> Self {
        let mut lfo = Lfo::new();
        let depth = depth.clamp(0.0, 1.0);
        lfo.update(frequency as f64, depth as f64);
        Self { lfo }
    }
}

impl AudioFilter for TremoloFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.lfo.depth == 0.0 || self.lfo.frequency == 0.0 {
            return;
        }

        // Process per-frame (both L and R get the same multiplier per frame)
        for chunk in samples.as_chunks_mut::<2>().0 {
            let multiplier = self.lfo.process();

            let left = (chunk[0] as f64 * multiplier) as i32;
            chunk[0] = left.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

            let right = (chunk[1] as f64 * multiplier) as i32;
            chunk[1] = right.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.lfo.depth > 0.0 && self.lfo.frequency > 0.0
    }

    fn reset(&mut self) {
        self.lfo.reset();
    }
}
