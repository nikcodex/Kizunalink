import re

with open('src/player/guild_player.rs', 'r') as f:
    lines = f.readlines()

new_lines = []
skip = False
brace_count = 0
for line in lines:
    if skip:
        if '{' in line:
            brace_count += line.count('{')
        if '}' in line:
            brace_count -= line.count('}')
        if brace_count <= 0:
            skip = False
        continue

    # Identify if let Some(handle) = &self.track_handle
    if 'if let Some(handle) = &self.track_handle {' in line:
        skip = True
        brace_count = line.count('{') - line.count('}')
        if brace_count <= 0:
            skip = False
        continue
    if '} else if let Some(handle) = &self.track_handle {' in line:
        skip = True
        brace_count = line.count('{') - line.count('}')
        if brace_count <= 0:
            skip = False
        continue

    # Identify driver
    if 'let mut driver_lock = self.driver.lock().await;' in line:
        continue
    if 'let handle = driver_lock.play(Track::new(input));' in line:
        continue
    if 'drop(driver_lock);' in line:
        continue
    if 'self.track_handle = Some(handle);' in line:
        continue
    
    new_lines.append(line)

with open('src/player/guild_player.rs', 'w') as f:
    f.writelines(new_lines)
