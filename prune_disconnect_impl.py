import re
with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

content = re.sub(r'impl DisconnectHandler \{\n.*?\}\n\}\n', '', content, flags=re.DOTALL)
content = re.sub(r'use context_data::DisconnectReason as DR;\n', '', content)
content = re.sub(r'use context_data::DisconnectReason;\n', '', content)
with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
