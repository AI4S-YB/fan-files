use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub scan: ScanConfig,
    #[serde(default)]
    pub watch: WatchConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub plugins: PluginConfig,
    #[serde(default)]
    pub retention: RetentionConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub threads: Option<usize>,
    #[serde(default)]
    pub servers: ServersConfig,
    #[serde(default)]
    pub transfer: TransferConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_socket")]
    pub socket: PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket: default_socket(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub external_api_url: Option<String>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            external_api_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(default = "default_plugin_dir")]
    pub dir: PathBuf,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            dir: default_plugin_dir(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    #[serde(default = "default_retention_days")]
    pub deleted_keep_days: u32,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            deleted_keep_days: default_retention_days(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    #[serde(default = "default_sync_time")]
    pub full_sync: String,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            full_sync: default_sync_time(),
        }
    }
}

/// 服务器注册表配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServersConfig {
    #[serde(flatten)]
    pub servers: std::collections::HashMap<String, ServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// SSH Host 名（~/.ssh/config 中定义），空字符串 = 本地
    pub host: String,
    /// 扫描根目录（支持多个路径）
    #[serde(default)]
    pub scan_roots: Vec<String>,
    /// 人类可读的描述（可选）
    #[serde(default)]
    pub label: Option<String>,
    /// 是否参与扫描
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

/// 传输配置（config.toml [transfer] 段）
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransferConfig {
    /// 块大小（MB，默认 4）
    #[serde(default = "default_chunk_size_mb")]
    pub chunk_size_mb: u64,
    /// 并发传输数（默认 4）
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// 接收目录（None → 系统默认：当前目录）
    #[serde(default)]
    pub receive_dir: Option<String>,
    /// 是否启用 UDP 打洞直连（默认 true；false 等价 FAN_NO_UDP=1）
    #[serde(default = "default_true")]
    pub udp_enabled: bool,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            chunk_size_mb: default_chunk_size_mb(),
            concurrency: default_concurrency(),
            receive_dir: None,
            udp_enabled: default_true(),
        }
    }
}

fn default_chunk_size_mb() -> u64 { 4 }
fn default_concurrency() -> usize { 4 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default = "default_api_type")]
    pub api_type: String,   // "openai" | "anthropic"
}

fn default_llm_model() -> String {
    "gpt-4o-mini".into()
}

/// 旧 config.toml 无 api_type → 默认 openai（OpenAI 兼容协议）
fn default_api_type() -> String {
    "openai".into()
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            api_key: String::new(),
            model: default_llm_model(),
            api_type: default_api_type(),
        }
    }
}

