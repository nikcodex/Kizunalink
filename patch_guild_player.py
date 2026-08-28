import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Replace stop_handle_silently
content = content.replace(
'''    pub fn stop_handle_silently(&mut self) {
        if let Some(handle) = self.track_handle.take() {
            let _ = handle.stop();
        }
    }''',
'''    pub fn stop_handle_silently(&mut self) {
        if let Some(handle) = self.track_handle.take() {
            let _ = handle.stop();
        }
        if let Some(k_handle) = self.kizuna_track_handle.take() {
            tokio::spawn(async move {
                let _ = k_handle.stop().await;
            });
        }
    }'''
)

content = content.replace(
'''        if was_paused {
            let _ = handle.pause();''',
'''        if was_paused {
            let _ = handle.pause();
            if let Some(k_handle) = &self.kizuna_track_handle {
                let k = k_handle.clone();
                tokio::spawn(async move { let _ = k.pause().await; });
            }'''
)

content = content.replace(
'''        if let Err(e) = handle.set_volume(self.volume as f32 / 100.0) {
            warn!("Failed to set volume on handle: {:?}", e);
        }''',
'''        if let Err(e) = handle.set_volume(self.volume as f32 / 100.0) {
            warn!("Failed to set volume on handle: {:?}", e);
        }
        if let Some(k_handle) = &self.kizuna_track_handle {
            let k = k_handle.clone();
            let vol = self.volume as f32 / 100.0;
            tokio::spawn(async move { let _ = k.set_volume(vol).await; });
        }'''
)

content = content.replace(
'''    pub async fn set_pause(&mut self, pause: bool) -> bool {
        if let Some(handle) = &self.track_handle {
            let result = if pause {
                handle.pause()
            } else {
                handle.play()
            };''',
'''    pub async fn set_pause(&mut self, pause: bool) -> bool {
        if let Some(k_handle) = &self.kizuna_track_handle {
            let k = k_handle.clone();
            tokio::spawn(async move {
                if pause { let _ = k.pause().await; } else { let _ = k.resume().await; }
            });
        }
        if let Some(handle) = &self.track_handle {
            let result = if pause {
                handle.pause()
            } else {
                handle.play()
            };'''
)

content = content.replace(
'''    pub fn update_volume(&mut self, volume: u32) {
        self.volume = volume;
        if let Some(handle) = &self.track_handle {
            let _ = handle.set_volume(volume as f32 / 100.0);
        }
    }''',
'''    pub fn update_volume(&mut self, volume: u32) {
        self.volume = volume;
        if let Some(handle) = &self.track_handle {
            let _ = handle.set_volume(volume as f32 / 100.0);
        }
        if let Some(k_handle) = &self.kizuna_track_handle {
            let k = k_handle.clone();
            tokio::spawn(async move { let _ = k.set_volume(volume as f32 / 100.0).await; });
        }
    }'''
)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)

