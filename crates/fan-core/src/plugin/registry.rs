use super::WasmPlugin;
use fan_plugin_sdk::{BioMetadata, FileContext, FormatInfo, PluginType};
use std::path::PathBuf;
use tracing::info;

pub struct PluginRegistry {
    plugins_dir: PathBuf,
    format_detectors: Vec<WasmPlugin>,
    context_interpreters: Vec<WasmPlugin>,
}

impl PluginRegistry {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self {
            plugins_dir,
            format_detectors: vec![],
            context_interpreters: vec![],
        }
    }

    pub fn discover(&mut self) -> Result<usize, Box<dyn std::error::Error>> {
        let dir = &self.plugins_dir;
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
            return Ok(0);
        }
        let mut count = 0;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "wasm").unwrap_or(false) {
                match WasmPlugin::load(&path) {
                    Ok(plugin) => {
                        info!("Loaded plugin: {} ({})", plugin.info.name, path.display());
                        match plugin.info.plugin_type {
                            PluginType::FormatDetector => self.format_detectors.push(plugin),
                            PluginType::ContextInterpreter => {
                                self.context_interpreters.push(plugin)
                            }
                        }
                        count += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to load plugin {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }
        // Sort by priority (highest first)
        self.format_detectors
            .sort_by_key(|p| 100 - p.info.priority);
        self.context_interpreters
            .sort_by_key(|p| 100 - p.info.priority);
        Ok(count)
    }

    pub fn detect_format(&self, path: &str, magic: &[u8]) -> Option<FormatInfo> {
        for plugin in &self.format_detectors {
            if let Some(info) = plugin.detect_format(path, magic) {
                return Some(info);
            }
        }
        None
    }

    pub fn interpret(&self, ctx: &FileContext) -> Vec<(String, f64, BioMetadata)> {
        self.context_interpreters
            .iter()
            .filter_map(|p| {
                let meta = p.interpret_context(ctx)?;
                Some((p.info.name.clone(), 0.8, meta))
            })
            .collect()
    }
}
