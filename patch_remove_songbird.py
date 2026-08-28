import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# 1. Remove songbird imports
content = re.sub(r'use songbird::.*?\n', '', content)

# 2. Remove songbird types from GuildPlayer struct
content = re.sub(r'    pub driver: Arc<Mutex<Driver>>,\n', '', content)
content = re.sub(r'    pub track_handle: Option<TrackHandle>,\n', '', content)

# 3. Clean up GuildPlayer::new
content = re.sub(r'        let driver = Driver::new\(Default::default\(\)\);\n', '', content)
content = re.sub(r'            driver: Arc::new\(Mutex::new\(driver\)\),\n', '', content)
content = re.sub(r'            track_handle: None,\n', '', content)

# 4. In `update_voice`, remove driver connection entirely.
# We just need to initialize KizunaVoiceAdapter.
update_voice_target = r'''        let mut driver_lock = self\.driver\.lock\(\)\.await;\n\s*if let Err\(e\) = driver_lock\.connect\(info\)\.await \{\n.*?drop\(driver_lock\);'''
update_voice_replace = '''        // Driver connect removed'''
content = re.sub(update_voice_target, update_voice_replace, content, flags=re.DOTALL)

# 5. Remove Disconnect handler registering
disconnect_target = r'''            let mut driver_lock = self\.driver\.lock\(\)\.await;\n\s*driver_lock\.add_global_event\(CoreEvent::DriverDisconnect\.into\(\), disconnect_handler\);'''
content = re.sub(disconnect_target, '', content)

# 6. Remove TrackEndNotifier completely (including struct and impl)
notifier_target = r'''struct TrackEndNotifier \{\n.*?None\n    \}\n\}'''
content = re.sub(notifier_target, '', content, flags=re.DOTALL)
# Also remove DisconnectHandler implementation if it was Songbird EventHandler
disconnect_handler_target = r'''impl EventHandler for DisconnectHandler \{\n.*?None\n    \}\n\}'''
content = re.sub(disconnect_handler_target, '''impl DisconnectHandler {
    fn handle_disconnect(&self, reason: &Option<String>) -> String {
        let payload = ws_closed_event_json(&self.guild_id, reason);
        let json = payload.to_string();
        let _ = self.event_tx.send(json.clone());
        json
    }
}''', content, flags=re.DOTALL)

# 7. Modify build_input to just return KizunaFilteredSource or remove it,
# since play_track now uses create_kizuna_source directly. 
# We'll remove build_input later, let's fix restart_at and play_track first.

# 8. In restart_at
restart_at_target = r'''        let mut driver_lock = self\.driver\.lock\(\)\.await;\n\s*let handle = driver_lock\.play\(Track::new\(input\)\);\n\s*drop\(driver_lock\);\n\n\s*if std::env::var\("KIZUNA_VOICE"\)\.unwrap_or_default\(\) == "1" \{\n\s*if let Some\(adapter_arc\) = &self\.kizuna_voice_adapter \{\n\s*// To prove the architecture, we rebuild the source for Kizuna\n\s*if let Ok\(k_source\) = crate::dsp::pipeline::create_kizuna_source\(\n\s*crate::config::http_client\(\),\n\s*url\.clone\(\),\n\s*None,\n\s*self\.shared_chain\.clone\(\),\n\s*0,\n\s*\)\n\s*\.await\n\s*\{\n\s*use std::sync::Arc;\n\s*use tokio::sync::Mutex;\n\s*let k_src = Arc::new\(Mutex::new\(k_source\)\);\n\s*let mut adapter = adapter_arc\.lock\(\)\.await;\n\s*let k_handle = adapter\.play_source\(k_src, self\.user_id\.clone\(\)\);\n\n\s*let guild_id = self\.guild_id\.clone\(\);\n\s*let tx = self\.track_end_tx\.clone\(\);\n\s*let kh_clone = k_handle\.clone\(\);\n\n\s*tokio::spawn\(async move \{\n\s*while let Ok\(event\) = kh_clone\.next_event\(\)\.await \{\n\s*if matches!\(event, kizuna_voice::audio::TrackEvent::Ended \| kizuna_voice::audio::TrackEvent::Error\(_\)\) \{\n\s*let _ = tx\.send\(guild_id\.clone\(\)\);\n\s*break;\n\s*\}\n\s*\}\n\s*\}\);\n\n\s*self\.kizuna_track_handle = Some\(k_handle\);\n\s*\}\n\s*\}\n\s*\}'''

restart_at_replace = r'''
        if let Some(adapter_arc) = &self.kizuna_voice_adapter {
            if let Ok(k_source) = crate::dsp::pipeline::create_kizuna_source(
                crate::config::http_client(),
                url.clone(),
                None,
                self.shared_chain.clone(),
                0,
            )
            .await
            {
                use std::sync::Arc;
                use tokio::sync::Mutex;
                let k_src = Arc::new(Mutex::new(k_source));
                let mut adapter = adapter_arc.lock().await;
                let k_handle = adapter.play_source(k_src, self.user_id.clone());

                let guild_id = self.guild_id.clone();
                let tx = self.track_end_tx.clone();
                let kh_clone = k_handle.clone();

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
'''
content = re.sub(restart_at_target, restart_at_replace, content)

