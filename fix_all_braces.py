import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# In restart_at
content = re.sub(
r'''                self\.kizuna_track_handle = Some\(k_handle\);\n\s*\}\n\s*\}\n''',
r'''                self.kizuna_track_handle = Some(k_handle);
            }
''', content)

# In play_track
content = re.sub(
r'''                self\.kizuna_track_handle = Some\(k_handle\);\n\s*\}\n\s*\}\n\s*\}\n''',
r'''                self.kizuna_track_handle = Some(k_handle);
            }
        }
''', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
