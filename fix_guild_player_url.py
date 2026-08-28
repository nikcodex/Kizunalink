import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# We need to find the `create_kizuna_source` call inside `restart_at` and change stream_url to url.
# Let's just find the `restart_at` block and replace it.
parts = content.split('pub async fn restart_at(&mut self, position_ms: u64) {')
if len(parts) == 2:
    p2 = parts[1].split('pub async fn play_track')
    if len(p2) == 2:
        restart_at = p2[0]
        restart_at = restart_at.replace('stream_url.clone()', 'url.clone()')
        content = parts[0] + 'pub async fn restart_at(&mut self, position_ms: u64) {' + restart_at + 'pub async fn play_track' + p2[1]

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
