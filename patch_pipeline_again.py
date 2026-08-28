import re

with open('src/dsp/pipeline.rs', 'r') as f:
    content = f.read()

# Remove the function entirely
content = re.sub(r'pub async fn create_filtered_input[\s\S]*?\}\n\n', '', content)

# Remove comments mentioning songbird
content = re.sub(r'///.*songbird.*\n', '', content)

with open('src/dsp/pipeline.rs', 'w') as f:
    f.write(content)
