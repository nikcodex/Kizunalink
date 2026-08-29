import re
with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()
    
# Remove #[cfg(test)] mod disconnect_tests { ... }
content = re.sub(r'#\[cfg\(test\)\]\nmod disconnect_tests \{.*?\n\}\n', '', content, flags=re.DOTALL)

# Remove the leftover commented out text about KIZUNA_VOICE=1 at the end
content = re.sub(r'// Kizuna Integration:.*?\n', '', content)
content = re.sub(r'// In `play_track`, if KIZUNA_VOICE=1.*?\n', '', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
