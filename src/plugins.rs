use rhai::{Engine, EvalAltResult, Scope};
use std::fs;
use std::path::Path;
use tracing::{info, warn};

pub struct PluginManager {
    pub engine: Engine,
    pub loaded_plugins: usize,
}

impl PluginManager {
    pub fn new() -> Self {
        let plugin_dir = Path::new("plugins");
        if !plugin_dir.exists() {
            if let Err(e) = fs::create_dir(plugin_dir) {
                warn!("Failed to create plugins directory: {}", e);
            } else {
                info!("Created plugins/ directory for Script extensions");
            }
        }

        Self {
            engine: Engine::new(),
            loaded_plugins: 0,
        }
    }

    pub fn load_all(&mut self) {
        let plugin_dir = Path::new("plugins");
        if let Ok(entries) = fs::read_dir(plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("rhai") {
                    if let Ok(script) = fs::read_to_string(&path) {
                        // Compile it to make sure there are no syntax errors
                        if let Ok(_ast) = self.engine.compile(&script) {
                            info!("🚀 Loaded Plugin: {:?}", path.file_name().unwrap());
                            self.loaded_plugins += 1;
                        } else {
                            warn!("❌ Failed to compile plugin: {:?}", path.file_name().unwrap());
                        }
                    }
                }
            }
        }
        
        if self.loaded_plugins > 0 {
            info!("Successfully loaded {} active plugins", self.loaded_plugins);
        }
    }

    pub fn execute_search(&self, prefix: &str, query: &str) -> Option<String> {
        let plugin_name = prefix.strip_suffix("search").unwrap_or(prefix);
        let path = format!("plugins/{}.rhai", plugin_name);
        
        if Path::new(&path).exists() {
            if let Ok(script) = fs::read_to_string(&path) {
                let mut scope = Scope::new();
                scope.push("query", query.to_string());
                
                if let Ok(result) = self.engine.eval_with_scope::<String>(&mut scope, &script) {
                    return Some(result);
                }
            }
        }
        None
    }
}
