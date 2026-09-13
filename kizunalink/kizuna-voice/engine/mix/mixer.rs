// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
};

use flume::Receiver;

use super::layer::MixLayer;
use crate::{
    config::player::PlayerConfig,
    engine::{
        AudioFrame,
        buffer::PooledBuffer,
        constants::{MAX_LAYERS, MIXER_CHANNELS, TARGET_SAMPLE_RATE},
        flow::FlowController,
        playback::handle::PlaybackState,
    },
};

pub struct AudioMixer {
    pub layers: HashMap<String, MixLayer>,
    pub max_layers: usize,
    pub enabled: bool,
    acc_buf: Vec<i32>,
}

impl Default for AudioMixer {
    fn default() -> Self {
        Self::new()
    }
}

/// Transparent below ~0.9 full scale; exponentially compresses the excess above it so that
/// loud sums saturate smoothly instead of hard-clipping into square-wave distortion (B23).
/// The asymptote is ±full scale, so the result always fits in `i16`.
#[inline]
pub(crate) fn soft_clip_i16(sum: i32) -> i16 {
    /// Soft-knee threshold at 0.9 of full scale, in the i16 sample domain.
    const SOFT_LIMIT: i32 = 29491; // (0.9 * 32768) as i32
    const THRESHOLD: f32 = SOFT_LIMIT as f32 / 32768.0;
    const HEADROOM: f32 = 1.0 - THRESHOLD;

    // `unsigned_abs` keeps the i32::MIN edge case well-defined (no `abs` overflow).
    let mag = sum.unsigned_abs();
    if mag <= SOFT_LIMIT as u32 {
        return sum as i16;
    }

    let sign = if sum < 0 { -1.0 } else { 1.0 };
    let over = mag as f32 / 32768.0 - THRESHOLD;
    let y = THRESHOLD + HEADROOM * (1.0 - (-over / HEADROOM).exp());
    ((y * 32768.0).min(i16::MAX as f32) as i16 as f32 * sign) as i16
}

impl AudioMixer {
    pub fn new() -> Self {
        Self {
            layers: HashMap::new(),
            max_layers: MAX_LAYERS,
            enabled: true,
            acc_buf: Vec::with_capacity(1920),
        }
    }

    pub fn add_layer(
        &mut self,
        id: String,
        rx: Receiver<PooledBuffer>,
        volume: f32,
    ) -> Result<(), &'static str> {
        if self.layers.len() >= self.max_layers {
            return Err("Maximum mix layers reached");
        }
        self.layers
            .insert(id.clone(), MixLayer::new(id, rx, volume));
        Ok(())
    }

    pub fn remove_layer(&mut self, id: &str) {
        self.layers.remove(id);
    }

    pub fn set_layer_volume(&mut self, id: &str, volume: f32) {
        if let Some(layer) = self.layers.get_mut(id) {
            layer.volume = volume.clamp(0.0, 1.0);
        }
    }

    pub fn mix(&mut self, main_frame: &mut [i16]) {
        if !self.enabled || self.layers.is_empty() {
            return;
        }

        let out_len = main_frame.len();
        if self.acc_buf.len() != out_len {
            self.acc_buf.resize(out_len, 0);
        }

        for (acc, &sample) in self.acc_buf.iter_mut().zip(main_frame.iter()) {
            *acc = sample as i32;
        }

        self.layers.retain(|_, layer| {
            layer.fill();
            !layer.is_dead()
        });

        for layer in self.layers.values_mut() {
            layer.accumulate(&mut self.acc_buf);
        }

        for (out, &sum) in main_frame.iter_mut().zip(self.acc_buf.iter()) {
            *out = soft_clip_i16(sum);
        }
    }
}

pub struct Mixer {
    tracks: Vec<MixerTrack>,
    mix_buf: Vec<i32>,
    pub audio_mixer: AudioMixer,
    opus_passthrough_track: Option<usize>, // index of track providing opus passthrough
    final_pcm_buf: Vec<i16>,
}

// PassthroughTrack is now implicitly handled by MixerTrack via FlowController's latest_opus

struct MixerTrack {
    flow: FlowController,
    pending: Vec<i16>,
    pending_pos: usize,
    state: Arc<AtomicU8>,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicU64>,
    is_buffering: Arc<AtomicBool>,
    config: PlayerConfig,
    finished: bool,
}

impl Mixer {
    pub fn new(_sample_rate: u32) -> Self {
        Self {
            tracks: Vec::new(),
            mix_buf: Vec::with_capacity(1920),
            audio_mixer: AudioMixer::new(),
            opus_passthrough_track: None,
            final_pcm_buf: Vec::with_capacity(1920),
        }
    }