fn default_socket() -> PathBuf {
    dirs_fan().join("fan.sock")
}
fn default_model() -> String {
    "all-MiniLM-L6-v2".into()
}
fn default_plugin_dir() -> PathBuf {
    dirs_fan().join("plugins")
}
fn default_retention_days() -> u32 {
    30
}
fn default_sync_time() -> String {
    "03:00".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig {
                socket: default_socket(),
            },
            scan: ScanConfig {
                include: vec![],
                exclude: vec![
                    "/tmp".into(), "*.tmp".into(),
                    "/proc".into(), "/sys".into(), "/dev".into(), "/run".into(),
                    "/etc".into(), "/bin".into(), "/sbin".into(), "/lib".into(), "/lib64".into(),
                    "/boot".into(), "/snap".into(), "/var/cache".into(), "/var/log".into(),
                    "/lost+found".into(), "/root".into(), "/cdrom".into(), "/media".into(),
                ],
            },
            watch: WatchConfig {
                include: vec![],
                exclude: vec!["*.tmp".into(), ".*".into()],
            },
            embedding: EmbeddingConfig {
                model: default_model(),
                external_api_url: None,
            },
            plugins: PluginConfig {
                dir: default_plugin_dir(),
            },
            retention: RetentionConfig {
                deleted_keep_days: default_retention_days(),
            },
            schedule: ScheduleConfig {
                full_sync: default_sync_time(),
            },
            llm: LlmConfig::default(),
            threads: None,
            servers: ServersConfig::default(),
            transfer: TransferConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = dirs_fan().join("config.toml");
        Self::load_from(&path)
    }

    pub fn load_from(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        if path.exists() {
            let s = std::fs::read_to_string(path)?;
            let mut cfg: Config = toml::from_str(&s)?;
            // Migrate old scan_root → scan_roots
            let raw: toml::Value = toml::from_str(&s)?;
            if let Some(servers_table) = raw.get("servers") {
                for (name, server_val) in servers_table.as_table().unwrap_or(&Default::default()) {
                    if let Some(scan_root) = server_val.get("scan_root").and_then(|v| v.as_str()) {
                        if let Some(srv) = cfg.servers.servers.get_mut(name) {
                            if srv.scan_roots.is_empty() {
                                srv.scan_roots = vec![scan_root.to_string()];
                            }
                        }
                    }
                }
            }
            Ok(cfg)
        } else {
            let cfg = Config::default();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, toml::to_string_pretty(&cfg)?)?;
            Ok(cfg)
        }
    }

    /// Return the list of (server_name, ServerConfig) for enabled servers.
    /// If `servers` map is empty but `scan.include` is populated (old config),
    /// implicitly treat that as a single "local" server.
    pub fn enabled_servers(&self) -> Vec<(String, ServerConfig)> {
        if self.servers.servers.is_empty() && !self.scan.include.is_empty() {
            vec![(
                "local".to_string(),
                ServerConfig {
                    host: String::new(),
                    scan_roots: self.scan.include.clone(),
                    label: Some("本地 (自动迁移)".to_string()),
                    enabled: true,
                },
            )]
        } else {
            let mut v: Vec<_> = self
                .servers
                .servers
                .iter()
                .filter(|(_, cfg)| cfg.enabled)
                .map(|(name, cfg)| (name.clone(), cfg.clone()))
                .collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v
        }
    }
}

/// Which data layer an index belongs to.
#[derive(Debug, Clone, PartialEq)]
pub enum DataLayer {
    /// User private index (~/.fan-files/data/)
    User,
    /// Global public index (/var/lib/fan-files/data/)
    Global,
}

/// Global (admin-managed) data directory: /var/lib/fan-files
pub fn dirs_fan_global() -> PathBuf {
    PathBuf::from("/var/lib/fan-files")
}

/// Global config path: /etc/fan-files/config.toml
pub fn config_path_global() -> PathBuf {
    PathBuf::from("/etc/fan-files/config.toml")
}

