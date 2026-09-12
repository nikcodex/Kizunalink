import os
import re

def extract_deps(content):
    in_deps = False
    deps = {}
    lines = content.split('\n')
    for line in lines:
        if line.strip().startswith('[dependencies]'):
            in_deps = True
            continue
        if line.strip().startswith('[') and in_deps:
            in_deps = False
        
        if in_deps and line.strip() and not line.strip().startswith('#'):
            parts = line.split('=', 1)
            if len(parts) == 2:
                key = parts[0].strip()
                if key != "kizunalink":
                    deps[key] = parts[1].strip()
    return deps

w_cargo = "/root/kizunalink/Cargo.toml"
s_cargo = "/root/kizunalink/kizuna-server/Cargo.toml"
l_cargo = "/root/kizunalink/kizunalink/Cargo.toml"

with open(w_cargo, "r") as f:
    w_text = f.read()

with open(s_cargo, "r") as f:
    s_text = f.read()

with open(l_cargo, "r") as f:
    l_text = f.read()

s_deps = extract_deps(s_text)
l_deps = extract_deps(l_text)

shared = {}
for k, v in s_deps.items():
    if k in l_deps and s_deps[k] == l_deps[k]:
        shared[k] = v

w_text += "\n[workspace.dependencies]\n"
for k, v in shared.items():
    w_text += f"{k} = {v}\n"

def replace_deps(content, shared):
    lines = content.split('\n')
    out = []
    in_deps = False
    for line in lines:
        if line.strip().startswith('[dependencies]'):
            in_deps = True
            out.append(line)
            continue
        if line.strip().startswith('[') and in_deps:
            in_deps = False
        
        if in_deps and line.strip() and not line.strip().startswith('#'):
            parts = line.split('=', 1)
            if len(parts) == 2:
                key = parts[0].strip()
                if key in shared:
                    out.append(f'{key} = {{ workspace = true }}')
                    continue
        out.append(line)
    return '\n'.join(out)

with open(w_cargo, "w") as f:
    f.write(w_text)

with open(s_cargo, "w") as f:
    f.write(replace_deps(s_text, shared))

with open(l_cargo, "w") as f:
    f.write(replace_deps(l_text, shared))