    pub fn add_track(
        &mut self,
        rx: Receiver<AudioFrame>,
        state: Arc<AtomicU8>,
        volume: Arc<AtomicU32>,
        position: Arc<AtomicU64>,
        is_buffering: Arc<AtomicBool>,
        config: PlayerConfig,
    ) {
        let vol_raw = f32::from_bits(volume.load(Ordering::Acquire));
        let mut flow = FlowController::for_mixer(rx, TARGET_SAMPLE_RATE, MIXER_CHANNELS, vol_raw);
        flow.volume.set_volume_instant(vol_raw);

        self.tracks.push(MixerTrack {
            flow,
            pending: Vec::new(),
            pending_pos: 0,
            state,
            volume,
            position,
            is_buffering,
            config,
            finished: false,
        });
    }

    pub fn set_passthrough_track(&mut self, track_index: usize) {
        self.opus_passthrough_track = Some(track_index);
    }

    pub fn take_opus_frame(&mut self) -> Option<Vec<u8>> {
        for track in self.tracks.iter_mut() {
            let state = PlaybackState::from(track.state.load(Ordering::Acquire));
            if matches!(
                state,
                PlaybackState::Paused
                    | PlaybackState::Stopped
                    | PlaybackState::Stopping
                    | PlaybackState::Starting
            ) {
                continue;
            }

            if let Some(packet) = track.flow.take_opus() {
                track.position.fetch_add(960, Ordering::Relaxed);
                return Some(packet);
            }
        }
        None
    }

    pub fn stop_all(&mut self) {
        for track in self.tracks.iter_mut() {
            track
                .state
                .store(PlaybackState::Stopped as u8, Ordering::Release);
        }
        self.tracks.clear();
        self.audio_mixer.enabled = false;
    }

    pub fn mix(&mut self, buf: &mut [i16]) -> bool {
        let out_len = buf.len();

        if self.mix_buf.len() != out_len {
            self.mix_buf.resize(out_len, 0);
        }
        self.mix_buf.fill(0);

        self.tracks
            .retain(|t| t.state.load(Ordering::Acquire) != PlaybackState::Stopped as u8);

        let mut has_audio = false;

        for track in self.tracks.iter_mut() {
            let state = PlaybackState::from(track.state.load(Ordering::Acquire));

            if matches!(state, PlaybackState::Paused | PlaybackState::Stopped) {
                continue;
            }

            let vol_f = f32::from_bits(track.volume.load(Ordering::Acquire));
            if (vol_f - track.flow.volume.current_volume()).abs() > 0.001 {
                track.flow.volume.set_volume(vol_f);
            }

            if state == PlaybackState::Stopping && !track.flow.tape.is_ramping() {
                track.flow.tape.tape_to(
                    track.config.tape.tape_stop_duration_ms as f32,
                    "stop",
                    track.config.tape.curve,
                );
            } else if state == PlaybackState::Starting && !track.flow.tape.is_ramping() {
                track.flow.tape.tape_to(
                    track.config.tape.tape_stop_duration_ms as f32,
                    "start",
                    track.config.tape.curve,
                );
            }

            let mut filled = 0usize;

            // 1. Drain pending buffer
            if track.pending_pos < track.pending.len() {
                let (_, room) = self.mix_buf.split_at_mut(filled);
                let avail = (out_len - filled)
                    .min(room.len())
                    .min(track.pending.len() - track.pending_pos);
                let pending = &track.pending[track.pending_pos..track.pending_pos + avail];
                for (acc, &s) in room.iter_mut().take(avail).zip(pending) {
                    *acc += s as i32;
                }
                track.pending_pos += avail;
                filled += avail;

                if track.pending_pos >= track.pending.len() {
                    track.pending.clear();
                    track.pending_pos = 0;
                }
            }

            // 2. Pull new frames from flow
            'pull: while filled < out_len && !track.finished {
                match track.flow.try_pop_frame() {
                    Ok(Some(frame)) => {
                        let (_, room) = self.mix_buf.split_at_mut(filled);
                        let n = frame.len().min(room.len());
                        for (acc, &s) in room.iter_mut().take(n).zip(frame.iter()) {
                            *acc += s as i32;
                        }

                        if n < frame.len() {
                            track.pending.extend_from_slice(&frame[n..]);
                            track.pending_pos = 0;
                        }
                        filled += n;
                        crate::engine::buffer::release_buffer(frame);
                    }
                    Ok(None) => break 'pull,
                    Err(_) => {
                        track.finished = true;
                        break 'pull;
                    }
                }
            }

            if filled > 0 {
                has_audio = true;
                track
                    .position
                    .fetch_add(filled as u64 / MIXER_CHANNELS as u64, Ordering::Relaxed);
                track.is_buffering.store(false, Ordering::Release);
            } else if !track.finished {
                track.is_buffering.store(true, Ordering::Release);
            }

            if track.finished && track.pending.is_empty() && !track.flow.tape.is_active() {
                track
                    .state
                    .store(PlaybackState::Stopped as u8, Ordering::Release);
            }

            if track.flow.tape.check_ramp_completed() {
                match state {
                    PlaybackState::Stopping => {
                        track
                            .state
                            .store(PlaybackState::Paused as u8, Ordering::Release);
                    }
                    PlaybackState::Starting => {
                        track
                            .state
                            .store(PlaybackState::Playing as u8, Ordering::Release);
                    }
                    _ => {}
                }
            }
        }

