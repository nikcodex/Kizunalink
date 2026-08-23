use std::fs;
use std::path::Path;
use tracing::{info, warn};

// In the future, we will add `extism` here to load .wasm files.
// For now, this is the cross-platform Plugin Manager scaffold.

pub struct PluginManager {
    pub loaded_plugins: usize,
}

impl PluginManager {
    pub fn new() -> Self {
        // Ensure the plugins directory exists across all operating systems
        let plugin_dir = Path::new("plugins");
        if !plugin_dir.exists() {
            if let Err(e) = fs::create_dir(plugin_dir) {
                warn!("Failed to create plugins directory: {}", e);
            } else {
                info!("Created plugins/ directory for WASM extensions");
            }
        }

        Self { loaded_plugins: 0 }
    }

    pub fn load_all(&mut self) {
        let plugin_dir = Path::new("plugins");
        if let Ok(entries) = fs::read_dir(plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("wasm") {
                    info!("Discovered WASM plugin: {:?}", path.file_name().unwrap());
                    // Next step: Load into Extism Sandbox
                    self.loaded_plugins += 1;
                }
            }
        }
        
        if self.loaded_plugins > 0 {
            info!("Successfully loaded {} WASM plugins", self.loaded_plugins);
        }
    }
}
