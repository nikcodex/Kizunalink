with open("kizuna-voice/src/gateway/connection.rs", "r") as f:
    lines = f.readlines()

new_lines = []
in_resume = False
in_identify = False
dave_added = False

for line in lines:
    if "pub async fn send_identify" in line:
        in_identify = True
        in_resume = False
    elif "pub async fn send_resume" in line:
        in_resume = True
        in_identify = False
    
    if "dave_protocol_version: Some(1)," in line:
        if in_resume:
            continue # Drop it from resume
        if in_identify:
            if dave_added:
                continue # Drop duplicates
            dave_added = True
    
    new_lines.append(line)

with open("kizuna-voice/src/gateway/connection.rs", "w") as f:
    f.writelines(new_lines)

