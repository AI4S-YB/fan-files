use clap::Parser;
use fan_core::config::LlmConfig;
use serde::Deserialize;
use std::{env, fs, net::SocketAddr, path::PathBuf};

#[derive(Debug, Parser)]
#[command(name = "fan-files-share", version)]
pub struct Args {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub database: Option<PathBuf>,
    #[arg(long)]
    pub bind: Option<SocketAddr>,
    /// 暴露数据集的绝对路径（默认关闭；桌面壳需要路径列与"打开目录"）
    #[arg(long)]
    pub expose_absolute_paths: bool,
    /// /stats 与 /facets 的缓存 TTL（秒）。缺省用配置文件或内置默认值 60
    #[arg(long)]
    pub stats_cache_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub bind: SocketAddr,
    pub database: PathBuf,
    pub pool_size: u32,
    pub busy_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub stats_cache_seconds: u64,
    pub max_page_size: u32,
    pub expose_absolute_paths: bool,
    pub supported_schema_versions: Vec<i64>,
    /// LLM 模型配置（config.toml [llm] 段；未配置时 chat-search 返回 503，
    /// 前端降级基础搜索）。旧配置文件无该段 → 默认空配置，不破坏加载
    pub llm: LlmConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8932".parse().unwrap(),
            database: PathBuf::from("index.db"),
            pool_size: 4,
            busy_timeout_ms: 5_000,
            request_timeout_ms: 5_000,
            stats_cache_seconds: 60,
            max_page_size: 200,
            expose_absolute_paths: false,
            supported_schema_versions: vec![4],
            llm: LlmConfig::default(),
        }
    }
}

impl Settings {
    pub fn load(args: Args) -> Result<Self, Box<dyn std::error::Error>> {
        let mut value = if let Some(path) = args.config {
            toml::from_str(&fs::read_to_string(path)?)?
        } else {
            Self::default()
        };
        if let Some(database) = args.database {
            value.database = database;
        }
        if let Some(bind) = args.bind {
            value.bind = bind;
        }
        // CLI flag 只负责打开；未传时保持配置文件值或内置默认 false
        // （share 作为网络服务默认不暴露绝对路径）。
        if args.expose_absolute_paths {
            value.expose_absolute_paths = true;
        }
        if let Some(ttl) = args.stats_cache_seconds {
            value.stats_cache_seconds = ttl;
        }
        // The sidecar may be started from an arbitrary working directory
        // (e.g. the desktop shell), so make the database path absolute;
        // the tantivy index dir is derived from its parent.
        if value.database.is_relative() {
            value.database = env::current_dir()?.join(&value.database);
        }
        if value.pool_size == 0 || value.max_page_size == 0 {
            return Err("pool_size and max_page_size must be positive".into());
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn args_parse_expose_absolute_paths_flag() {
        let args = Args::try_parse_from(["fan-files-share", "--expose-absolute-paths"]).unwrap();
        assert!(args.expose_absolute_paths);
        let args = Args::try_parse_from(["fan-files-share"]).unwrap();
        assert!(!args.expose_absolute_paths);
    }

    #[test]
    fn args_parse_stats_cache_seconds_flag() {
        let args =
            Args::try_parse_from(["fan-files-share", "--stats-cache-seconds", "5"]).unwrap();
        assert_eq!(args.stats_cache_seconds, Some(5));
        let args = Args::try_parse_from(["fan-files-share"]).unwrap();
        assert_eq!(args.stats_cache_seconds, None);
    }

    #[test]
    fn settings_load_applies_expose_flag_and_keeps_default_false() {
        let args = Args::try_parse_from(["fan-files-share", "--expose-absolute-paths"]).unwrap();
        let settings = Settings::load(args).unwrap();
        assert!(settings.expose_absolute_paths);
        // 未传 flag：默认 false 不变
        let args = Args::try_parse_from(["fan-files-share"]).unwrap();
        let settings = Settings::load(args).unwrap();
        assert!(!settings.expose_absolute_paths);
    }

    #[test]
    fn settings_load_applies_stats_cache_flag_and_keeps_default_60() {
        let args =
            Args::try_parse_from(["fan-files-share", "--stats-cache-seconds", "5"]).unwrap();
        let settings = Settings::load(args).unwrap();
        assert_eq!(settings.stats_cache_seconds, 5);
        // 未传 flag：内置默认 60 不变
        let args = Args::try_parse_from(["fan-files-share"]).unwrap();
        let settings = Settings::load(args).unwrap();
        assert_eq!(settings.stats_cache_seconds, 60);
    }

    /// [llm] 段解析进 Settings.llm（NR-T2：chat-search 的模型配置来源）；
    /// 缺 [llm] 段 → 默认空配置（未配置，chat-search 返回 503）
    #[test]
    fn settings_load_parses_llm_section_and_defaults_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[llm]\nendpoint = \"https://api.example.com/v1\"\napi_key = \"sk-x\"\nmodel = \"deepseek-chat\"\napi_type = \"anthropic\"\n",
        )
        .unwrap();
        let args = Args::try_parse_from(["fan-files-share", "--config", path.to_str().unwrap()])
            .unwrap();
        let settings = Settings::load(args).unwrap();
        assert_eq!(settings.llm.endpoint, "https://api.example.com/v1");
        assert_eq!(settings.llm.api_key, "sk-x");
        assert_eq!(settings.llm.model, "deepseek-chat");
        assert_eq!(settings.llm.api_type, "anthropic");
        // 无 [llm] 段 → 默认空配置（未配置，chat-search 返回 503）
        let plain = Args::try_parse_from(["fan-files-share"]).unwrap();
        let settings = Settings::load(plain).unwrap();
        assert!(settings.llm.endpoint.is_empty());
        assert!(settings.llm.api_key.is_empty());
    }
}
