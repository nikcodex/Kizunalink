use rhai::{Engine, Scope, AST};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{info, warn};

pub struct PluginManager {
    pub engine: Engine,
    pub loaded_plugins: usize,
    /// Pre-compiled AST cache: plugin_name -> AST
    compiled_scripts: HashMap<String, AST>,
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

        let mut engine = Engine::new();

        // Register HTTP GET function for plugins
        // Uses tokio::task::block_in_place to avoid stalling the async runtime
        engine.register_fn("fetch_json", |url: &str| -> String {
            let url_owned = url.to_string();
            // block_in_place tells Tokio this will block, allowing it to
            // schedule other tasks on remaining worker threads
            tokio::task::block_in_place(|| {
                if let Ok(res) = reqwest::blocking::get(&url_owned) {
                    if let Ok(text) = res.text() {
                        return text;
                    }
                }
                "{}".to_string()
            })
        });

        Self {
            engine,
            loaded_plugins: 0,
            compiled_scripts: HashMap::new(),
        }
    }

    pub fn load_all(&mut self) {
        let plugin_dir = Path::new("plugins");
        if let Ok(entries) = fs::read_dir(plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("rhai") {
                    if let Ok(script) = fs::read_to_string(&path) {
                        match self.engine.compile(&script) {
                            Ok(ast) => {
                                let name = path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                info!("🚀 Loaded Plugin: {:?}", path.file_name().unwrap());
                                self.compiled_scripts.insert(name, ast);
                                self.loaded_plugins += 1;
                            }
                            Err(e) => {
                                warn!(
                                    "❌ Failed to compile plugin {:?}: {}",
                                    path.file_name().unwrap(),
                                    e
                                );
                            }
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

        // Use pre-compiled AST from cache instead of reading from disk every time
        if let Some(ast) = self.compiled_scripts.get(plugin_name) {
            let mut scope = Scope::new();
            scope.push("query", query.to_string());

            match self.engine.eval_ast_with_scope::<String>(&mut scope, ast) {
                Ok(result) => return Some(result),
                Err(e) => {
                    warn!("Plugin '{}' execution error: {}", plugin_name, e);
                }
            }
        }
        None
    }
}
