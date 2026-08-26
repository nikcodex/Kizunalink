import re

with open("src/plugins.rs", "r") as f:
    content = f.read()

# Register the fetch_json function
new_init = """
        let mut engine = Engine::new();
        
        // Register HTTP GET function for plugins
        engine.register_fn("fetch_json", |url: &str| -> String {
            if let Ok(res) = reqwest::blocking::get(url) {
                if let Ok(text) = res.text() {
                    return text;
                }
            }
            "{}".to_string()
        });

        Self {
            engine,
            loaded_plugins: 0,
        }
"""

content = re.sub(r'Self\s*{\s*engine:\s*Engine::new\(\),\s*loaded_plugins:\s*0,\s*}', new_init.strip(), content)

with open("src/plugins.rs", "w") as f:
    f.write(content)
