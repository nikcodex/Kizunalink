// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use super::AudioFilter;

const BAND_COUNT: usize = 15;
const DEFAULT_MAKEUP_GAIN: f32 = 4.0;

struct Coefficients {
    beta: f32,
    alpha: f32,
    gamma: f32,
}

#[allow(clippy::excessive_precision)]
const COEFFICIENTS_48000: [Coefficients; BAND_COUNT] = [
    Coefficients {
        beta: 9.9847546664e-01,
        alpha: 7.6226668143e-04,
        gamma: 1.9984647656e+00,
    },
    Coefficients {
        beta: 9.9756184654e-01,
        alpha: 1.2190767289e-03,
        gamma: 1.9975344645e+00,
    },
    Coefficients {
        beta: 9.9616261379e-01,
        alpha: 1.9186931041e-03,
        gamma: 1.9960947369e+00,
    },
    Coefficients {
        beta: 9.9391578543e-01,
        alpha: 3.0421072865e-03,
        gamma: 1.9937449618e+00,
    },
    Coefficients {
        beta: 9.9028307215e-01,
        alpha: 4.8584639242e-03,
        gamma: 1.9898465702e+00,
    },
    Coefficients {
        beta: 9.8485897264e-01,
        alpha: 7.5705136795e-03,
        gamma: 1.9837962543e+00,
    },
    Coefficients {
        beta: 9.7588512657e-01,
        alpha: 1.2057436715e-02,
        gamma: 1.9731772447e+00,
    },
    Coefficients {
        beta: 9.6228521814e-01,
        alpha: 1.8857390928e-02,
        gamma: 1.9556164694e+00,
    },
    Coefficients {
        beta: 9.4080933132e-01,
        alpha: 2.9595334338e-02,
        gamma: 1.9242054384e+00,
    },
    Coefficients {
        beta: 9.0702059196e-01,
        alpha: 4.6489704022e-02,
        gamma: 1.8653476166e+00,
    },
    Coefficients {
        beta: 8.5868004289e-01,
        alpha: 7.0659978553e-02,
        gamma: 1.7600401337e+00,
    },
    Coefficients {
        beta: 7.8409610788e-01,
        alpha: 1.0795194606e-01,
        gamma: 1.5450725522e+00,
    },
    Coefficients {
        beta: 6.8332861002e-01,
        alpha: 1.5833569499e-01,
        gamma: 1.1426447155e+00,
    },
    Coefficients {
        beta: 5.5267518228e-01,
        alpha: 2.2366240886e-01,
        gamma: 4.0186190803e-01,
    },
    Coefficients {
        beta: 4.1811888447e-01,
        alpha: 2.9094055777e-01,
        gamma: -7.0905944223e-01,
    },
];

