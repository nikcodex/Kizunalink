import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

content = re.sub(r'#\[async_trait::async_trait\]\n', '', content)
content = re.sub(r'use std::num::NonZeroU64;\n', '', content)
content = re.sub(r'use std::sync::Arc;\n', '', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)


with open('src/dsp/pipeline.rs', 'r') as f:
    content = f.read()

content = content.replace('use super::decoder::{AudioDecoder, ChannelByteSource, TARGET_SAMPLE_RATE};', 'use super::decoder::{AudioDecoder, ChannelByteSource};')

with open('src/dsp/pipeline.rs', 'w') as f:
    f.write(content)