# 9. In play_track
play_track_target = r'''        let mut driver_lock = self\.driver\.lock\(\)\.await;\n\s*let handle = driver_lock\.play\(Track::new\(input\)\);\n\s*drop\(driver_lock\);\n\n\s*if std::env::var\("KIZUNA_VOICE"\)\.unwrap_or_default\(\) == "1" \{\n\s*if let Some\(adapter_arc\) = &self\.kizuna_voice_adapter \{\n\s*// To prove the architecture, we rebuild the source for Kizuna\n\s*if let Ok\(k_source\) = crate::dsp::pipeline::create_kizuna_source\(\n\s*crate::config::http_client\(\),\n\s*stream_url\.clone\(\),\n\s*None,\n\s*self\.shared_chain\.clone\(\),\n\s*0,\n\s*\)\n\s*\.await\n\s*\{\n\s*use std::sync::Arc;\n\s*use tokio::sync::Mutex;\n\s*let k_src = Arc::new\(Mutex::new\(k_source\)\);\n\s*let mut adapter = adapter_arc\.lock\(\)\.await;\n\s*let k_handle = adapter\.play_source\(k_src, self\.user_id\.clone\(\)\);\n\n\s*let guild_id = self\.guild_id\.clone\(\);\n\s*let tx = self\.track_end_tx\.clone\(\);\n\s*let kh_clone = k_handle\.clone\(\);\n\n\s*tokio::spawn\(async move \{\n\s*while let Ok\(event\) = kh_clone\.next_event\(\)\.await \{\n\s*if matches!\(event, kizuna_voice::audio::TrackEvent::Ended \| kizuna_voice::audio::TrackEvent::Error\(_\)\) \{\n\s*let _ = tx\.send\(guild_id\.clone\(\)\);\n\s*break;\n\s*\}\n\s*\}\n\s*\}\);\n\n\s*self\.kizuna_track_handle = Some\(k_handle\);\n\s*\}\n\s*\}\n\s*\}'''
play_track_replace = r'''
        if let Some(adapter_arc) = &self.kizuna_voice_adapter {
            if let Ok(k_source) = crate::dsp::pipeline::create_kizuna_source(
                crate::config::http_client(),
                stream_url.clone(),
                None,
                self.shared_chain.clone(),
                0,
            )
            .await
            {
                use std::sync::Arc;
                use tokio::sync::Mutex;
                let k_src = Arc::new(Mutex::new(k_source));
                let mut adapter = adapter_arc.lock().await;
                let k_handle = adapter.play_source(k_src, self.user_id.clone());

                let guild_id = self.guild_id.clone();
                let tx = self.track_end_tx.clone();
                let kh_clone = k_handle.clone();

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
'''
content = re.sub(play_track_target, play_track_replace, content)

# 10. Clean stop, pause, resume
content = re.sub(r'        if let Some\(handle\) = self\.track_handle\.take\(\) \{\n\s*let _ = handle\.stop\(\);\n\s*\}\n', '', content)
content = re.sub(r'        if let Some\(handle\) = &self\.track_handle \{\n\s*let _ = handle\.set_volume\(volume as f32 / 100\.0\);\n\s*\}\n', '', content)
content = re.sub(r'        if let Some\(handle\) = &self\.track_handle \{\n\s*let result = if pause \{\n\s*handle\.pause\(\)\n\s*\} else \{\n\s*handle\.play\(\)\n\s*\};\n\s*if let Err\(e\) = result \{\n\s*warn!\("Failed to set pause state: \{\:\?\}", e\);\n\s*return false;\n\s*\}\n\s*\}\n', '', content)
content = re.sub(r'            let _ = handle\.pause\(\);\n', '', content)
content = re.sub(r'        if let Err\(e\) = handle\.set_volume\(self\.volume as f32 / 100\.0\) \{\n\s*warn!\("Failed to set volume on handle: \{\:\?\}", e\);\n\s*\}\n', '', content)

# 11. Remove build_input call in restart_at and play_track, and build_input method
content = re.sub(r'        let \(input, filtered\) = self\.build_input\(&url, position_ms\)\.await;\n', '        let filtered = self.shared_chain.lock().unwrap().is_active();\n', content)
content = re.sub(r'        let \(input, filtered\) = self\.build_input\(&stream_url, 0\)\.await;\n', '        let filtered = self.shared_chain.lock().unwrap().is_active();\n', content)
content = re.sub(r'    async fn build_input[\s\S]*?\}', '', content)

# 12. Fix the DisconnectReason imports which were Songbird
content = re.sub(r'use songbird::events::context_data::DisconnectReason as DR;\n', '', content)
content = re.sub(r'use songbird::model::CloseCode;\n', '', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
