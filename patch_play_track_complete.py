import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# We need to find play_track and inject the KIZUNA_VOICE initialization.
target = '''        let mut driver_lock = self.driver.lock().await;
        let handle = driver_lock.play(Track::new(input));
        drop(driver_lock);'''

replacement = '''        let mut driver_lock = self.driver.lock().await;
        let handle = driver_lock.play(Track::new(input));
        drop(driver_lock);

        if std::env::var("KIZUNA_VOICE").unwrap_or_default() == "1" {
            if let Some(adapter_arc) = &self.kizuna_voice_adapter {
                // To prove the architecture, we rebuild the source for Kizuna
                if let Ok(k_source) = crate::dsp::pipeline::create_kizuna_source(
                    crate::config::http_client(),
                    stream_url.clone(),
                    None,
                    self.shared_chain.clone(),
                    0,
                ).await {
                    use std::sync::Arc;
                    use tokio::sync::Mutex;
                    let k_src = Arc::new(Mutex::new(k_source));
                    let mut adapter = adapter_arc.lock().await;
                    let k_handle = adapter.play_source(k_src, self.user_id.clone());
                    
                    let guild_id = self.guild_id.clone();
                    let tx = self.track_end_tx.clone();
                    let kh_clone = k_handle.clone();
                    
                    // TrackEndNotifier replacement loop
                    tokio::spawn(async move {
                        while let Ok(event) = kh_clone.next_event().await {
                            if matches!(event, kizuna_voice::audio::TrackEvent::Ended | kizuna_voice::audio::TrackEvent::Error(_)) {
                                let _ = tx.send(guild_id.clone());
                                break;
                            }
                        }
                    });
                    
                    self.kizuna_track_handle = Some(k_handle);
                }
            }
        }'''

content = content.replace(target, replacement)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
