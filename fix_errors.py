import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Restore imports
content = 'use std::sync::Arc;\nuse tokio::sync::Mutex;\n' + content
content = content.replace('use crate::dsp::pipeline;', 'use crate::dsp::pipeline::{self, SharedChain};')

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
