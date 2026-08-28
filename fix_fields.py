import re

with open('src/player/guild_player.rs', 'r') as f:
    content = f.read()

# Fix `track_handle`
content = re.sub(r'        if let Some\(handle\) = &self\.track_handle \{\n\s*let _ = handle\.stop\(\);\n\s*\}\n', '', content)
content = re.sub(r'        self\.track_handle = None;\n', '', content)
content = re.sub(r'        self\.track_handle = Some\(handle\);\n', '', content)
content = re.sub(r'        if let Some\(old_handle\) = &self\.track_handle \{\n\s*let _ = old_handle\.stop\(\);\n', '        if let Some(old_handle) = &self.kizuna_track_handle {\n            let k = old_handle.clone(); tokio::spawn(async move { let _ = k.stop().await; });\n', content)
content = re.sub(r'        if let Some\(handle\) = &self\.track_handle \{\n\s*let _ = handle\.set_volume\(volume as f32 / 100\.0\);\n\s*\}\n', '', content)
content = re.sub(r'        } else if let Some\(handle\) = &self\.track_handle \{\n\s*let _ = handle\.set_volume\(volume as f32 / 100\.0\);\n\s*\}\n', '', content)
content = re.sub(r'        if let Some\(handle\) = &self\.track_handle \{\n\s*let result = if pause \{\n\s*handle\.pause\(\)\n\s*\} else \{\n\s*handle\.play\(\)\n\s*\};\n\s*if let Err\(e\) = result \{\n\s*warn!\("Failed to set pause state: \{\:\?\}", e\);\n\s*return false;\n\s*\}\n\s*\}\n', '', content)

# Fix `driver`
content = re.sub(r'        let mut driver_lock = self\.driver\.lock\(\)\.await;\n\s*let handle = driver_lock\.play\(Track::new\(input\)\);\n\s*drop\(driver_lock\);\n', '', content)
content = re.sub(r'        let mut driver_lock = self\.driver\.lock\(\)\.await;\n\s*driver_lock\.add_global_event\(CoreEvent::DriverDisconnect\.into\(\), disconnect_handler\);\n', '', content)
content = re.sub(r'        let mut driver_lock = self\.driver\.lock\(\)\.await;\n\s*if let Err\(e\) = driver_lock\.connect\(info\)\.await \{\n.*?drop\(driver_lock\);\n', '', content, flags=re.DOTALL)

# Fix `KizunaVoiceAdapter::new` arguments 
# Error E0061: this function takes 4 arguments but 3 arguments were supplied
# `KizunaVoiceAdapter::new(merged.session_id.clone(), merged.token.clone(), merged.endpoint.clone())` missing `guild_id`
content = re.sub(
r'''        let mut adapter = crate::player::kizuna_adapter::KizunaVoiceAdapter::new\(\n\s*merged\.session_id\.clone\(\),\n\s*merged\.token\.clone\(\),\n\s*merged\.endpoint\.clone\(\),\n\s*\);''',
r'''        let mut adapter = crate::player::kizuna_adapter::KizunaVoiceAdapter::new(
            merged.session_id.clone(),
            merged.token.clone(),
            merged.endpoint.clone(),
            self.guild_id.clone(),
        );''', content)

with open('src/player/guild_player.rs', 'w') as f:
    f.write(content)


with open('src/dsp/pipeline.rs', 'r') as f:
    pipe = f.read()

# Error E0433: cannot find type `KizunaFilteredSource` in this scope
# Need to add `pub struct KizunaFilteredSource` to `src/dsp/pipeline.rs`!
# Wait, did I delete `KizunaFilteredSource` while pruning `pipeline.rs`?
# Yes! `KizunaFilteredSource` was defined below `create_filtered_input` and I might have deleted it!
# Wait, let's restore it.
if 'struct KizunaFilteredSource' not in pipe:
    pipe += '''
use kizuna_voice::audio::{AudioSource, AudioFrame};
use async_trait::async_trait;
use std::io::Read;

pub struct KizunaFilteredSource {
    reader: FilteredAudioReader,
}

impl KizunaFilteredSource {
    pub fn new(reader: FilteredAudioReader) -> Self {
        Self { reader }
    }
}

#[async_trait]
impl AudioSource for KizunaFilteredSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        let mut buf = [0u8; 7680];
        let mut total_read = 0;
        
        while total_read < buf.len() {
            match self.reader.read(&mut buf[total_read..]) {
                Ok(0) => break,
                Ok(n) => total_read += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(kizuna_voice::error::Error::Connection(e.to_string())),
            }
        }
        
        if total_read == 0 {
            return Ok(None);
        }
        
        let num_samples = total_read / 4;
        let mut samples = Vec::with_capacity(num_samples);
        for chunk in buf[..total_read].chunks_exact(4) {
            let f = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let s = (f * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            samples.push(s);
        }
        
        Ok(Some(AudioFrame::Pcm(samples)))
    }
}
'''
with open('src/dsp/pipeline.rs', 'w') as f:
    f.write(pipe)
