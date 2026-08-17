use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// GUI 视角的 ~/.fan-files/config.toml（与 CLI 共享同一文件）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FanConfig {
    pub threads: Option<usize>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".fan-files/config.toml")
}

/// 从指定路径读取 config。文件不存在时返回全空默认值（GUI 首次启动场景），
/// 其余 IO/解析错误仍返回 Err。
fn read_config_at(path: &Path) -> Result<FanConfig, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FanConfig::default()),
        Err(e) => return Err(e.to_string()),
    };
    let v: toml::Value = toml::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(FanConfig {
        threads: v
            .get("threads")
            .and_then(|t| t.as_integer())
            .map(|t| t as usize),
        include: v
            .get("scan")
            .and_then(|s| s.get("include"))
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        exclude: v
            .get("scan")
            .and_then(|s| s.get("exclude"))
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        endpoint: v
            .get("llm")
            .and_then(|l| l.get("endpoint"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
        api_key: v
            .get("llm")
            .and_then(|l| l.get("api_key"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
        model: v
            .get("llm")
            .and_then(|l| l.get("model"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
    })
}

/// 将 config 写入指定路径（自动创建父目录）。
fn write_config_at(path: &Path, cfg: FanConfig) -> Result<(), String> {
    // toml::toml! 只接受字面量 token（不支持表达式），故显式构建 Value，
    // 输出结构与 CLI 一致：threads、[scan] include/exclude、[llm] endpoint/api_key/model。
    let mut scan = toml::map::Map::new();
    scan.insert("include".into(), toml::Value::Array(cfg.include.iter().map(|s| toml::Value::String(s.clone())).collect()));
    scan.insert("exclude".into(), toml::Value::Array(cfg.exclude.iter().map(|s| toml::Value::String(s.clone())).collect()));
    let mut llm = toml::map::Map::new();
    llm.insert("endpoint".into(), toml::Value::String(cfg.endpoint));
    llm.insert("api_key".into(), toml::Value::String(cfg.api_key));
    llm.insert("model".into(), toml::Value::String(cfg.model));
    let mut root = toml::map::Map::new();
    if let Some(threads) = cfg.threads {
        root.insert("threads".into(), toml::Value::Integer(threads as i64));
    }
    root.insert("scan".into(), toml::Value::Table(scan));
    root.insert("llm".into(), toml::Value::Table(llm));
    let v = toml::Value::Table(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, toml::to_string_pretty(&v).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

// 注意：Tauri 2 的 #[tauri::command] 对 `pub fn` 会生成 #[macro_export] 宏 + 同模块
// `pub use` 重导出，放在 crate root 时两者宏命名空间冲突（E0255），故命令保持私有
// （Tauri 2 官方示例亦然）。前端 invoke 按字符串名调用，与可见性无关。
#[tauri::command]
fn read_config() -> Result<FanConfig, String> {
    read_config_at(&config_path())
}

#[tauri::command]
fn write_config(cfg: FanConfig) -> Result<(), String> {
    write_config_at(&config_path(), cfg)
}

#[tauri::command]
fn fan_home() -> Result<String, String> {
    Ok(config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_string_lossy()
        .to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![read_config, write_config, fan_home])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个测试独享的临时 config.toml 路径（在系统临时目录下按进程号+用例名隔离）。
    fn temp_config_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("fan-files-config-test-{}-{}", std::process::id(), name))
            .join("config.toml")
    }

    fn cleanup(p: &Path) {
        if let Some(parent) = p.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    fn sample_config() -> FanConfig {
        FanConfig {
            threads: Some(10),
            include: vec!["/data/kentnf/orders".into()],
            exclude: vec!["*.tmp".into()],
            endpoint: "http://182.92.166.143:3200/v1/chat/completions".into(),
            api_key: "sk-test".into(),
            model: "DSv4-flash".into(),
        }
    }

    #[test]
    fn config_dto_roundtrip_keeps_fields() {
        let dto = sample_config();
        let s = serde_json::to_string(&dto).unwrap();
        let back: FanConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.include, dto.include);
        assert_eq!(back.model, dto.model);
    }

    #[test]
    fn read_config_missing_file_returns_defaults() {
        let p = temp_config_path("missing");
        cleanup(&p);
        let cfg = read_config_at(&p).unwrap();
        assert_eq!(cfg, FanConfig::default());
    }

    #[test]
    fn write_then_read_config_roundtrip() {
        let p = temp_config_path("roundtrip");
        cleanup(&p);
        let dto = sample_config();
        write_config_at(&p, dto.clone()).unwrap();
        let back = read_config_at(&p).unwrap();
        assert_eq!(back, dto);
        cleanup(&p);
    }

    #[test]
    fn write_config_without_threads_omits_key() {
        let p = temp_config_path("no-threads");
        cleanup(&p);
        let mut dto = sample_config();
        dto.threads = None;
        write_config_at(&p, dto.clone()).unwrap();
        let back = read_config_at(&p).unwrap();
        assert_eq!(back, dto);
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(!raw.contains("threads"), "threads=None 应省略该键:\n{raw}");
        cleanup(&p);
    }

    /// 对照 bioinfo7 上真实 ~/.fan-files/config.toml 的结构。
    #[test]
    fn read_config_parses_cli_layout() {
        let p = temp_config_path("cli-layout");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"threads = 10
[scan]
include = ["/data/kentnf/orders"]
exclude = ["/data/kentnf/orders/z_syns_fdr", "*.tmp"]
[llm]
endpoint = "http://182.92.166.143:3200/v1/chat/completions"
api_key = "sk-jgv6-example"
model = "DSv4-flash"
"#,
        )
        .unwrap();
        let cfg = read_config_at(&p).unwrap();
        assert_eq!(cfg.threads, Some(10));
        assert_eq!(cfg.include, vec!["/data/kentnf/orders"]);
        assert_eq!(
            cfg.exclude,
            vec!["/data/kentnf/orders/z_syns_fdr", "*.tmp"]
        );
        assert_eq!(cfg.endpoint, "http://182.92.166.143:3200/v1/chat/completions");
        assert_eq!(cfg.api_key, "sk-jgv6-example");
        assert_eq!(cfg.model, "DSv4-flash");
        cleanup(&p);
    }
}
