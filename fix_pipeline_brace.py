import re

with open('src/dsp/pipeline.rs', 'r') as f:
    content = f.read()

# I will find where create_kizuna_source starts, and remove everything above it back to the end of `symphonia::core::io::MediaSource for FilteredAudioReader`

content = re.sub(
r'impl symphonia::core::io::MediaSource for FilteredAudioReader \{\n\s*fn is_seekable\(\&self\) -> bool \{\n\s*false\n\s*\}\n\n\s*fn byte_len\(\&self\) -> Option<u64> \{\n\s*None\n\s*\}\n\}[\s\S]*?pub async fn create_kizuna_source',
r'''impl symphonia::core::io::MediaSource for FilteredAudioReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

pub async fn create_kizuna_source''', content)

with open('src/dsp/pipeline.rs', 'w') as f:
    f.write(content)
