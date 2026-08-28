import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

target = r'''        // Filters are active — must decode to PCM, apply DSP, re-encode[\s\S]*?            \}\n        \}\n    \}\n'''
content = re.sub(target, '', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
