import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

target = r'''        let user_num = self\.user_id\.parse::<u64>\(\)\.unwrap_or\(0\);\n\n\s*let guild_nz = NonZeroU64::new\(guild_num\)\.unwrap_or\(NonZeroU64::new\(1\)\.unwrap\(\)\);\n\s*let user_nz = NonZeroU64::new\(user_num\)\.unwrap_or\(NonZeroU64::new\(1\)\.unwrap\(\)\);\n\n\s*let channel_id_val = merged\n\s*\.channel_id\n\s*\.as_deref\(\)\n\s*\.and_then\(\|c\| c\.parse::<u64>\(\)\.ok\(\)\);\n\n\s*let info = ConnectionInfo \{\n\s*endpoint,\n\s*guild_id: GuildId::from\(guild_nz\),\n\s*channel_id: channel_id_val\.map\(\|id\| \{\n\s*let nz = NonZeroU64::new\(id\)\.unwrap_or\(NonZeroU64::new\(1\)\.unwrap\(\)\);\n\s*songbird::id::ChannelId::from\(nz\)\n\s*\}\),\n\s*session_id: merged\.session_id\.clone\(\),\n\s*token: merged\.token\.clone\(\),\n\s*user_id: UserId::from\(user_nz\),\n\s*\};\n'''
replace = ''
content = re.sub(target, replace, content)

content = content.replace('if std::env::var("KIZUNA_VOICE").unwrap_or_default() == "1" {', '')

# There are block closures remaining from `if let Some(adapter_arc) = &self.kizuna_voice_adapter {`
# We should format it, but keeping the block is fine.
# Wait, `if std::env::var("KIZUNA_VOICE").unwrap_or_default() == "1" {` was wrapped around adapter initialization!
adapter_target = r'''        // Driver connect removed\n\n\s*let mut adapter = crate::player::kizuna_adapter::KizunaVoiceAdapter::new\(\n\s*merged\.session_id\.clone\(\),\n\s*merged\.token\.clone\(\),\n\s*merged\.endpoint\.clone\(\),\n\s*\);\n\s*let _ = adapter\.connect\(self\.guild_id\.clone\(\), self\.user_id\.clone\(\)\)\.await;\n\s*self\.kizuna_voice_adapter = Some\(Arc::new\(Mutex::new\(adapter\)\)\);\n\s*\}\n'''
adapter_replace = r'''        let mut adapter = crate::player::kizuna_adapter::KizunaVoiceAdapter::new(
            merged.session_id.clone(),
            merged.token.clone(),
            merged.endpoint.clone(),
        );
        let _ = adapter.connect(self.guild_id.clone(), self.user_id.clone()).await;
        self.kizuna_voice_adapter = Some(Arc::new(Mutex::new(adapter)));
'''
content = re.sub(adapter_target, adapter_replace, content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)
