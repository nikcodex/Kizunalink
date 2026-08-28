import re

with open('src/player/kizuna_adapter.rs', 'r') as f:
    content = f.read()

# Fix encoder capture
content = re.sub(
    r'let mut encoder = OpusEncoder::new\(\)\.unwrap\(\);\n\n\s*scheduler\n\s*\.run\(cmd_rx, event_tx, \|frame\| \{\n\s*let udp = udp\.clone\(\);\n\s*let dave = dave\.clone\(\);\n\s*let sender_id_clone = sender_id\.clone\(\);',
    '''let encoder = std::sync::Arc::new(tokio::sync::Mutex::new(OpusEncoder::new().unwrap()));
            scheduler.run(cmd_rx, event_tx, |frame| {
                let udp = udp.clone();
                let dave = dave.clone();
                let sender_id_clone = sender_id.clone();
                let enc_clone = encoder.clone();''',
    content,
    flags=re.MULTILINE
)

content = re.sub(
    r'AudioFrame::Pcm\(pcm\) => \{\n\s*let encoded = encoder\.encode\(OpusSource::Pcm\(pcm\)\)\.unwrap\(\);',
    r'''AudioFrame::Pcm(pcm) => {
                            let mut enc = enc_clone.try_lock().unwrap();
                            let encoded = enc.encode(OpusSource::Pcm(pcm)).unwrap();''',
    content
)

# Fix stream_url in restart_at
with open('src/player/guild_player.rs', 'r') as f:
    guild_content = f.read()

parts = guild_content.split('pub async fn restart_at(&mut self, position_ms: u64) {')
if len(parts) == 2:
    p2 = parts[1].split('pub async fn play_track')
    if len(p2) == 2:
        restart_at = p2[0]
        # replace the specific `stream_url.clone()` that was incorrectly injected
        restart_at = restart_at.replace('stream_url.clone()', 'url.clone()')
        guild_content = parts[0] + 'pub async fn restart_at(&mut self, position_ms: u64) {' + restart_at + 'pub async fn play_track' + p2[1]

with open('src/player/guild_player.rs', 'w') as f:
    f.write(guild_content)

with open('src/player/kizuna_adapter.rs', 'w') as f:
    f.write(content)
