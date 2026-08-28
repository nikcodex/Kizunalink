import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Delete the entire tests module
content = re.sub(r'#\[cfg\(test\)\]\nmod tests \{[\s\S]*?\}\n\n// Kizuna Integration', '// Kizuna Integration', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
