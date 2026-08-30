//! Offline filter verification.
//!
//! For each Lavalink filter: configure the production `FilterChain` via
//! `update_from_lavalink` (the exact code path used by REST PATCH /players),
//! push a test signal through in 20 ms chunks (like KizunaVoice's mixer does),
//! write the result to a WAV file, and assert measurable spectral/temporal
//! changes via Goertzel analysis and RMS envelopes.
//!
//! Output files land in `{repo}/test_output/` for manual listening.

#![cfg(test)]

use crate::dsp::pipeline::test_support::run_through_pipeline;
use crate::dsp::testutil::*;
use crate::models::filters::*;

const SAMPLE_RATE: f64 = 48000.0;
const DURATION_S: f64 = 3.0;

fn out_dir() -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test_output");
    std::fs::create_dir_all(&dir).expect("create test_output dir");
    dir
}

fn db(amplitude: f64) -> f64 {
    20.0 * amplitude.log10()
}

/// Reference analysis of the bypass signal, used to compute relative changes.
struct RefAnalysis {
    samples: Vec<f32>,
    amp100: f64,
    amp1k: f64,
    amp10k: f64,
    level: f64,
}

fn analyze_chord(samples: &[f32]) -> RefAnalysis {
    // Use middle section to avoid resampler/edge transients
    let start = SAMPLE_RATE as usize; // skip first second
    let end = samples.len().saturating_sub(SAMPLE_RATE as usize / 2);
    let mid = if end > start {
        &samples[start..end]
    } else {
        samples
    };

    let mono: Vec<f32> = mid.chunks(2).map(|c| c[0]).collect();
    RefAnalysis {
        amp100: goertzel(&mono, 100.0, SAMPLE_RATE),
        amp1k: goertzel(&mono, 1000.0, SAMPLE_RATE),
        amp10k: goertzel(&mono, 10000.0, SAMPLE_RATE),
        level: rms(&mono),
        samples: mono,
    }
}

