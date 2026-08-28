import re

def repl_file(path, replacements):
    try:
        with open(path, 'r') as f:
            c = f.read()
        for t, r in replacements:
            c = re.sub(t, r, c)
        with open(path, 'w') as f:
            f.write(c)
    except FileNotFoundError:
        pass

# src/player/guild_player.rs
repl_file('src/player/guild_player.rs', [
    (r'use tracing::\{error, info, warn\};', 'use tracing::{info, warn};'),
    (r'use crate::dsp::pipeline::\{self, SharedChain\};', 'use crate::dsp::pipeline;'),
])

# src/player/kizuna_adapter.rs
repl_file('src/player/kizuna_adapter.rs', [
    (r'AudioFrame, AudioSource, FrameScheduler, KizunaTrackHandle, OpusEncoder, OpusSource, TrackEvent,',
     r'AudioFrame, AudioSource, FrameScheduler, KizunaTrackHandle, OpusEncoder, OpusSource,'),
    (r'use kizuna_voice::dave::protocol::\{DaveClientMessage, DaveGatewayMessage, DaveSession\};',
     r'use kizuna_voice::dave::protocol::DaveSession;'),
    (r'use tokio::time::Instant;\n', ''),
    (r'use tracing::\{error, info, warn\};\n', 'use tracing::{error, info};\n'),
])

# src/ratelimit.rs
repl_file('src/ratelimit.rs', [
    (r'use tokio::sync::RwLock;\n', ''),
    (r'use tracing::warn;\n', ''),
])

# src/rest/decodetrack.rs
repl_file('src/rest/decodetrack.rs', [
    (r'use crate::security;\n', ''),
])

# src/ws/handler.rs
repl_file('src/ws/handler.rs', [
    (r'use crate::security;\n', ''),
])

# src/dsp/timescale.rs
repl_file('src/dsp/timescale.rs', [
    (r'if let Ok\(mut wave_out\) = resampler\.process\(\&wave_in, None\) \{', 'if let Ok(wave_out) = resampler.process(&wave_in, None) {'),
    (r'if let Ok\(mut wave_out\) = resampler\.process\(\&silence, None\) \{', 'if let Ok(wave_out) = resampler.process(&silence, None) {'),
])

# src/dsp/wsola.rs
repl_file('src/dsp/wsola.rs', [
    (r'fn find_best_offset\(\&self, analysis_start: usize, analysis_hop: i64\) -> usize', 'fn find_best_offset(&self, analysis_start: usize, _analysis_hop: i64) -> usize'),
])

# kizuna-voice/src/audio/demuxer.rs
repl_file('kizuna-voice/src/audio/demuxer.rs', [
    (r'use std::time::Duration;\n', ''),
])

# kizuna-voice/src/audio/opus.rs
repl_file('kizuna-voice/src/audio/opus.rs', [
    (r'use crate::error::\{Error, Result\};', 'use crate::error::Result;'),
])

# kizuna-voice/src/audio/scheduler.rs
repl_file('kizuna-voice/src/audio/scheduler.rs', [
    (r'use tracing::\{debug, error, info, warn\};', 'use tracing::warn;'),
    (r'use tracing::\{debug, error, warn\};', 'use tracing::warn;'),
])

