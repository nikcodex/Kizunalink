import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Delete classify_disconnect
content = re.sub(r'fn classify_disconnect.*?\n\}\n', '', content, flags=re.DOTALL)
# Delete ws_closed_event_json
content = re.sub(r'pub fn ws_closed_event_json.*?\n\}\n', '', content, flags=re.DOTALL)
# Delete DisconnectHandler struct and impl
content = re.sub(r'#\[derive\(Clone\)\]\nstruct DisconnectHandler.*?\n\}\n', '', content, flags=re.DOTALL)
# Delete DisconnectHandler instantiation inside set_voice
content = re.sub(r'        let disconnect_handler = DisconnectHandler \{\n\s*guild_id: self\.guild_id\.clone\(\),\n\s*event_tx: self\.event_tx\.clone\(\),\n\s*\};\n', '', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)

with open('src/dsp/pipeline.rs', 'r') as f:
    content = f.read()

# Delete duplicate std::io::Read
content = content.replace('use std::io::Read;\n', '')

with open('src/dsp/pipeline.rs', 'w') as f:
    f.write(content)