pub fn dirs_fan() -> PathBuf {
    dirs_home().join(".fan-files")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// 国内常见 LLM 厂商预设
pub const LLM_PROVIDERS: &[LlmProvider] = &[
    LlmProvider {
        name: "DeepSeek",
        endpoint: "https://api.deepseek.com/v1/chat/completions",
        default_model: "deepseek-chat",
        description: "国内推荐，性价比最高",
    },
    LlmProvider {
        name: "通义千问 (Qwen)",
        endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
        default_model: "qwen-plus",
        description: "阿里云，模型矩阵丰富",
    },
    LlmProvider {
        name: "智谱 GLM",
        endpoint: "https://open.bigmodel.cn/api/paas/v4/chat/completions",
        default_model: "glm-4-flash",
        description: "国产均衡，教育优惠",
    },
    LlmProvider {
        name: "百度文心 (ERNIE)",
        endpoint: "https://aip.baidubce.com/rpc/2.0/ai_custom/v1/wenxinworkshop/chat/completions",
        default_model: "ernie-4.0-turbo-8k",
        description: "稳定性强，企业级",
    },
    LlmProvider {
        name: "OpenAI / 自定义",
        endpoint: "",
        default_model: "gpt-4o-mini",
        description: "自行填写 endpoint 和 key",
    },
];

pub struct LlmProvider {
    pub name: &'static str,
    pub endpoint: &'static str,
    pub default_model: &'static str,
    pub description: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [transfer] 段默认值：4MB / 4 并发 / None 接收目录 / UDP 启用
    #[test]
    fn transfer_config_defaults() {
        let t = TransferConfig::default();
        assert_eq!(t.chunk_size_mb, 4);
        assert_eq!(t.concurrency, 4);
        assert!(t.receive_dir.is_none());
        assert!(t.udp_enabled);
    }

    /// Config::default() 也应带上 [transfer] 默认段
    #[test]
    fn config_default_includes_transfer() {
        let cfg = Config::default();
        assert_eq!(cfg.transfer.chunk_size_mb, 4);
        assert_eq!(cfg.transfer.concurrency, 4);
    }

    /// config.toml [transfer] 读写 roundtrip（含部分覆盖默认值）
    #[test]
    fn transfer_config_toml_roundtrip() {
        let cfg = Config {
            transfer: TransferConfig {
                chunk_size_mb: 16,
                concurrency: 8,
                receive_dir: Some("/data/inbox".into()),
                udp_enabled: false,
            },
            ..Config::default()
        };
        let s = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.transfer.chunk_size_mb, 16);
        assert_eq!(back.transfer.concurrency, 8);
        assert_eq!(back.transfer.receive_dir.as_deref(), Some("/data/inbox"));
        assert!(!back.transfer.udp_enabled);
    }

    /// 旧 config.toml 无 [transfer] 段 → 加载不崩，用默认值
    #[test]
    fn legacy_config_without_transfer_loads() {
        let s = "[daemon]\nsocket = \"/tmp/test.sock\"\n[scan]\ninclude = [\"/data\"]\n";
        let cfg: Config = toml::from_str(s).unwrap();
        assert_eq!(cfg.transfer.chunk_size_mb, 4);
        assert_eq!(cfg.transfer.concurrency, 4);
        assert!(cfg.transfer.udp_enabled);
        assert!(cfg.transfer.receive_dir.is_none());
        // 旧字段不受影响
        assert_eq!(cfg.scan.include, vec!["/data".to_string()]);
    }

    /// [transfer] 部分字段缺省 → 其余用默认值
    #[test]
    fn transfer_partial_table_uses_defaults() {
        let s = "[transfer]\nconcurrency = 8\n";
        let cfg: Config = toml::from_str(s).unwrap();
        assert_eq!(cfg.transfer.chunk_size_mb, 4, "未写的 chunk_size_mb 用默认 4");
        assert_eq!(cfg.transfer.concurrency, 8);
        assert!(cfg.transfer.udp_enabled);
    }

    /// 旧 config.toml 无 llm.api_type → 默认 openai，不破坏加载
    #[test]
    fn legacy_llm_config_defaults_api_type_openai() {
        let s = "[llm]\nendpoint = \"https://api.deepseek.com/v1/chat/completions\"\napi_key = \"sk-x\"\nmodel = \"deepseek-chat\"\n";
        let cfg: Config = toml::from_str(s).unwrap();
        assert_eq!(cfg.llm.api_type, "openai");
        assert_eq!(cfg.llm.endpoint, "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(cfg.llm.model, "deepseek-chat");
    }

    /// [llm] 段整体缺失 → LlmConfig::default() 同样默认 openai
    #[test]
    fn llm_section_missing_defaults_openai() {
        let cfg = Config::default();
        assert_eq!(cfg.llm.api_type, "openai");
    }

    /// LlmConfig 读写 roundtrip：api_type 往返保留
    #[test]
    fn llm_config_roundtrip_keeps_api_type() {
        let mut cfg = LlmConfig::default();
        cfg.api_type = "anthropic".into();
        let s = toml::to_string(&cfg).unwrap();
        assert!(s.contains("api_type = \"anthropic\""), "s: {}", s);
        let back: LlmConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.api_type, "anthropic");
    }
}
