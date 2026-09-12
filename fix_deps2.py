import toml
import os

workspace_cargo = "/root/kizunalink/Cargo.toml"
server_cargo = "/root/kizunalink/kizuna-server/Cargo.toml"
link_cargo = "/root/kizunalink/kizunalink/Cargo.toml"

w_data = toml.load(workspace_cargo)
s_data = toml.load(server_cargo)
l_data = toml.load(link_cargo)

s_deps = s_data.get("dependencies", {})
l_deps = l_data.get("dependencies", {})

shared_deps = {}

for dep, s_val in list(s_deps.items()):
    if dep in l_deps:
        l_val = l_deps[dep]
        if s_val == l_val:
            shared_deps[dep] = s_val

if "workspace" not in w_data:
    w_data["workspace"] = {}

w_data["workspace"]["dependencies"] = shared_deps

for dep in shared_deps:
    s_deps[dep] = {"workspace": True}
    l_deps[dep] = {"workspace": True}

s_data["dependencies"] = s_deps
l_data["dependencies"] = l_deps

with open(workspace_cargo, "w") as f:
    toml.dump(w_data, f)
with open(server_cargo, "w") as f:
    toml.dump(s_data, f)
with open(link_cargo, "w") as f:
    toml.dump(l_data, f)
