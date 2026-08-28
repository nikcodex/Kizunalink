import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

replacement = '''        if std::env::var("KIZUNA_VOICE").unwrap_or_default() == "1" {
            let reader = input.into_inner(); // Assuming Input has an inner we can extract, or wait.
'''

# We need to construct AudioSource. 
# `FilteredAudioReader` implements `Read`. We can wrap it. 
# Actually `build_input` returns `(songbird::input::Input, bool)`.
