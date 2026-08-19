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

pub(crate) fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".fan-files/config.toml")
}

/// 从指定路径读取 config。文件不存在时返回全空默认值（GUI 首次启动场景），
/// 其余 IO/解析错误仍返回 Err。
pub(crate) fn read_config_at(path: &Path) -> Result<FanConfig, String> {
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
///
/// read-modify-write：只重写 GUI 拥有的键（threads、[scan] include/exclude、
/// [llm] endpoint/api_key/model），保留文件中的其他配置节（如 CLI 的
/// [servers] 远程服务器注册表、[daemon]、[watch] 等），避免 GUI 保存一次
/// 就静默删除 CLI 配置。文件不存在或无法解析为 TOML 表时从空表开始。
pub(crate) fn write_config_at(path: &Path, cfg: FanConfig) -> Result<(), String> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(raw) => match toml::from_str::<toml::Value>(&raw) {
            Ok(toml::Value::Table(t)) => t,
            _ => toml::map::Map::new(), // unparseable → start fresh
        },
        // 文件不存在 → 空表起步（GUI 首次保存场景）；其他 IO 错误 → 返回 Err，
        // 不得覆盖不可读的既有文件。
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::map::Map::new(),
        Err(e) => return Err(e.to_string()),
    };
    // only touch the keys the GUI owns
    match cfg.threads {
        Some(t) => {
            root.insert("threads".into(), toml::Value::Integer(t as i64));
        }
        None => {
            root.remove("threads");
        }
    }
    root.insert(
        "scan".into(),
        toml::Value::Table({
            let mut scan = toml::map::Map::new();
            scan.insert(
                "include".into(),
                toml::Value::Array(
                    cfg.include
                        .iter()
                        .map(|s| toml::Value::String(s.clone()))
                        .collect(),
                ),
            );
            scan.insert(
                "exclude".into(),
                toml::Value::Array(
                    cfg.exclude
                        .iter()
                        .map(|s| toml::Value::String(s.clone()))
                        .collect(),
                ),
            );
            scan
        }),
    );
    root.insert(
        "llm".into(),
        toml::Value::Table({
            let mut llm = toml::map::Map::new();
            llm.insert("endpoint".into(), toml::Value::String(cfg.endpoint.clone()));
            llm.insert("api_key".into(), toml::Value::String(cfg.api_key.clone()));
            llm.insert("model".into(), toml::Value::String(cfg.model.clone()));
            llm
        }),
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, toml::to_string_pretty(&toml::Value::Table(root)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

// ---------- [transfer] 段读写（GUI 传输参数，与 CLI 共享同一文件） ----------

/// 把 serde_json::Value 转 toml::Value（JSON null 跳过——TOML 没有 null）。
fn json_to_toml(v: &serde_json::Value) -> Option<toml::Value> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(toml::Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(toml::Value::Integer(i))
            } else {
                n.as_f64().map(toml::Value::Float)
            }
        }
        serde_json::Value::String(s) => Some(toml::Value::String(s.clone())),
        serde_json::Value::Array(a) => Some(toml::Value::Array(
            a.iter().filter_map(json_to_toml).collect(),
        )),
        serde_json::Value::Object(o) => {
            let mut t = toml::map::Map::new();
            for (k, v) in o {
                if let Some(tv) = json_to_toml(v) {
                    t.insert(k.clone(), tv);
                }
            }
            Some(toml::Value::Table(t))
        }
    }
}

