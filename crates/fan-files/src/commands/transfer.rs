//! `fan-files transfer` — P2P 数据传输（magic-wormhole）
//!
//! 数据方（内网）：`fan-files transfer send --dataset <name>` → 输出一次性配对码
//! 请求方（任意网络）：`fan-files transfer get <code> --output <path>` → 接收数据
//! 审计：`fan-files transfer log` → 数据方本地记录每次传输
//!
//! 机制：rendezvous 邮箱交换配对信息 → UDP 打洞直连 → 失败走 transit relay 兜底。
//! 配对码 + 公钥识别双方身份，分块 SHA-256 校验完整性。

use fan_core::config::{dirs_fan, DataLayer, Config};
use fan_core::index::sqlite::SqliteStore;
use magic_wormhole::transfer::{self, APP_CONFIG};
use magic_wormhole::transit;
use magic_wormhole::{Code, MailboxConnection, Wormhole};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 我们的 rendezvous 服务器（47.94.142.52:4000）
const RENDEZVOUS_URL: &str = "ws://47.94.142.52:4000/v1";
/// 我们的 transit relay（打洞失败时中继兜底）
const TRANSIT_RELAY: &str = "tcp://47.94.142.52:4001";

fn relay_hints() -> Vec<transit::RelayHint> {
    match url::Url::parse(TRANSIT_RELAY) {
        Ok(url) => transit::RelayHint::from_urls(Some("fan-relay".to_string()), [url])
            .map(|h| vec![h])
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// 审计日志表
const AUDIT_DDL: &str = "
CREATE TABLE IF NOT EXISTS transfer_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    direction TEXT NOT NULL,
    dataset TEXT NOT NULL,
    code TEXT NOT NULL,
    peer_key TEXT,
    bytes_sent INTEGER DEFAULT 0,
    bytes_received INTEGER DEFAULT 0,
    status TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER
);
";

pub fn run(config: &Config, layer: &DataLayer, action: TransferAction) {
    match action {
        TransferAction::Send { dataset, ttl_hours } => send(config, layer, &dataset, ttl_hours),
        TransferAction::Get { code, output } => get(config, layer, &code, output),
        TransferAction::Log => log(config, layer),
    }
}

fn open_store(config: &Config, layer: &DataLayer) -> SqliteStore {
    let data_dir = match layer {
        DataLayer::User => dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };
    match SqliteStore::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法打开索引数据库: {}", e);
            std::process::exit(1);
        }
    }
}

fn dataset_path(store: &SqliteStore, name: &str) -> Option<String> {
    // 直接路径优先（方便分享未索引的目录）
    let p = std::path::Path::new(name);
    if p.exists() {
        return Some(p.to_string_lossy().to_string());
    }
    // 否则在索引中按名称查找
    store.all_datasets().ok()?.into_iter()
        .find(|d| d.name == name || d.path.ends_with(&format!("/{}", name)))
        .map(|d| d.path)
}

