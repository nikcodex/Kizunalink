import re

with open('src/dsp/pipeline.rs', 'r') as f:
    content = f.read()

# Remove create_filtered_input
content = re.sub(r'/// Build a songbird `Input`.*?Result<songbird::input::Input, String> \{\n.*?\}\n\n', '', content, flags=re.DOTALL)

with open('src/dsp/pipeline.rs', 'w') as f:
    f.write(content)