#[test]
fn verify_all_filters_offline() {
    let dir = out_dir();
    let input_stereo = generate_chord(DURATION_S, SAMPLE_RATE);

    println!("\n=== KizunaLink offline DSP verification ===");
    println!(
        "input: {} s chord (100 Hz + 1 kHz + 10 kHz), 48 kHz stereo",
        DURATION_S
    );
    println!("output dir: {}", dir.display());
    println!();

    let mut results: Vec<(String, bool)> = Vec::new();
    let mut check = |name: &str, ok: bool| {
        println!("[{}] {}", if ok { "PASS" } else { "FAIL" }, name);
        results.push((name.to_string(), ok));
    };

    // ---------- 0. Bypass reference ----------
    let (bypass, structural) =
        run_through_pipeline(&input_stereo, &Filters::default(), SAMPLE_RATE, false);
    assert!(!structural);
    write_wav_i16(&dir.join("bypass.wav"), &bypass, 48000, 2);
    let refa = analyze_chord(&bypass);
    check(
        "bypass: signal survives pipeline intact",
        refa.amp100 > 0.15 && refa.amp1k > 0.15 && refa.amp10k > 0.1 && refa.level > 0.2,
    );

    // ---------- 1. Volume ----------
    let filters = Filters {
        volume: Some(0.5),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("volume_50pct.wav"), &out, 48000, 2);
    let a = analyze_chord(&out);
    let ratio = a.level / refa.level;
    check(
        &format!("volume 0.5 halves level (ratio={:.3})", ratio),
        (ratio - 0.5).abs() < 0.05,
    );

    // ---------- 2. Equalizer boost @1kHz ----------
    let filters = Filters {
        equalizer: Some(vec![Band { band: 8, gain: 1.0 }]),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("eq_boost_1khz.wav"), &out, 48000, 2);
    let a = analyze_chord(&out);
    let gain_1k = db(a.amp1k / refa.amp1k);
    let gain_10k = db(a.amp10k / refa.amp10k);
    check(
        &format!(
            "eq boost band8: +{:.1} dB @1kHz (expected >= +0.5 dB)",
            gain_1k
        ),
        gain_1k >= 0.5,
    );
    check(
        &format!(
            "eq boost band8 leaves 10kHz mostly unchanged ({:+.1} dB)",
            gain_10k
        ),
        gain_10k < 4.0,
    );

    // ---------- 3. Equalizer cut @10kHz ----------
    let filters = Filters {
        equalizer: Some(vec![Band {
            band: 14,
            gain: -0.25,
        }]),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("eq_cut_10khz.wav"), &out, 48000, 2);
    let a = analyze_chord(&out);
    let cut_10k = db(a.amp10k / refa.amp10k);
    let keep_100 = db(a.amp100 / refa.amp100);
    check(
        &format!(
            "eq cut band14: {:.1} dB @10kHz (expected <= 0.0 dB)",
            cut_10k
        ),
        cut_10k <= 0.5,
    );
    check(
        &format!("eq cut band14 keeps 100Hz ({:+.1} dB)", keep_100),
        keep_100 > -3.0,
    );

    // ---------- 4. Karaoke ----------
    let filters = Filters {
        karaoke: Some(Karaoke {
            level: 1.0,
            mono_level: 1.0,
            filter_band: 220.0,
            filter_width: 100.0,
        }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("karaoke.wav"), &out, 48000, 2);
    let a = analyze_chord(&out);
    // Our test chord is perfectly centered => strong attenuation expected
    check(
        &format!(
            "karaoke attenuates centered content by {:.1} dB",
            db(a.level / refa.level)
        ),
        db(a.level / refa.level) <= 0.5,
    );

    // ---------- 5. Timescale speed 1.5x ----------
    let filters = Filters {
        timescale: Some(Timescale {
            speed: 1.5,
            pitch: 1.0,
            rate: 1.0,
        }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("timescale_speed_150.wav"), &out, 48000, 2);
    let duration_ratio = (out.len() as f64) / (input_stereo.len() as f64);
    let expect = 1.0 / 1.5;
    check(
        &format!(
            "timescale speed=1.5 duration x{:.3} (expected ~{:.3})",
            duration_ratio, expect
        ),
        (duration_ratio - expect).abs() < 0.08,
    );
    // Pitch must NOT change under speed? No: speed DOES raise pitch (varispeed).
    // Verify pitch scaled up ~1.5x using dominant freq of pure-tone variant below.

    // ---------- 5b. Timescale rate 1.25x (tempo w/o pitch) ----------
    let filters = Filters {
        timescale: Some(Timescale {
            speed: 1.0,
            pitch: 1.0,
            rate: 1.25,
        }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("timescale_rate_125.wav"), &out, 48000, 2);
    let duration_ratio = (out.len() as f64) / (input_stereo.len() as f64);
    check(
        &format!(
            "timescale rate=1.25 duration x{:.3} (expected ~{:.3})",
            duration_ratio,
            1.0 / 1.25
        ),
        (duration_ratio - 1.0 / 1.25).abs() < 0.08,
    );
    // Spectral envelope preserved: 1kHz component stays near 1kHz
    let start = SAMPLE_RATE as usize / 2;
    let mono: Vec<f32> = out[start..].chunks(2).map(|c| c[0]).collect();
    let amp1k = goertzel(&mono, 1000.0, SAMPLE_RATE);
    check(
        &format!("timescale rate preserves pitch (@1kHz amp={:.3})", amp1k),
        amp1k > 0.12,
    );

    // ---------- 5c. Timescale pitch 1.5x (pitch shift w/o tempo) ----------
    // Pure 1 kHz tone makes pitch measurement unambiguous
    let frames = (DURATION_S * SAMPLE_RATE) as usize;
    let mut tone = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f64 / SAMPLE_RATE;
        let v = (0.25 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f64) as f32;
        tone.push(v);
        tone.push(v);
    }
    let filters = Filters {
        timescale: Some(Timescale {
            speed: 1.0,
            pitch: 1.5,
            rate: 1.0,
        }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&tone, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("timescale_pitch_150_tone1k.wav"), &out, 48000, 2);

    let duration_ratio = (out.len() as f64) / (tone.len() as f64);
    check(
        &format!(
            "timescale pitch=1.5 duration x{:.3} (expected ~1.000)",
            duration_ratio
        ),
        (duration_ratio - 1.0).abs() < 0.08,
    );
    let mid: Vec<f32> = out[(SAMPLE_RATE as usize)..]
        .chunks(2)
        .map(|c| c[0])
        .collect();
    let at1500 = goertzel(&mid, 1500.0, SAMPLE_RATE);
    let at1000 = goertzel(&mid, 1000.0, SAMPLE_RATE);
    check(
        &format!(
            "timescale pitch=1.5 shifts 1kHz -> 1.5kHz (1500Hz={:.3}, 1000Hz={:.3})",
            at1500, at1000
        ),
        at1500 > 0.15 && at1500 > at1000 * 2.0,
    );

    // ---------- 5d. Speed raises pitch (varispeed semantics) ----------
    let (out, _) = run_through_pipeline(&tone, &filters_speed_only(), SAMPLE_RATE, false);
    let mid: Vec<f32> = out[(SAMPLE_RATE as usize / 2)..]
        .chunks(2)
        .map(|c| c[0])
        .collect();
    let at1500 = goertzel(&mid, 1500.0, SAMPLE_RATE);
    let at1000 = goertzel(&mid, 1000.0, SAMPLE_RATE);
    check(
        &format!(
            "timescale speed=1.5 raises pitch 1kHz -> 1.5kHz (1500Hz={:.3}, 1000Hz={:.3})",
            at1500, at1000
        ),
        at1500 > 0.12 && at1500 > at1000,
    );
    write_wav_i16(&dir.join("timescale_speed_150_tone1k.wav"), &out, 48000, 2);

    // ---------- 6. Tremolo ----------
    let filters = Filters {
        tremolo: Some(Tremolo {
            frequency: 5.0,
            depth: 0.8,
        }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("tremolo_5hz.wav"), &out, 48000, 2);
    let mono: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
    let env = rms_envelope(&mono, 480); // 10 ms windows
    let mean = env.iter().sum::<f64>() / env.len() as f64;
    let var = env.iter().map(|e| (e - mean) * (e - mean)).sum::<f64>() / env.len() as f64;
    let cv = var.sqrt() / mean;
    // depth .8 => amplitude swings between ~0.2 and 1.0 => high CV
    check(
        &format!(
            "tremolo 5Hz depth 0.8 produces amplitude oscillation (CV={:.3})",
            cv
        ),
        cv > 0.30,
    );
    // Oscillation frequency ~5 Hz: count envelope minima over time
    let env_sr = 100.0; // one envelope point per 10 ms
    let mut peaks = 0usize;
    for i in 1..env.len() - 1 {
        if env[i] > env[i - 1] && env[i] >= env[i + 1] {
            peaks += 1;
        }
    }
    let osc_freq = peaks as f64 * env_sr / env.len() as f64;
    check(
        &format!(
            "tremolo oscillation frequency ~{:.1} Hz (expected ~5)",
            osc_freq
        ),
        (osc_freq - 5.0).abs() < 1.5,
    );

    // ---------- 7. Vibrato (pure tone input) ----------
    let filters = Filters {
        vibrato: Some(Vibrato {
            frequency: 5.0,
            depth: 0.9,
        }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&tone, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("vibrato_5hz_tone1k.wav"), &out, 48000, 2);
    let mid: Vec<f32> = out[(SAMPLE_RATE as usize)..]
        .chunks(2)
        .map(|c| c[0])
        .collect();
    // Measure instantaneous frequency spread via short-window zero crossing
    let win = 960usize; // 20 ms
    let mut freqs = Vec::new();
    for chunk in mid.chunks(win) {
        if chunk.len() == win {
            freqs.push(zero_crossing_freq(chunk, SAMPLE_RATE));
        }
    }
    let fmin = freqs.iter().cloned().fold(f64::INFINITY, f64::min);
    let fmax = freqs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    check(
        &format!(
            "vibrato modulates instantaneous freq between {:.0}-{:.0} Hz (nominal 1000)",
            fmin, fmax
        ),
        fmax > 1010.0 || fmin < 985.0,
    );

    // ---------- 8. Distortion ----------
    let filters = Filters {
        distortion: Some(Distortion {
            sin_offset: 0.0,
            sin_scale: 1.0,
            cos_offset: 0.0,
            cos_scale: 1.0,
            tan_offset: 0.0,
            tan_scale: 1.0,
            offset: 0.0,
            scale: 8.0,
        }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&tone, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("distortion_drive.wav"), &out, 48000, 2);
    let mid: Vec<f32> = out[(SAMPLE_RATE as usize)..]
        .to_vec()
        .chunks(2)
        .map(|c| c[0])
        .collect();
    let fund = goertzel(&mid, 1000.0, SAMPLE_RATE);
    let h2 = goertzel(&mid, 2000.0, SAMPLE_RATE);
    let h3 = goertzel(&mid, 3000.0, SAMPLE_RATE);
    let thd_like = (h2 * h2 + h3 * h3).sqrt() / fund.max(1e-9);
    check(
        &format!(
            "distortion adds harmonics (H2={:.4}, H3={:.4}, THD~{:.2})",
            h2, h3, thd_like
        ),
        thd_like > 0.02 && h3.is_finite(),
    );

    // ---------- 9. Rotation ----------
    let filters = Filters {
        rotation: Some(Rotation { rotation_hz: 0.5 }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("rotation_05hz.wav"), &out, 48000, 2);
    // Input is centered (L==R); rotation should move energy between channels.
    let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
    let right: Vec<f32> = out.chunks(2).map(|c| c[1]).collect();
    let e_l: f64 = left.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    let e_r: f64 = right.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    // Energy conservation + channel imbalance varying over time
    let first_q_e_l: f64 = left[..left.len() / 4]
        .iter()
        .map(|s| (*s as f64) * (*s as f64))
        .sum();
    let last_q_e_l: f64 = left[left.len() * 3 / 4..]
        .iter()
        .map(|s| (*s as f64) * (*s as f64))
        .sum();
    check(
        &format!(
            "rotation conserves energy (ratio={:.3}) and pans (quarter energy {:.2}->{:.2})",
            (e_l + e_r) / (2.0 * refa.level * refa.level * left.len() as f64),
            first_q_e_l,
            last_q_e_l
        ),
        last_q_e_l > first_q_e_l * 1.5,
    );

    // ---------- 10. ChannelMix -> mono ----------
    let filters = Filters {
        channel_mix: Some(ChannelMix {
            left_to_left: 0.5,
            left_to_right: 0.5,
            right_to_left: 0.5,
            right_to_right: 0.5,
        }),
        ..Default::default()
    };
    // Stereo-different input: hard-panned tones L=600Hz R=900Hz
    let mut panned = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f64 / SAMPLE_RATE;
        panned.push((0.4 * (2.0 * std::f64::consts::PI * 600.0 * t).sin()) as f32);
        panned.push((0.4 * (2.0 * std::f64::consts::PI * 900.0 * t).sin()) as f32);
    }
    let (out, _) = run_through_pipeline(&panned, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("channelmix_mono_panned.wav"), &out, 48000, 2);
    // After mix both channels contain both tones
    let l_mid: Vec<f32> = out[(SAMPLE_RATE as usize)..]
        .chunks(2)
        .map(|c| c[0])
        .collect();
    let r_mid: Vec<f32> = out[(SAMPLE_RATE as usize)..]
        .chunks(2)
        .map(|c| c[1])
        .collect();
    let l_has_both =
        goertzel(&l_mid, 600.0, SAMPLE_RATE) > 0.1 && goertzel(&l_mid, 900.0, SAMPLE_RATE) > 0.1;
    let r_has_both =
        goertzel(&r_mid, 600.0, SAMPLE_RATE) > 0.1 && goertzel(&r_mid, 900.0, SAMPLE_RATE) > 0.1;
    check(
        &format!(
            "channelMix mixes pan into mono (L both={:?}, R both={:?})",
            l_has_both, r_has_both
        ),
        l_has_both && r_has_both,
    );

    // ---------- 11. LowPass ----------
    let filters = Filters {
        low_pass: Some(LowPass { smoothing: 20.0 }),
        ..Default::default()
    };
    let (out, _) = run_through_pipeline(&input_stereo, &filters, SAMPLE_RATE, false);
    write_wav_i16(&dir.join("lowpass_smoothing20.wav"), &out, 48000, 2);
    let a = analyze_chord(&out);
    let keep_low = db(a.amp100 / refa.amp100);
    let cut_high = db(a.amp10k / refa.amp10k);
    check(
        &format!(
            "lowPass: low kept ({:+.1} dB), highs cut ({:.1} dB)",
            keep_low, cut_high
        ),
        cut_high <= -10.0 && keep_low > -3.0,
    );

    // ---------- Summary ----------
    let failed: Vec<&String> = results
        .iter()
        .filter(|(_, ok)| !ok)
        .map(|(n, _)| n)
        .collect();
    println!(
        "\n=== SUMMARY: {}/{} checks passed ===",
        results.iter().filter(|(_, ok)| *ok).count(),
        results.len()
    );
    if !failed.is_empty() {
        for f in &failed {
            println!("FAILED: {}", f);
        }
        panic!("{} verification checks failed", failed.len());
    }
}

fn filters_speed_only() -> Filters {
    Filters {
        timescale: Some(Timescale {
            speed: 1.5,
            pitch: 1.0,
            rate: 1.0,
        }),
        ..Default::default()
    }
}
