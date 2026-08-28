import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Replace the orphaned brace
content = content.replace(
'''        let mut adapter = crate::player::kizuna_adapter::KizunaVoiceAdapter::new(
            merged.session_id.clone(),
            merged.token.clone(),
            merged.endpoint.clone(),
            self.guild_id.clone(),
        );
        
            let _ = adapter
                .connect(self.guild_id.clone(), self.user_id.clone())
                .await;
            self.kizuna_voice_adapter = Some(Arc::new(Mutex::new(adapter)));
        }
        // Driver connect removed''',
'''        let mut adapter = crate::player::kizuna_adapter::KizunaVoiceAdapter::new(
            merged.session_id.clone(),
            merged.token.clone(),
            merged.endpoint.clone(),
        );
        let _ = adapter.connect(self.guild_id.clone(), self.user_id.clone()).await;
        self.kizuna_voice_adapter = Some(std::sync::Arc::new(tokio::sync::Mutex::new(adapter)));
'''
)

# And empty block
content = content.replace(
'''        let disconnect_handler = DisconnectHandler {
            guild_id: self.guild_id.clone(),
            event_tx: self.event_tx.clone(),
        };
        {

        }''', '')

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
