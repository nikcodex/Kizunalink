import re

with open("src/player/guild_player.rs", "r") as f:
    code = f.read()

# Fix stop_handle_silently
new_stop = """    fn stop_handle_silently(&mut self) {
        if let Some(handle) = self.kizuna_track_handle.take() {
            tokio::spawn(async move {
                let _ = handle.stop().await;
            });
        }
    }"""

code = re.sub(r'    fn stop_handle_silently\(\&mut self\) \{\}', new_stop, code)

with open("src/player/guild_player.rs", "w") as f:
    f.write(code)