/// 读取 config.toml 的 [transfer] 段。文件缺失 / 无该段时返回全默认值，
/// 其余 IO/解析错误仍返回 Err。
/// 返回对象始终含四个键：chunk_size_mb(4)/concurrency(4)/receive_dir(null)/udp_enabled(true)。
pub(crate) fn read_transfer_config_at(path: &Path) -> Result<serde_json::Value, String> {
    fn defaults() -> serde_json::Value {
        serde_json::json!({
            "chunk_size_mb": 4,
            "concurrency": 4,
            "receive_dir": serde_json::Value::Null,
            "udp_enabled": true,
        })
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(defaults()),
        Err(e) => return Err(e.to_string()),
    };
    let v: toml::Value = toml::from_str(&raw).map_err(|e| e.to_string())?;
    let mut out = defaults();
    if let Some(toml::Value::Table(tab)) = v.get("transfer") {
        // 已有的键覆盖默认值；缺失的键保留默认（serde default 语义对齐 CLI）
        if let Ok(found) = serde_json::to_value(tab) {
            if let (Some(out), Some(found)) = (out.as_object_mut(), found.as_object()) {
                for (k, v) in found {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
    }
    Ok(out)
}

/// 写 [transfer] 段（read-modify-write，保留其他节——与 write_config_at 同模式）。
/// cfg 必须是 JSON 对象：只合并 cfg 中提供的键，未提供的键保留原文件值；
/// 值为 null 的键删除（TOML 无 null，读取时回退默认）。
pub(crate) fn write_transfer_config_at(path: &Path, cfg: &serde_json::Value) -> Result<(), String> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(raw) => match toml::from_str::<toml::Value>(&raw) {
            Ok(toml::Value::Table(t)) => t,
            _ => toml::map::Map::new(), // unparseable → start fresh
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::map::Map::new(),
        Err(e) => return Err(e.to_string()),
    };
    let mut transfer = match root.get("transfer") {
        Some(toml::Value::Table(t)) => t.clone(),
        _ => toml::map::Map::new(),
    };
    match cfg {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                match json_to_toml(v) {
                    Some(tv) => {
                        transfer.insert(k.clone(), tv);
                    }
                    None => {
                        transfer.remove(k);
                    }
                }
            }
        }
        _ => return Err("transfer 配置必须是 JSON 对象".into()),
    }
    root.insert("transfer".into(), toml::Value::Table(transfer));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, toml::to_string_pretty(&toml::Value::Table(root)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// 从 config.toml [transfer] 段解析 CLI 传输参数，返回 (chunk_size 字节, concurrency)。
/// 缺省 4MB / 4；0 视为未设置（与 CLI resolve_transfer_params 语义一致）。
/// 读失败（文件不可读等）静默回退默认——spawn 传输不能因配置损坏而失败。
pub(crate) fn transfer_cli_params() -> (u64, usize) {
    transfer_cli_params_at(&config_path())
}

/// 同 transfer_cli_params，但路径可注入（测试用）。
pub(crate) fn transfer_cli_params_at(path: &Path) -> (u64, usize) {
    let v = read_transfer_config_at(path).unwrap_or_default();
    let chunk_mb = v
        .get("chunk_size_mb")
        .and_then(|x| x.as_u64())
        .filter(|m| *m > 0)
        .unwrap_or(4);
    let concurrency = v
        .get("concurrency")
        .and_then(|x| x.as_u64())
        .map(|c| c as usize)
        .filter(|c| *c > 0)
        .unwrap_or(4);
    (chunk_mb.saturating_mul(1024 * 1024), concurrency)
}

/// config [transfer].receive_dir（None = 未配置）。
pub(crate) fn configured_receive_dir() -> Option<String> {
    configured_receive_dir_at(&config_path())
}

/// 同 configured_receive_dir，但路径可注入（测试用）。
pub(crate) fn configured_receive_dir_at(path: &Path) -> Option<String> {
    read_transfer_config_at(path)
        .ok()?
        .get("receive_dir")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// 接收默认目录：~/Downloads/fan-received（与前端现有默认一致，后端兜底）。
pub(crate) fn default_receive_dir() -> String {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads/fan-received")
        .to_string_lossy()
        .to_string()
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

    /// 回归：write_config_at 必须 read-modify-write，保留 GUI 不拥有的配置节
    /// （如 CLI 的 [servers.*] 远程服务器注册表与 [daemon]），只更新 threads/[scan]/[llm]。
    #[test]
    fn write_config_preserves_unknown_sections() {
        let p = temp_config_path("preserve-sections");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"threads = 8
[servers.foo]
url = "http://remote:8929"
token = "abc"
[daemon]
port = 3232
[scan]
include = ["/old"]
exclude = []
[llm]
endpoint = "http://old"
api_key = ""
model = "old-model"
"#,
        )
        .unwrap();
        write_config_at(&p, sample_config()).unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(raw.contains("url = \"http://remote:8929\""), "servers.foo.url 应逐字保留:\n{raw}");
        assert!(raw.contains("token = \"abc\""), "servers.foo.token 应逐字保留:\n{raw}");
        assert!(raw.contains("[daemon]"), "[daemon] 节应保留:\n{raw}");
        assert!(raw.contains("port = 3232"), "daemon.port 应保留:\n{raw}");
        // [scan]/[llm] 与 threads 已按 GUI 值更新
        let back = read_config_at(&p).unwrap();
        assert_eq!(back, sample_config());
        cleanup(&p);
    }

    /// threads=None 时从已有文件里删除 threads 键（而非仅在新文件中省略）。
    #[test]
    fn write_config_none_threads_removes_existing_key() {
        let p = temp_config_path("remove-threads");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "threads = 4\n[servers.foo]\nurl = \"http://x\"\n").unwrap();
        let mut dto = sample_config();
        dto.threads = None;
        write_config_at(&p, dto.clone()).unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(!raw.contains("threads"), "threads=None 应删除既有键:\n{raw}");
        assert!(raw.contains("[servers.foo]"), "未知节应保留:\n{raw}");
        assert_eq!(read_config_at(&p).unwrap(), dto);
        cleanup(&p);
    }

    /// 既有文件不可读（非 NotFound）时必须返回 Err，不得静默重建覆盖。
    /// 用 0o200（只写不可读）制造 read 失败但 write 可行的场景：
    /// 若实现吞掉读取错误从空表起步，fs::write 会成功并抹掉原内容。
    #[cfg(unix)]
    #[test]
    fn write_config_read_error_does_not_overwrite() {
        use std::os::unix::fs::PermissionsExt;
        let p = temp_config_path("unreadable");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let original = "threads = 2\n[servers.foo]\nurl = \"http://x\"\n";
        std::fs::write(&p, original).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o200);
        std::fs::set_permissions(&p, perms).unwrap();
        let result = write_config_at(&p, sample_config());
        // 恢复权限以便断言与清理
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&p, perms).unwrap();
        assert!(result.is_err(), "不可读的既有文件应返回 Err 而非覆盖");
        let raw = std::fs::read_to_string(&p).unwrap();
        assert_eq!(raw, original, "不可读的既有文件内容不得被改动");
        cleanup(&p);
    }

    /// 文件存在但无法解析成 TOML 表时从空表开始重建（不报错、不残留旧内容）。
    #[test]
    fn write_config_unparseable_file_starts_fresh() {
        let p = temp_config_path("unparseable");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "not valid toml [[[ ~~~~").unwrap();
        let dto = sample_config();
        write_config_at(&p, dto.clone()).unwrap();
        assert_eq!(read_config_at(&p).unwrap(), dto);
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

    // ---------- [transfer] 段 ----------

    #[test]
    fn read_transfer_config_missing_file_returns_defaults() {
        let p = temp_config_path("transfer-missing");
        cleanup(&p);
        let v = read_transfer_config_at(&p).unwrap();
        assert_eq!(v["chunk_size_mb"], 4);
        assert_eq!(v["concurrency"], 4);
        assert!(v["receive_dir"].is_null());
        assert_eq!(v["udp_enabled"], true);
        cleanup(&p);
    }

    #[test]
    fn read_transfer_config_partial_section_fills_defaults() {
        let p = temp_config_path("transfer-partial");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "threads = 8\n[transfer]\nconcurrency = 8\n").unwrap();
        let v = read_transfer_config_at(&p).unwrap();
        assert_eq!(v["chunk_size_mb"], 4, "未写的 chunk_size_mb 用默认 4");
        assert_eq!(v["concurrency"], 8);
        assert!(v["receive_dir"].is_null());
        assert_eq!(v["udp_enabled"], true);
        cleanup(&p);
    }

    /// 回归：write_transfer_config_at 必须 read-modify-write，保留 [servers]/[scan]/[llm] 等
    /// GUI 不拥有的节；null 字段省略（TOML 无 null），读取回退默认。
    #[test]
    fn write_transfer_config_preserves_other_sections() {
        let p = temp_config_path("transfer-preserve");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"threads = 8
[servers.foo]
url = "http://remote:8929"
[scan]
include = ["/old"]
exclude = []
[llm]
endpoint = "http://old"
api_key = ""
model = "old-model"
"#,
        )
        .unwrap();
        write_transfer_config_at(
            &p,
            &serde_json::json!({
                "chunk_size_mb": 16,
                "concurrency": 8,
                "receive_dir": serde_json::Value::Null,
                "udp_enabled": false,
            }),
        )
        .unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(raw.contains("url = \"http://remote:8929\""), "servers.foo 应保留:\n{raw}");
        assert!(raw.contains("[llm]"), "[llm] 节应保留:\n{raw}");
        assert!(raw.contains("chunk_size_mb = 16"), "chunk_size_mb 应写入:\n{raw}");
        assert!(raw.contains("udp_enabled = false"), "udp_enabled 应写入:\n{raw}");
        assert!(!raw.contains("receive_dir"), "null 的 receive_dir 应省略:\n{raw}");
        let v = read_transfer_config_at(&p).unwrap();
        assert_eq!(v["chunk_size_mb"], 16);
        assert_eq!(v["concurrency"], 8);
        assert_eq!(v["udp_enabled"], false);
        assert!(v["receive_dir"].is_null());
        cleanup(&p);
    }

    /// 写只合并 cfg 提供的键，未提供的键保留原值（前端可只回传修改项）。
    #[test]
    fn write_transfer_config_merges_unspecified_keys() {
        let p = temp_config_path("transfer-merge");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            "[transfer]\nchunk_size_mb = 16\nconcurrency = 8\nreceive_dir = \"/data/inbox\"\nudp_enabled = true\n",
        )
        .unwrap();
        write_transfer_config_at(&p, &serde_json::json!({ "chunk_size_mb": 32 })).unwrap();
        let v = read_transfer_config_at(&p).unwrap();
        assert_eq!(v["chunk_size_mb"], 32);
        assert_eq!(v["concurrency"], 8, "未提供的 concurrency 应保留原值");
        assert_eq!(v["receive_dir"], "/data/inbox");
        assert_eq!(v["udp_enabled"], true);
        cleanup(&p);
    }

    #[test]
    fn write_transfer_config_rejects_non_object() {
        let p = temp_config_path("transfer-nonobject");
        cleanup(&p);
        assert!(write_transfer_config_at(&p, &serde_json::json!(42)).is_err());
        assert!(write_transfer_config_at(&p, &serde_json::json!("str")).is_err());
        cleanup(&p);
    }

    #[test]
    fn transfer_cli_params_from_config() {
        let p = temp_config_path("params");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "[transfer]\nchunk_size_mb = 16\nconcurrency = 3\n").unwrap();
        assert_eq!(transfer_cli_params_at(&p), (16 * 1024 * 1024, 3));
        cleanup(&p);
    }

    #[test]
    fn transfer_cli_params_defaults_when_missing() {
        let p = temp_config_path("params-default");
        cleanup(&p);
        assert_eq!(transfer_cli_params_at(&p), (4 * 1024 * 1024, 4));
        cleanup(&p);
    }

    #[test]
    fn configured_receive_dir_reads_config() {
        let p = temp_config_path("receive-dir");
        cleanup(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "[transfer]\nreceive_dir = \"/data/inbox\"\n").unwrap();
        assert_eq!(
            configured_receive_dir_at(&p).as_deref(),
            Some("/data/inbox")
        );
        cleanup(&p);
        // 未配置 → None
        assert_eq!(configured_receive_dir_at(&p), None);
    }
}
