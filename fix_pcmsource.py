import re

with open('src/player/kizuna_adapter.rs', 'r') as f:
    content = f.read()

content = content.replace(
'''#[async_trait]
impl<R: Read + Send + Sync> AudioSource for PcmSourceWrapper<R> {
    async fn next_frame(&mut self) -> KzResult<Option<AudioFrame>> {
        // Read 1920 i16 samples (3840 bytes)
        let mut buf = [0u8; 3840];
        let mut total_read = 0;
        
        while total_read < buf.len() {
            match self.reader.read(&mut buf[total_read..]) {
                Ok(0) => break, // EOF
                Ok(n) => total_read += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(kizuna_voice::error::Error::Connection(e.to_string())),
            }
        }
        
        if total_read == 0 {
            return Ok(None);
        }
        
        let mut samples = Vec::with_capacity(total_read / 2);
        for chunk in buf[..total_read].chunks_exact(2) {
            samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        
        Ok(Some(AudioFrame::Pcm(samples)))
    }
}''',
'''#[async_trait]
impl<R: Read + Send + Sync> AudioSource for PcmSourceWrapper<R> {
    async fn next_frame(&mut self) -> KzResult<Option<AudioFrame>> {
        // Read 1920 f32 samples (7680 bytes)
        let mut buf = [0u8; 7680];
        let mut total_read = 0;
        
        while total_read < buf.len() {
            match self.reader.read(&mut buf[total_read..]) {
                Ok(0) => break, // EOF
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
            // Convert f32 to i16
            let s = (f * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            samples.push(s);
        }
        
        Ok(Some(AudioFrame::Pcm(samples)))
    }
}'''
)

with open('src/player/kizuna_adapter.rs', 'w') as f:
    f.write(content)
