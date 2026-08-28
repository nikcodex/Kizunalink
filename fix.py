import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Add to struct
content = re.sub(
    r'(pub driver: Arc<Mutex<Driver>>,\n)',
    r'\1    pub kizuna_voice_adapter: Option<Arc<Mutex<crate::player::kizuna_adapter::KizunaVoiceAdapter>>>,\n',
    content
)

# Add to new()
content = re.sub(
    r'(driver: Arc::new\(Mutex::new\(driver\)\),\n)',
    r'\1            kizuna_voice_adapter: None,\n',
    content
)

# Replace in set_voice (and ONLY set_voice)
# We find set_voice by looking for `pub async fn set_voice`
parts = content.split('pub async fn set_voice')
if len(parts) == 2:
    set_voice = parts[1]
    # find the first `let mut driver_lock = self.driver.lock().await;`
    set_voice = set_voice.replace(
        'let mut driver_lock = self.driver.lock().await;',
        '''
        let mut adapter = crate::player::kizuna_adapter::KizunaVoiceAdapter::new(
            merged.session_id.clone(),
            merged.token.clone(),
            merged.endpoint.clone(),
            self.guild_id.clone(),
        );
        if std::env::var("KIZUNA_VOICE").unwrap_or_default() == "1" {
            let _ = adapter.connect(self.guild_id.clone(), self.user_id.clone()).await;
            self.kizuna_voice_adapter = Some(Arc::new(Mutex::new(adapter)));
        }
        let mut driver_lock = self.driver.lock().await;''',
        1  # Only first occurrence
    )
    content = parts[0] + 'pub async fn set_voice' + set_voice

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
