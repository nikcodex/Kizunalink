/// WSOLA (Waveform Similarity Overlap-Add) time stretching.
///
/// Changes tempo (duration) WITHOUT changing pitch - this is what
/// lavaplayer's `rate` parameter does. Combined with resampling it also
/// gives us pitch shifting (`pitch`).
///
/// Algorithm: analysis frames are placed at synthesis hop Hs; for each new
/// frame we search a small window (delta) around the natural continuation
/// point for the offset whose overlap region best correlates with what we
/// just emitted, then Hann-crossfade it in. This avoids phasing artifacts
/// of plain OLA.

#[derive(Debug)]
pub struct Wsola {
    sample_rate: f64,
    frame_len: usize,
    overlap: usize,
    delta: usize,
    // stretch factor: output_len = input_len / alpha (alpha > 1 => faster)
    alpha: f64,

    hann: Vec<f32>,
    // pending input samples per channel (stereo deinterleaved)
    in_buf: [Vec<f32>; 2],
    // next analysis position (absolute index into consumed stream)
    next_analysis_start: usize,
    total_input_consumed: usize,
    // last emitted overlap tail used as correlation reference
    ref_tail: [Vec<f32>; 2],
    // finished output frames not yet drained
    out_buf: [Vec<f32>; 2],
}

impl Wsola {
    pub fn new(sample_rate: f64) -> Self {
        let frame_len = (sample_rate as usize * 30) / 1000; // 30 ms
        let overlap = frame_len / 2;
        let delta = (sample_rate as usize * 5) / 1000; // +/-5 ms search

        let mut hann = vec![0.0f32; frame_len];
        if frame_len > 1 {
            for (i, h) in hann.iter_mut().enumerate() {
                let x = i as f32 / (frame_len - 1) as f32;
                *h = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * x).cos();
            }
        }

