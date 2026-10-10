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
    let magnitude = (y * 32768.0).min(i16::MAX as f32) as i16;
    if sign < 0.0 { -magnitude } else { magnitude }
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
        // Re-enable after stop_all() so layers can be added again
        if !self.enabled {
            self.enabled = true;
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

    /// Stops and removes the track whose state cell is the same allocation as
    /// `state`, leaving other tracks and sound-effect layers untouched.
    ///
    /// Does nothing when no track has that state cell.
    pub fn stop_track(&mut self, state: &Arc<AtomicU8>) {
        let Some(idx) = self
            .tracks
            .iter()
            .position(|t| Arc::ptr_eq(&t.state, state))
        else {
            return;
        };

        let track = self.tracks.remove(idx);
        track
            .state
            .store(PlaybackState::Stopped as u8, Ordering::Release);

        // Keep the passthrough index pointing at the same track after removal.
        match self.opus_passthrough_track {
            Some(p) if p == idx => self.opus_passthrough_track = None,
            Some(ref mut p) if *p > idx => *p -= 1,
            _ => {}
        }
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

            // Drive the tape rate toward the desired state whenever no ramp is in
            // flight. This must key off the *target* state (Playing/Starting vs
            // Stopping), not the previous state, so that a resume arriving mid
            // stop-ramp still recovers instead of leaving the tape pinned at 0.01.
            if !track.flow.tape.is_ramping() {
                let want_play = matches!(state, PlaybackState::Playing | PlaybackState::Starting);
                let rate = track.flow.tape.rate();
                if want_play && rate < 0.999 {
                    track.flow.tape.tape_to(
                        track.config.tape.tape_stop_duration_ms as f32,
                        "start",
                        track.config.tape.curve,
                    );
                } else if !want_play && rate > 0.011 {
                    track.flow.tape.tape_to(
                        track.config.tape.tape_stop_duration_ms as f32,
                        "stop",
                        track.config.tape.curve,
                    );
                }
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
    use crate::engine::constants::FRAME_SIZE_SAMPLES;

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

    /// PERF-002: `stop_all` disables the mixer, but a later `add_layer` must
    /// restore `enabled` so newly added sound-effect layers are actually mixed
    /// (otherwise the whole audio mixer stays silently dead until restart).
    #[test]
    fn add_layer_re_enables_after_stop_all() {
        let mut mixer = AudioMixer::new();
        mixer.enabled = false; // state left behind by Mixer::stop_all
        let (_tx, rx) = flume::unbounded();
        assert!(mixer.add_layer("sfx".into(), rx, 1.0).is_ok());
        assert!(mixer.enabled, "add_layer must re-enable the mixer");
        assert_eq!(mixer.layers.len(), 1);

        // A disabled-but-populated mixer must still modify the frame, proving the
        // re-enable actually restores the mixing path.
        mixer.enabled = true;
        let mut frame = [0i16, 0, 0, 0];
        mixer.mix(&mut frame); // populates layer state; no panic
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
            assert_eq!(out.signum(), sum.signum() as i16, "preserves sign at {sum}");
            if sum > -29492 {
                assert!(out >= prev, "monotonic at {sum}");
            }
            prev = out;
        }
        // Beyond the knee the curve saturates but keeps sign and ordering.
        assert!(soft_clip_i16(-1_000_000) < soft_clip_i16(-29492));
        assert!(soft_clip_i16(1_000_000) > soft_clip_i16(29492));
        // At extreme magnitudes, finite-precision arithmetic rounds the asymptote to full scale.
        assert_eq!(soft_clip_i16(1_000_000), i16::MAX);
    }

    fn make_track_mixer() -> (
        Mixer,
        flume::Sender<AudioFrame>,
        Arc<AtomicU8>,
        Arc<AtomicU32>,
        Arc<AtomicU64>,
    ) {
        let mut mixer = Mixer::new(TARGET_SAMPLE_RATE);
        let (frame_tx, frame_rx) = flume::unbounded::<AudioFrame>();
        let state = Arc::new(AtomicU8::new(PlaybackState::Playing as u8));
        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let position = Arc::new(AtomicU64::new(0));
        let is_buffering = Arc::new(AtomicBool::new(false));
        let mut config = PlayerConfig::default();
        config.tape.tape_stop_duration_ms = 200;
        mixer.add_track(
            frame_rx,
            state.clone(),
            volume.clone(),
            position.clone(),
            is_buffering,
            config,
        );
        (mixer, frame_tx, state, volume, position)
    }

    fn feed_loud(tx: &flume::Sender<AudioFrame>, frames: usize) {
        for _ in 0..frames {
            let pcm: Vec<i16> = (0..FRAME_SIZE_SAMPLES)
                .map(|i| if i % 2 == 0 { 8_000 } else { -8_000 })
                .collect();
            tx.send(AudioFrame::Pcm(pcm)).unwrap();
        }
    }

    fn mix_mean_abs(mixer: &mut Mixer) -> i64 {
        let mut buf = vec![0i16; FRAME_SIZE_SAMPLES];
        mixer.mix(&mut buf);
        buf.iter().map(|&x| (x as i32).abs() as i64).sum::<i64>() / buf.len() as i64
    }

    /// A `pause()` followed by a `resume()` inside the tape-stop ramp window
    /// (wavelink's default `pause(); resume()` sequence) must not leave the tape
    /// frozen at rate 0.01. Before the fix the completion of the stop ramp moved
    /// the state to `Paused`/`Playing`, so the `Starting` branch never ran again
    /// and playback stayed silently stuck at ~0.01 forever.
    #[test]
    fn tape_recovers_when_resume_lands_mid_stop_ramp() {
        let (mut mixer, tx, state, _volume, _position) = make_track_mixer();
        feed_loud(&tx, 400);

        let mut audible = false;
        for _ in 0..10 {
            if mix_mean_abs(&mut mixer) > 100 {
                audible = true;
            }
        }
        assert!(audible, "baseline playback must be audible");

        // Pause: engage the tape stop ramp.
        state.store(PlaybackState::Stopping as u8, Ordering::Release);
        for _ in 0..3 {
            mix_mean_abs(&mut mixer);
        }

        // Resume mid-ramp.
        state.store(PlaybackState::Starting as u8, Ordering::Release);

        let mut last = 0;
        for _ in 0..80 {
            last = mix_mean_abs(&mut mixer);
        }

        let rate = mixer.tracks[0].flow.tape.rate();
        assert!(rate > 0.99, "tape rate must recover to ~1.0, got {rate}");
        assert!(
            last > 100,
            "audio must be non-silent after resume, got {last}"
        );
    }
}
