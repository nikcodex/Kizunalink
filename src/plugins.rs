use rhai::{Engine, Scope, AST};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use tracing::{info, warn};

/// Native plugin loaded from a .so/.dylib shared library.
///
/// The library is kept alive for the process lifetime. Function pointers are
/// obtained via `libloading` and stored as raw `usize` to avoid lifetime issues
/// between `Library` and `Symbol`.
pub struct NativePlugin {
    name: String,
    _lib: libloading::Library,
    search_fn: usize,
    free_fn: Option<usize>,
}

// SAFETY: The loaded library is process-lifetime, and the function pointers are
// only called through our safe `search()` wrapper which serializes access.
unsafe impl Send for NativePlugin {}
unsafe impl Sync for NativePlugin {}

impl NativePlugin {
    /// Load a native plugin from a shared library file.
    ///
    /// The shared library must export:
    /// - `plugin_search(query_ptr: *const u8, query_len: usize) -> *mut u8`
    ///   Returns a null-terminated UTF-8 string allocated with malloc.
    /// - `plugin_free?(ptr: *mut u8)` (optional) Frees the string returned by search.
    unsafe fn load(path: &std::path::Path) -> Result<Self, String> {
        let lib = libloading::Library::new(path)
            .map_err(|e| format!("Failed to load {:?}: {}", path, e))?;

        let search_sym: libloading::Symbol<'_, unsafe extern "C" fn(*const u8, usize) -> *mut u8> =
            lib.get(b"plugin_search")
                .map_err(|e| format!("No 'plugin_search' export in {:?}: {}", path, e))?;

        let free_sym: Option<libloading::Symbol<'_, unsafe extern "C" fn(*mut u8)>> =
            lib.get(b"plugin_free").ok();

        let search_fn = *search_sym as usize;
        let free_fn = free_sym.map(|s| *s as usize);

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Self {
            name,
            _lib: lib,
            search_fn,
            free_fn,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Call the native search function. Input: query string. Output: JSON result string.
    pub fn search(&self, query: &str) -> Option<String> {
        let input = query.as_bytes();
        let search_fn = unsafe {
            std::mem::transmute::<usize, unsafe extern "C" fn(*const u8, usize) -> *mut u8>(
                self.search_fn,
            )
        };

        let result_ptr = unsafe { search_fn(input.as_ptr(), input.len()) };
        if result_ptr.is_null() {
            return None;
        }

        // Read the null-terminated string from the pointer
        let result = unsafe {
            let mut len = 0;
            while *result_ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(result_ptr, len);
            String::from_utf8_lossy(slice).to_string()
        };

        // Free the allocated string if the plugin provides a free function
        if let Some(free_fn_addr) = self.free_fn {
            let free_fn = unsafe {
                std::mem::transmute::<usize, unsafe extern "C" fn(*mut u8)>(free_fn_addr)
            };
            unsafe { free_fn(result_ptr) };
        }

        Some(result)
    }
}

pub struct PluginManager {
    pub engine: Engine,
    pub loaded_plugins: usize,
    /// Pre-compiled AST cache: plugin_name -> AST
    compiled_scripts: HashMap<String, AST>,
    /// Native plugins loaded from .so/.dylib files
    native_plugins: Vec<NativePlugin>,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
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
            native_plugins: Vec::new(),
        }
    }

    pub fn load_all(&mut self) {
        let plugin_dir = Path::new("plugins");
        if let Ok(entries) = fs::read_dir(plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                match path.extension().and_then(OsStr::to_str) {
                    Some("rhai") => self.load_rhai_script(&path),
                    Some("so") | Some("dylib") | Some("dll") => self.load_native_plugin(&path),
                    _ => {}
                }
            }
        }

        if self.loaded_plugins > 0 {
            info!(
                "Loaded {} plugins (Rhai: {}, Native: {})",
                self.loaded_plugins,
                self.compiled_scripts.len(),
                self.native_plugins.len()
            );
        }
    }

    fn load_rhai_script(&mut self, path: &Path) {
        if let Ok(script) = fs::read_to_string(path) {
            match self.engine.compile(&script) {
                Ok(ast) => {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    info!("Loaded Rhai plugin: {:?}", path.file_name().unwrap());
                    self.compiled_scripts.insert(name, ast);
                    self.loaded_plugins += 1;
                }
                Err(e) => {
                    warn!(
                        "Failed to compile plugin {:?}: {}",
                        path.file_name().unwrap(),
                        e
                    );
                }
            }
        }
    }

    fn load_native_plugin(&mut self, path: &Path) {
        unsafe {
            match NativePlugin::load(path) {
                Ok(plugin) => {
                    info!(
                        "Loaded native plugin: {} from {:?}",
                        plugin.name(),
                        path.file_name().unwrap()
                    );
                    self.native_plugins.push(plugin);
                    self.loaded_plugins += 1;
                }
                Err(e) => {
                    warn!(
                        "Failed to load native plugin {:?}: {}",
                        path.file_name().unwrap(),
                        e
                    );
                }
            }
        }
    }

    pub fn execute_search(&self, prefix: &str, query: &str) -> Option<String> {
        // Try native plugins first (higher priority)
        for plugin in &self.native_plugins {
            if plugin.name() == prefix || prefix.starts_with(plugin.name()) {
                if let Some(result) = plugin.search(query) {
                    return Some(result);
                }
            }
        }

        // Fall back to Rhai scripts
        let plugin_name = prefix
            .strip_suffix("search")
            .filter(|s| !s.is_empty())
            .unwrap_or(prefix);

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
