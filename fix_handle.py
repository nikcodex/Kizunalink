import re
with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Remove handle.add_event
content = re.sub(r'\s*let _ = handle\.add_event\([\s\S]*?\}\,\n\s*\);', '', content)

# Remove handle.set_volume
content = re.sub(r'\s*if let Err\(e\) = handle\.set_volume\(self\.volume as f32 / 100\.0\) \{\n\s*warn!\("Failed to set volume during restart: \{\:\?\}", e\);\n\s*\}', '', content)

# Remove handle.seek
content = re.sub(r'\s*let _ = handle\.seek\(Duration::from_millis\(position\)\);', '', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
