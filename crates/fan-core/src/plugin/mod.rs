use fan_plugin_sdk::{BioMetadata, FileContext, FormatInfo, PluginInfo, PluginType};
use std::path::Path;
use wasmtime::{Config, Engine, Module};

pub mod registry;

/// A loaded WASM plugin instance
#[allow(dead_code)]
pub struct WasmPlugin {
    pub info: PluginInfo,
    engine: Engine,
    module: Module,
}

impl WasmPlugin {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = Config::default();
        config.async_support(false);
        let engine = Engine::new(&config)?;
        let module = Module::from_file(&engine, path)?;

        // Extract plugin info from WASM exports or derive from filename
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let info = PluginInfo {
            name: stem.to_string(),
            version: "0.1.0".into(),
            description: String::new(),
            plugin_type: PluginType::FormatDetector,
            priority: 50,
        };

        Ok(Self {
            info,
            engine,
            module,
        })
    }

    pub fn detect_format(&self, _path: &str, _magic: &[u8]) -> Option<FormatInfo> {
        // WASM call: invoke exported "can_handle" and "detect" functions
        // For MVP, return None — real WASM plugin calls come later
        None
    }

    pub fn interpret_context(&self, _ctx: &FileContext) -> Option<BioMetadata> {
        // WASM call: invoke exported "score" and "extract" functions
        None
    }
}
