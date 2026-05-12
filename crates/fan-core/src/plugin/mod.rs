use fan_plugin_sdk::{BioMetadata, FileContext, FormatInfo, PluginInfo, PluginType};
use std::path::Path;
use wasmtime::{Config, Engine, Instance, Memory, Module, Store};
use tracing::{debug, warn};

pub mod registry;

/// A loaded WASM plugin with pre-compiled module.
/// Instantiates a fresh Store + Instance per invocation for memory isolation.
#[allow(dead_code)]
pub struct WasmPlugin {
    pub info: PluginInfo,
    engine: Engine,
    module: Module,
    has_detect: bool,
    has_interpret: bool,
}

impl WasmPlugin {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = Config::default();
        config.async_support(false);
        let engine = Engine::new(&config)?;
        let module = Module::from_file(&engine, path)?;

        let has_detect = module.exports().any(|e| e.name() == "can_handle");
        let has_interpret = module.exports().any(|e| e.name() == "score");

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let info = PluginInfo {
            name: stem.to_string(),
            version: "0.1.0".into(),
            description: String::new(),
            plugin_type: if has_detect {
                PluginType::FormatDetector
            } else {
                PluginType::ContextInterpreter
            },
            priority: 50,
        };

        debug!(
            "Loaded WASM plugin: {} (detect={}, interpret={})",
            stem, has_detect, has_interpret
        );
        Ok(Self {
            info,
            engine,
            module,
            has_detect,
            has_interpret,
        })
    }

    pub fn detect_format(&self, path: &str, magic: &[u8]) -> Option<FormatInfo> {
        if !self.has_detect {
            return None;
        }
        self.call_detect(path, magic).unwrap_or_else(|e| {
            warn!("WASM detect failed for {}: {}", self.info.name, e);
            None
        })
    }

    pub fn interpret_context(&self, ctx: &FileContext) -> Option<BioMetadata> {
        if !self.has_interpret {
            return None;
        }
        self.call_interpret(ctx).unwrap_or_else(|e| {
            warn!("WASM interpret failed for {}: {}", self.info.name, e);
            None
        })
    }

    fn instantiate(&self) -> Result<(Store<()>, Instance, Memory), Box<dyn std::error::Error>> {
        let mut store = Store::new(&self.engine, ());
        let instance = Instance::new(&mut store, &self.module, &[])?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or("no exported memory in WASM module")?;
        Ok((store, instance, memory))
    }

    fn get_output(
        &self,
        store: &mut Store<()>,
        instance: &Instance,
        memory: &Memory,
        out_len: i32,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if out_len <= 0 || out_len > 4096 {
            return Ok(None);
        }
        // Call output_buffer() to get the address of the plugin's static buffer
        let out_buf_fn = instance
            .get_typed_func::<(), i32>(&mut *store, "output_buffer")
            .map_err(|_| "plugin does not export output_buffer")?;
        let out_ptr = out_buf_fn.call(&mut *store, ())? as usize;

        let mut out_buf = vec![0u8; out_len as usize];
        memory.read(&mut *store, out_ptr, &mut out_buf)?;
        // Trim trailing null if present
        if let Some(pos) = out_buf.iter().position(|&b| b == 0) {
            out_buf.truncate(pos);
        }
        let json = String::from_utf8_lossy(&out_buf).into_owned();
        Ok(Some(json))
    }

    fn call_detect(
        &self,
        path: &str,
        magic: &[u8],
    ) -> Result<Option<FormatInfo>, Box<dyn std::error::Error>> {
        let (mut store, instance, memory) = self.instantiate()?;

        // Write path and magic to linear memory at fixed offset
        let path_ptr = 1024usize;
        let path_bytes = path.as_bytes();
        memory.write(&mut store, path_ptr, path_bytes)?;

        let magic_ptr = path_ptr + path_bytes.len() + 1; // +1 for null separator
        memory.write(&mut store, magic_ptr, magic)?;

        // 1. Check can_handle
        let can_handle = instance.get_typed_func::<(i32, i32, i32, i32), i32>(
            &mut store,
            "can_handle",
        )?;
        let can = can_handle.call(
            &mut store,
            (
                path_ptr as i32,
                path_bytes.len() as i32,
                magic_ptr as i32,
                magic.len() as i32,
            ),
        )?;
        if can == 0 {
            return Ok(None);
        }

        // 2. Call detect
        let detect_fn = instance.get_typed_func::<(i32, i32, i32, i32), i32>(
            &mut store,
            "detect",
        )?;
        let out_len = detect_fn.call(
            &mut store,
            (
                path_ptr as i32,
                path_bytes.len() as i32,
                magic_ptr as i32,
                magic.len() as i32,
            ),
        )?;

        // 3. Read output from the plugin's static buffer
        if let Some(json) = self.get_output(&mut store, &instance, &memory, out_len)? {
            let info: FormatInfo = serde_json::from_str(&json)?;
            return Ok(Some(info));
        }

        Ok(None)
    }

    fn call_interpret(
        &self,
        ctx: &FileContext,
    ) -> Result<Option<BioMetadata>, Box<dyn std::error::Error>> {
        let (mut store, instance, memory) = self.instantiate()?;

        // Write path to linear memory (extract function reads the path string)
        let path_ptr = 1024usize;
        let path_bytes = ctx.file_path.as_bytes();
        memory.write(&mut store, path_ptr, path_bytes)?;

        // 1. Check score
        let score_fn = instance.get_typed_func::<(i32, i32), f64>(
            &mut store,
            "score",
        )?;
        let score = score_fn.call(
            &mut store,
            (path_ptr as i32, path_bytes.len() as i32),
        )?;
        if score < 0.3 {
            return Ok(None);
        }

        // 2. Call extract
        let extract_fn = instance.get_typed_func::<(i32, i32), i32>(
            &mut store,
            "extract",
        )?;
        let out_len = extract_fn.call(
            &mut store,
            (path_ptr as i32, path_bytes.len() as i32),
        )?;

        // 3. Read output from the plugin's static buffer
        if let Some(json) = self.get_output(&mut store, &instance, &memory, out_len)? {
            let meta: BioMetadata = serde_json::from_str(&json)?;
            return Ok(Some(meta));
        }

        Ok(None)
    }
}
