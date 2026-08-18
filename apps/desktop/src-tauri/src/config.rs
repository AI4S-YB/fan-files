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
}
