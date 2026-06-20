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
    pub servers: ServersConfig,
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
    /// 扫描根目录
    pub scan_root: String,
    /// 人类可读的描述（可选）
    #[serde(default)]
    pub label: Option<String>,
    /// 是否参与扫描
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
}

fn default_llm_model() -> String {
    "gpt-4o-mini".into()
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            api_key: String::new(),
            model: default_llm_model(),
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
                exclude: vec!["/tmp".into(), "*.tmp".into()],
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
            servers: ServersConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = dirs_fan().join("config.toml");
        if path.exists() {
            let s = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&s)?)
        } else {
            let cfg = Config::default();
            std::fs::create_dir_all(dirs_fan())?;
            std::fs::write(&path, toml::to_string_pretty(&cfg)?)?;
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
                    scan_root: self.scan.include.first().cloned().unwrap_or_default(),
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
