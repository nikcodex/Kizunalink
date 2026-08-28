import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Modify play_track
play_track_regex = r'(let mut driver_lock = self\.driver\.lock\(\)\.await;\n\s*let handle = driver_lock\.play\(Track::new\(input\)\);\n\s*drop\(driver_lock\);\n\n\s*let _ = handle\.add_event\(\n\s*Event::Track\(TrackEvent::End\),\n\s*TrackEndNotifier \{\n\s*guild_id: self\.guild_id\.clone\(\),\n\s*track_end_tx: self\.track_end_tx\.clone\(\),\n\s*\},\n\s*\);\n\n\s*if let Err\(e\) = handle\.set_volume\(self\.volume as f32 / 100\.0\) \{\n\s*warn!\("Failed to set volume on handle: \{\:\?\}", e\);\n\s*\})'

replacement = '''
        if std::env::var("KIZUNA_VOICE").unwrap_or_default() == "1" {
            if let Some(adapter_arc) = &self.kizuna_voice_adapter {
                // We need to bypass build_input for Kizuna
                // Wait, play_track already called build_input!
                // To avoid rewriting play_track heavily, we can spawn a task to listen to Kizuna events
                // Let's implement it cleanly below.
            }
        }
\\1
'''
# Actually, it's easier to modify `stop_handle_silently` and `stop` and `pause` to also call `kizuna_track_handle`.