fn audit(
    store: &SqliteStore,
    direction: &str,
    dataset: &str,
    code: &str,
    peer_key: Option<&str>,
    bytes_sent: u64,
    bytes_received: u64,
    status: &str,
    started_at: i64,
) {
    let conn = match store.conn.lock() {
        Ok(c) => c,
        Err(e) => { eprintln!("审计失败: 数据库锁 {}", e); return; }
    };
    let _ = conn.execute_batch(AUDIT_DDL);
    let _ = conn.execute(
        "INSERT INTO transfer_log
         (direction, dataset, code, peer_key, bytes_sent, bytes_received, status, started_at, finished_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        rusqlite::params![direction, dataset, code, peer_key, bytes_sent as i64, bytes_received as i64, status, started_at, now_secs()],
    );
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn app_config() -> magic_wormhole::AppConfig<transfer::AppVersion> {
    let mut cfg = APP_CONFIG.clone();
    cfg.rendezvous_url = std::borrow::Cow::Borrowed(RENDEZVOUS_URL);
    cfg
}

/// 发送端：`transfer send <dataset|path>`
fn send(config: &Config, layer: &DataLayer, dataset: &str, ttl_hours: u64) {
    let store = open_store(config, layer);
    let path = match dataset_path(&store, dataset) {
        Some(p) => p,
        None => {
            eprintln!("未找到 dataset '{}'。可先用 'fan-files datasets' 查看已索引的数据集。", dataset);
            std::process::exit(1);
        }
    };
    let is_dir = PathBuf::from(&path).is_dir();

    let started_at = now_secs();
    println!("fan-files transfer send");
    println!("  数据集: {}", dataset);
    println!("  路径:   {}", path);
    println!("  配对码有效期: {} 小时", ttl_hours);

    // 目录 → 临时 tar 打包（v1 协议仅支持单文件）
    let temp_tar = if is_dir {
        let tmp = std::env::temp_dir().join(format!("fan-transfer-{}.tar", now_secs()));
        match tar_directory(&path, &tmp) {
            Ok(()) => Some(tmp),
            Err(e) => { eprintln!("打包目录失败: {}", e); std::process::exit(1); }
        }
    } else { None };

    let send_target = temp_tar.as_ref().map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let display_name = if is_dir {
        format!("{}.tar", PathBuf::from(&path).file_name().unwrap_or_default().to_string_lossy())
    } else {
        PathBuf::from(&path).file_name().unwrap_or_default().to_string_lossy().to_string()
    };

    let result = async_io::block_on(async {
        let mailbox = MailboxConnection::create(app_config(), 3)
            .await
            .map_err(|e| format!("无法连接 rendezvous: {}", e))?;
        let code = mailbox.code().to_string();

        // 配对码在 create 后即生成，先打印让接收方开始连接
        println!("\n  传输码: {}", code);
        println!("  ⏳ 等待对方输入此码开始传输…（{ttl_hours}h 内有效）\n");

        let mut wormhole = Wormhole::connect(mailbox)
            .await
            .map_err(|e| format!("Wormhole 连接失败: {}", e))?;
        let peer_key = format!("{:x}", wormhole.verifier());

        let relay_hints = relay_hints();
        let res = transfer::send_file_or_folder(
            wormhole,
            relay_hints,
            send_target.as_str(),
            display_name.clone(),
            transit::Abilities::ALL,
            |info| eprintln!("  连接: {}", fmt_conn(&info.conn_type)),
            |done, total| {
                if total > 0 {
                    eprintln!("\r  进度: {}/{} ({:.0}%)", done, total, done as f64 / total as f64 * 100.0);
                }
            },
            std::future::pending::<()>(),
        ).await;

        match res {
            Ok(()) => { println!("\n  ✅ 传输完成，校验通过"); Ok((code, peer_key)) }
            Err(e) => Err(format!("传输失败: {}", e)),
        }
    });

    // 清理临时 tar
    if let Some(tmp) = &temp_tar {
        let _ = std::fs::remove_file(tmp);
    }

    match result {
        Ok((code, peer_key)) => {
            audit(&store, "send", dataset, &code, Some(&peer_key), 0, 0, "ok", started_at);
        }
        Err(e) => {
            audit(&store, "send", dataset, "unknown", None, 0, 0, "failed", started_at);
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

/// 用系统 tar 打包目录到临时文件
fn tar_directory(src: &str, dst: &std::path::Path) -> Result<(), String> {
    let src_path = PathBuf::from(src);
    let parent = src_path.parent().unwrap_or(std::path::Path::new("."));
    let name = src_path.file_name().unwrap_or_default();
    let out = std::process::Command::new("tar")
        .arg("-cf")
        .arg(dst)
        .arg("-C")
        .arg(parent)
        .arg(name)
        .status()
        .map_err(|e| format!("tar 执行失败: {}", e))?;
    if out.success() { Ok(()) } else { Err(format!("tar 退出码 {:?}", out.code())) }
}

/// 解包 tar 到目标目录
fn extract_tar(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    let out = std::process::Command::new("tar")
        .arg("-xf")
        .arg(src)
        .arg("-C")
        .arg(dst)
        .status()
        .map_err(|e| format!("tar 解包执行失败: {}", e))?;
    if out.success() { Ok(()) } else { Err(format!("tar 解包退出码 {:?}", out.code())) }
}

/// 接收端：`transfer get <code> [--output <path>]`
fn get(config: &Config, layer: &DataLayer, code_str: &str, output: Option<String>) {
    let store = open_store(config, layer);
    let started_at = now_secs();

    let code: Code = match code_str.parse() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无效的配对码 '{}': {}", code_str, e);
            std::process::exit(1);
        }
    };

    println!("fan-files transfer get");
    println!("  配对码: {}", code_str);
    println!("  ⏳ 正在连接…");

    let result = async_io::block_on(async {
        let mailbox = MailboxConnection::connect(app_config(), code, false)
            .await
            .map_err(|e| format!("无法连接 rendezvous（请确认配对码有效）: {}", e))?;
        let mut wormhole = Wormhole::connect(mailbox)
            .await
            .map_err(|e| format!("Wormhole 连接失败: {}", e))?;
        let peer_key = format!("{:x}", wormhole.verifier());

        let relay_hints = relay_hints();
        let req = match transfer::request_file(wormhole, relay_hints, transit::Abilities::ALL, std::future::pending::<()>()).await {
            Ok(Some(r)) => r,
            Ok(None) => return Err("对方取消了传输".to_string()),
            Err(e) => return Err(format!("请求失败: {}", e)),
        };

        let file_name = req.file_name();
        eprintln!("  收到: {}", file_name);

        // 目标路径：--output 指定（文件或目录），否则当前目录 + 原文件名
        let target_path: PathBuf = match &output {
            Some(p) if PathBuf::from(p).is_dir() => PathBuf::from(p).join(&file_name),
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(&file_name),
        };
        if let Some(parent) = target_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // v1 accept 需要 writer（AsyncWrite）写出到文件
        let mut file = async_fs::File::create(&target_path).await
            .map_err(|e| format!("无法创建输出文件 {}: {}", target_path.display(), e))?;
        let res = req.accept(
            &|info: magic_wormhole::transit::TransitInfo| eprintln!("  连接: {}", fmt_conn(&info.conn_type)),
            |done, total| {
                if total > 0 {
                    eprintln!("\r  进度: {}/{} ({:.0}%)", done, total, done as f64 / total as f64 * 100.0);
                }
            },
            &mut file,
            std::future::pending::<()>(),
        ).await;

        match res {
            Ok(()) => {
                println!("\n  ✅ 接收完成，SHA-256 校验通过");
                println!("  已保存: {}", target_path.display());
                // 若是 tar 包则自动解包到输出目录（目录数据集场景）
                if target_path.extension().map(|e| e == "tar").unwrap_or(false) {
                    let out_dir = target_path.parent().unwrap_or(std::path::Path::new("."));
                    match extract_tar(&target_path, out_dir) {
                        Ok(()) => {
                            println!("  ✅ 已解包到: {}", out_dir.display());
                            let _ = std::fs::remove_file(&target_path);
                        }
                        Err(e) => eprintln!("  ⚠ 解包失败: {}（tar 文件保留在 {}）", e, target_path.display()),
                    }
                }
                Ok(peer_key)
            }
            Err(e) => Err(format!("接收失败: {}", e)),
        }
    });

    match result {
        Ok(peer_key) => {
            audit(&store, "get", code_str, code_str, Some(&peer_key), 0, 0, "ok", started_at);
        }
        Err(e) => {
            audit(&store, "get", code_str, code_str, None, 0, 0, "failed", started_at);
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

fn fmt_conn(t: &transit::ConnectionType) -> &'static str {
    match t {
        transit::ConnectionType::Direct => "P2P 直连",
        transit::ConnectionType::Relay { .. } => "relay 中继",
        _ => "unknown",
    }
}

/// 审计：`transfer log`
fn log(config: &Config, layer: &DataLayer) {
    let store = open_store(config, layer);
    let conn = match store.conn.lock() {
        Ok(c) => c,
        Err(e) => { eprintln!("无法读审计日志: {}", e); return; }
    };
    let _ = conn.execute_batch(AUDIT_DDL);
    println!("时间                方向  数据集/码                  状态     字节");
    let mut stmt = match conn.prepare(
        "SELECT datetime(started_at,'unixepoch','localtime'), direction, dataset, status, bytes_sent+bytes_received
         FROM transfer_log ORDER BY id DESC LIMIT 50"
    ) {
        Ok(s) => s,
        Err(e) => { eprintln!("查询失败: {}", e); return; }
    };
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
            r.get::<_, String>(3)?, r.get::<_, i64>(4)?))
    });
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            println!("{}  {:5}  {:<24}  {:7}  {}", row.0, row.1, row.2, row.3, row.4);
        }
    }
}

/// CLI 子命令（由 main.rs 解析）
#[derive(Debug, Clone)]
pub enum TransferAction {
    Send { dataset: String, ttl_hours: u64 },
    Get { code: String, output: Option<String> },
    Log,
}