        Self {
            sample_rate,
            frame_len,
            overlap,
            delta,
            alpha: 1.0,
            hann,
            in_buf: [Vec::new(), Vec::new()],
            next_analysis_start: 0,
            total_input_consumed: 0,
            ref_tail: [Vec::new(), Vec::new()],
            out_buf: [Vec::new(), Vec::new()],
        }
    }

    /// Set stretch factor alpha (>1 faster/shorter, <1 slower/longer).
    pub fn set_alpha(&mut self, alpha: f64) {
        self.alpha = alpha.clamp(0.25, 4.0);
    }

    pub fn reset(&mut self) {
        self.in_buf = [Vec::new(), Vec::new()];
        self.out_buf = [Vec::new(), Vec::new()];
        self.ref_tail = [Vec::new(), Vec::new()];
        self.next_analysis_start = 0;
        self.total_input_consumed = 0;
    }

    /// Feed stereo interleaved input.
    pub fn push(&mut self, interleaved: &[f32]) {
        for chunk in interleaved.chunks_exact(2) {
            self.in_buf[0].push(chunk[0]);
            self.in_buf[1].push(chunk[1]);
        }
        self.total_input_consumed += interleaved.len() / 2;
        self.process_available(false);
    }

    /// Signal end-of-stream so buffered tail is flushed.
    pub fn flush(&mut self) {
        self.process_available(true);

        // If there's remaining unprocessed input shorter than a frame, emit it
        // time-scaled approximately by copying with linear resample.
        let start = self.next_analysis_start.min(self.in_buf[0].len());
        let remaining = self.in_buf[0].len().saturating_sub(start);
        if remaining > 0 && self.alpha != 1.0 {
            let target = ((remaining as f64) / self.alpha) as usize;
            for i in 0..target {
                let pos = start as f64 + i as f64 * self.alpha;
                let idx = (pos as usize).min(self.in_buf[0].len() - 1);
                self.out_buf[0].push(self.in_buf[0][idx]);
                self.out_buf[1].push(self.in_buf[1][idx]);
            }
        }
        self.next_analysis_start = self.in_buf[0].len();
    }

    /// Drain processed output into `dst` (interleaved). Returns frames written.
    pub fn drain(&mut self, dst: &mut Vec<f32>) -> usize {
        let frames = self.out_buf[0].len();
        for i in 0..frames {
            dst.push(self.out_buf[0][i]);
            dst.push(self.out_buf[1][i]);
        }
        self.out_buf[0].clear();
        self.out_buf[1].clear();
        frames
    }

    fn process_available(&mut self, flush: bool) {
        let alpha = self.alpha;
        if alpha == 1.0 {
            // passthrough: move everything through
            let n = self.in_buf[0]
                .len()
                .saturating_sub(self.next_analysis_start);
            for i in 0..n {
                let s = self.next_analysis_start + i;
                self.out_buf[0].push(self.in_buf[0][s]);
                self.out_buf[1].push(self.in_buf[1][s]);
            }
            self.ref_tail = if self.frame_len <= self.in_buf[0].len() {
                let start = self.in_buf[0].len() - self.overlap;
                [
                    self.in_buf[0][start..].to_vec(),
                    self.in_buf[1][start..].to_vec(),
                ]
            } else {
                [Vec::new(), Vec::new()]
            };
            self.next_analysis_start = self.in_buf[0].len();
            return;
        }

        loop {
            // Synthesis hop is fixed at overlap; analysis hop ~ overlap*alpha.
            let analysis_hop = (self.overlap as f64 * alpha).round() as i64;

            // We need input up to candidate_end = analysis_start + frame_len + delta
            let need_end = (self.next_analysis_start + self.frame_len + self.delta) as i64;
            let have = self.in_buf[0].len() as i64;

            if have < need_end {
                if flush && have >= self.next_analysis_start as i64 + self.overlap as i64 {
                    // allow processing with truncated search near EOF
                } else {
                    break;
                }
            }

            let analysis_start = self.next_analysis_start as usize;

            // Choose segment offset via cross-correlation against ref_tail
            let seg_start = self.find_best_offset(analysis_start, analysis_hop);

            if self.ref_tail[0].is_empty() {
                // First frame: emit first `overlap` samples, keep next `overlap` as ref_tail
                let end = (seg_start + self.overlap).min(self.in_buf[0].len());
                if end <= seg_start {
                    break;
                }
                for ch in 0..2 {
                    for &s in &self.in_buf[ch][seg_start..end] {
                        self.out_buf[ch].push(s);
                    }
                }

                let tail_start = end;
                let tail_end = (tail_start + self.overlap).min(self.in_buf[0].len());
                self.ref_tail[0] = self.in_buf[0][tail_start..tail_end].to_vec();
                self.ref_tail[1] = self.in_buf[1][tail_start..tail_end].to_vec();
            } else {
                // Crossfade `overlap` region between ref_tail and new segment head
                let ol = self.overlap.min(self.ref_tail[0].len());
                let seg_end_limit = (seg_start + ol).min(self.in_buf[0].len());
                if seg_start + ol > seg_end_limit {
                    break;
                }

                for i in 0..ol {
                    // Linear crossfade (simpler and correct: always sums to 1.0)
                    let w = (i as f32 + 0.5) / ol as f32;
                    let inv_w = 1.0 - w;
                    for ch in 0..2 {
                        let old = self.ref_tail[ch][i];
                        let new_s = self.in_buf[ch][seg_start + i];
                        self.out_buf[ch].push(old * inv_w + new_s * w);
                    }
                }

                let tail_start = seg_start + ol;
                let tail_end = (tail_start + self.overlap).min(self.in_buf[0].len());
                self.ref_tail[0] = self.in_buf[0][tail_start..tail_end].to_vec();
                self.ref_tail[1] = self.in_buf[1][tail_start..tail_end].to_vec();
            }

            self.next_analysis_start = (analysis_start as i64 + analysis_hop.max(1)) as usize;
        }

        // Compact input buffer to bound memory: drop everything strictly before
        // the earliest position still needed.
        let keep_from = self
            .next_analysis_start
            .saturating_sub(1)
            .min(self.in_buf[0].len());
        // Only compact when buffer grows large to avoid frequent shifts.
        if self.in_buf[0].len() > self.frame_len * 16 {
            let drop_n = keep_from.saturating_sub(self.delta * 2);
            if drop_n > 0 {
                self.in_buf[0].drain(..drop_n);
                self.in_buf[1].drain(..drop_n);
                self.next_analysis_start -= drop_n;
            }
        }
    }

    fn find_best_offset(&self, analysis_start: usize, _analysis_hop: i64) -> usize {
        if self.ref_tail[0].is_empty() {
            return analysis_start;
        }

        let natural = analysis_start as i64;
        let low = (natural - self.delta as i64).max(0) as usize;
        let high = (natural + self.delta as i64) as usize;

        let mut best = natural.max(0) as usize;
        let mut best_score = f32::NEG_INFINITY;

        let ol = self.overlap.min(self.ref_tail[0].len());

        let mut cand = low;
        while cand + ol <= self.in_buf[0].len() && cand <= high {
            // normalized correlation on mono mixdown of both channels
            let mut dot = 0.0f32;
            let mut e_ref = 0.0f32;
            let mut e_cand = 0.0f32;
            for i in 0..ol {
                let r = 0.5 * (self.ref_tail[0][i] + self.ref_tail[1][i]);
                let c = 0.5 * (self.in_buf[0][cand + i] + self.in_buf[1][cand + i]);
                dot += r * c;
                e_ref += r * r;
                e_cand += c * c;
            }
            let denom = (e_ref.sqrt() * e_cand.sqrt()) + 1e-9;
            let score = dot / denom;

            if score > best_score {
                best_score = score;
                best = cand;
            }
            cand += 1;
        }

        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, seconds: f64, sample_rate: f64) -> Vec<f32> {
        let n = (seconds * sample_rate) as usize;
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate).sin() as f32)
            .collect()
    }

    fn interleave(l: &[f32], r: &[f32]) -> Vec<f32> {
        l.iter()
            .zip(r.iter())
            .flat_map(|(a, b)| vec![*a, *b])
            .collect()
    }

    #[test]
    fn test_wsola_alpha_1_passthrough_length() {
        let mut ws = Wsola::new(48000.0);
        let l = sine(440.0, 2.0, 48000.0);
        let input = interleave(&l, &l);

        ws.push(&input);
        ws.flush();
        let mut out = Vec::new();
        ws.drain(&mut out);

        assert!(
            (out.len() as i64 - input.len() as i64).abs() < 4096,
            "alpha=1 should preserve length: in={} out={}",
            input.len(),
            out.len()
        );
    }

    #[test]
    fn test_wsola_double_speed_halves_duration_keeps_pitch() {
        let mut ws = Wsola::new(48000.0);
        ws.set_alpha(2.0);

        let l = sine(440.0, 4.0, 48000.0);
        let input = interleave(&l, &l);
        ws.push(&input);
        ws.flush();
        let mut out = Vec::new();
        ws.drain(&mut out);

        // Duration should be roughly halved
        let ratio = out.len() as f64 / input.len() as f64;
        assert!(
            (ratio - 0.5).abs() < 0.08,
            "alpha=2 should halve duration, got ratio {}",
            ratio
        );

        // Pitch should remain ~440 Hz: measure dominant zero-crossing frequency
        let mono: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        let mut crossings = 0usize;
        for w in mono.windows(2) {
            if w[0] <= 0.0 && w[1] > 0.0 {
                crossings += 1;
            }
        }
        let duration = mono.len() as f64 / 48000.0;
        let est_freq = crossings as f64 / duration;
        assert!(
            (est_freq - 440.0).abs() < 25.0,
            "pitch must be preserved under WSOLA: estimated {} Hz",
            est_freq
        );
    }
}