        if self.final_pcm_buf.len() != out_len {
            self.final_pcm_buf.resize(out_len, 0);
        }

        for (final_pcm, &sum) in self.final_pcm_buf.iter_mut().zip(self.mix_buf.iter()) {
            *final_pcm = soft_clip_i16(sum);
        }

        self.audio_mixer.mix(&mut self.final_pcm_buf);
        if !self.audio_mixer.layers.is_empty() {
            has_audio = true;
        }

        buf.copy_from_slice(&self.final_pcm_buf);
        has_audio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_mixer_new_is_empty() {
        let mixer = AudioMixer::new();
        assert!(mixer.layers.is_empty());
        assert!(mixer.enabled);
        assert_eq!(mixer.max_layers, MAX_LAYERS);
    }

    #[test]
    fn audio_mixer_disabled_does_not_modify() {
        let mut mixer = AudioMixer::new();
        mixer.enabled = false;
        let mut frame = [100i16, 200, 300, 400];
        let original = frame;
        mixer.mix(&mut frame);
        assert_eq!(frame, original);
    }

    #[test]
    fn audio_mixer_empty_layers_does_not_modify() {
        let mut mixer = AudioMixer::new();
        let mut frame = [100i16, 200, 300, 400];
        let original = frame;
        mixer.mix(&mut frame);
        assert_eq!(frame, original);
    }

    #[test]
    fn audio_mixer_max_layers_enforced() {
        let mut mixer = AudioMixer::new();
        for i in 0..MAX_LAYERS {
            let (_tx, rx) = flume::unbounded();
            assert!(mixer.add_layer(format!("layer-{i}"), rx, 1.0).is_ok());
        }
        let (_tx, rx) = flume::unbounded();
        assert!(mixer.add_layer("overflow".into(), rx, 1.0).is_err());
    }

    #[test]
    fn audio_mixer_remove_layer() {
        let mut mixer = AudioMixer::new();
        let (_tx, rx) = flume::unbounded();
        mixer.add_layer("test".into(), rx, 1.0).unwrap();
        assert_eq!(mixer.layers.len(), 1);
        mixer.remove_layer("test");
        assert!(mixer.layers.is_empty());
    }

    #[test]
    fn audio_mixer_set_layer_volume_clamps() {
        let mut mixer = AudioMixer::new();
        let (_tx, rx) = flume::unbounded();
        mixer.add_layer("test".into(), rx, 1.0).unwrap();
        mixer.set_layer_volume("test", 5.0);
        assert_eq!(mixer.layers["test"].volume, 1.0);
        mixer.set_layer_volume("test", -1.0);
        assert_eq!(mixer.layers["test"].volume, 0.0);
    }

    #[test]
    fn soft_clip_is_transparent_below_threshold() {
        for sum in [-29491i32, -1000, -1, 0, 1, 1000, 29491] {
            assert_eq!(soft_clip_i16(sum), sum as i16, "transparent at {sum}");
        }
    }

    #[test]
    fn soft_clip_bounds_and_monotonicity() {
        let mut prev = i16::MIN;
        for sum in [
            i32::MIN,
            -1_000_000,
            -65536,
            -29492,
            29492,
            65536,
            1_000_000,
            i32::MAX,
        ] {
            let out = soft_clip_i16(sum);
            assert!((i16::MIN..=i16::MAX).contains(&out));
            assert!(out.abs() <= i16::MAX, "never exceeds full scale");
            if sum > -29492 {
                assert!(out >= prev, "monotonic at {sum}");
            }
            prev = out;
        }
        // Beyond the knee the curve saturates but keeps sign and ordering.
        assert!(soft_clip_i16(-1_000_000) < soft_clip_i16(-29492));
        assert!(soft_clip_i16(1_000_000) > soft_clip_i16(29492));
        assert!(soft_clip_i16(1_000_000) < i16::MAX); // asymptotic, not a hard wall
    }
}
