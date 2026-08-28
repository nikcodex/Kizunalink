import re

with open('src/dsp/pipeline.rs', 'r') as f:
    content = f.read()

# The orphaned code block ends right before #[cfg(test)]
content = re.sub(r'        // Source is Opus 48kHz[\s\S]*?\}\n\}\n', '', content)

with open('src/dsp/pipeline.rs', 'w') as f:
    f.write(content)
