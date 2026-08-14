use clap::Parser;
use serde::Deserialize;
use std::{fs, net::SocketAddr, path::PathBuf};

#[derive(Debug, Parser)]
#[command(name = "fan-files-share", version)]
pub struct Args {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub database: Option<PathBuf>,
    #[arg(long)]
    pub bind: Option<SocketAddr>,
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
        if value.pool_size == 0 || value.max_page_size == 0 {
            return Err("pool_size and max_page_size must be positive".into());
        }
        Ok(value)
    }
}