#[derive(Clone, Default)]
struct EqBandState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl EqBandState {
    fn process(&mut self, sample: f32, coeffs: &Coefficients) -> f32 {
        let result =
            coeffs.alpha * (sample - self.x2) + coeffs.gamma * self.y1 - coeffs.beta * self.y2;

        self.x2 = self.x1;
        self.x1 = sample;
        self.y2 = self.y1;

        if !result.is_finite() {
            self.y1 = 0.0;
            return 0.0;
        }

        self.y1 = result;
        result
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

pub struct EqualizerFilter {
    gains: [f32; BAND_COUNT],
    states: [[EqBandState; 2]; BAND_COUNT],
    makeup_gain: f32,
}

impl EqualizerFilter {
    pub fn new(bands: &[(u8, f32)]) -> Self {
        let mut gains = [0.0f32; BAND_COUNT];
        for &(band, gain) in bands {
            if (band as usize) < BAND_COUNT {
                gains[band as usize] = gain.clamp(-0.25, 1.0);
            }
        }

        let states: [[EqBandState; 2]; BAND_COUNT] =
            std::array::from_fn(|_| [EqBandState::default(), EqBandState::default()]);

        let makeup_gain = Self::compute_makeup_gain(&gains);

        Self {
            gains,
            states,
            makeup_gain,
        }
    }

    /// Normalize the summed filterbank so the theoretical peak fits in the unit range.
    ///
    /// The output of one sample is `0.25 * input + Σ(gain_b * band_b)`. With no boosted
    /// bands the worst-case peak is `0.25`, so the makeup gain must be `1 / 0.25 = 4`
    /// (this is why the legacy hardcoded value was `4.0`). Every unit of positive band
    /// gain can add up to 1.0 of peak, so the safe scale shrinks as boosts grow; it is
    /// capped at the legacy `DEFAULT_MAKEUP_GAIN` so cutting-only settings are unaffected.
    fn compute_makeup_gain(gains: &[f32; BAND_COUNT]) -> f32 {
        let boost_sum: f32 = gains.iter().map(|g| g.max(0.0)).sum();
        (1.0 / (0.25 + boost_sum)).min(DEFAULT_MAKEUP_GAIN)
    }
}

impl AudioFilter for EqualizerFilter {
    fn process(&mut self, samples: &mut [i16]) {
        let num_frames = samples.len() / 2;

        for frame in 0..num_frames {
            let offset = frame * 2;

            let left_f = samples[offset] as f32 / 32768.0;
            let right_f = samples[offset + 1] as f32 / 32768.0;

            let mut result_left = left_f * 0.25;
            let mut result_right = right_f * 0.25;

            for (b, coeffs) in COEFFICIENTS_48000.iter().enumerate() {
                let gain = self.gains[b];
                if gain.abs() < f32::EPSILON {
                    // B04 (intentional): a zero-gain band's output is summed with weight 0, so we
                    // skip mixing — but we still *advance* its filter state with the current sample.
                    // This keeps each band's memory in sync with the signal, so enabling a band
                    // later (Lavalink `filters` updates arrive at runtime) starts from the live
                    // signal instead of a stale state, which would produce an audible zipper click.
                    self.states[b][0].process(left_f, coeffs);
                    self.states[b][1].process(right_f, coeffs);
                    continue;
                }

                let band_left = self.states[b][0].process(left_f, coeffs);
                let band_right = self.states[b][1].process(right_f, coeffs);

                result_left += band_left * gain;
                result_right += band_right * gain;
            }

            let out_left = (result_left * self.makeup_gain).clamp(-1.0, 1.0);
            let out_right = (result_right * self.makeup_gain).clamp(-1.0, 1.0);

            samples[offset] = (out_left * 32767.0) as i16;
            samples[offset + 1] = (out_right * 32767.0) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.gains.iter().any(|g| g.abs() > f32::EPSILON)
    }

    fn reset(&mut self) {
        for band_states in self.states.iter_mut() {
            for state in band_states.iter_mut() {
                state.reset();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_boost_makes_up_exactly_the_quarter_scale() {
        // With every band at 0 gain, makeup must be 1 / 0.25 = 4.0 (legacy behavior).
        let eq = EqualizerFilter::new(&[]);
        assert_eq!(eq.makeup_gain, DEFAULT_MAKEUP_GAIN);

        let all_zero = [0.0f32; BAND_COUNT];
        assert_eq!(EqualizerFilter::compute_makeup_gain(&all_zero), 4.0);
    }

    #[test]
    fn boost_shrinks_makeup_gain() {
        let mut gains = [0.0f32; BAND_COUNT];
        gains[0] = 1.0; // one band at maximum boost
        // Worst-case peak is 0.25 + 1.0 = 1.25 -> scale to fit unity.
        assert!((EqualizerFilter::compute_makeup_gain(&gains) - 0.8).abs() < 1e-6);

        gains = [1.0; BAND_COUNT]; // every band fully boosted
        let g = EqualizerFilter::compute_makeup_gain(&gains);
        assert!((g - 1.0 / 15.25).abs() < 1e-6);
        assert!(g <= DEFAULT_MAKEUP_GAIN);
    }

    #[test]
    fn cuts_only_are_unaffected() {
        let mut gains = [0.0f32; BAND_COUNT];
        gains[3] = -0.25;
        assert_eq!(
            EqualizerFilter::compute_makeup_gain(&gains),
            DEFAULT_MAKEUP_GAIN
        );
    }

    #[test]
    fn flat_eq_is_transparent_on_dc() {
        // No bands configured: output should track the input within one LSB.
        let mut eq = EqualizerFilter::new(&[]);
        let original = [12345i16; 64];
        let mut samples = original;
        eq.process(&mut samples);
        for (out, input) in samples.iter().zip(original.iter()) {
            assert!((*out - input).abs() <= 1, "got {out} for {input}");
        }
    }

    #[test]
    fn full_scale_square_with_all_bands_stays_in_range() {
        let bands: Vec<(u8, f32)> = (0..BAND_COUNT as u8).map(|b| (b, 1.0)).collect();
        let mut eq = EqualizerFilter::new(&bands);
        let mut samples = [-32768i16, 32767].repeat(32);
        eq.process(&mut samples);
        assert!(samples.iter().any(|&sample| sample < 0));
        assert!(samples.iter().any(|&sample| sample > 0));
    }
}
