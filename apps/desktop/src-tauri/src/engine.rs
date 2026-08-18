//! sidecar 生命周期管理器：share 常驻子进程、端口回退、健康检查。
//!
//! 规格 §四 的子进程 sidecar 架构落地点：GUI 启动时拉起 fan-files-share，
//! 默认端口 17951，冲突时回退随机端口；前端通过 get_share_port 动态设置 API base。

use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// share 当前实际端口（前端 get_share_port 读取）。
pub static SHARE_PORT: AtomicU16 = AtomicU16::new(17951);

/// share 子进程句柄（kill 时使用；None = 未启动）。
pub struct Engine {
    pub share: Mutex<Option<Child>>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            share: Mutex::new(None),
        }
    }
}

/// sidecar 二进制定位：
/// 1. 打包运行 = 与主程序同目录的裸名（tauri externalBin 平铺到 Contents/MacOS）；
/// 2. dev 运行 = workspace target/release；
/// 3. 兜底 = PATH 上的裸名。
///
/// 打包运行必须优先用主程序同目录的 sidecar：打包后的应用是 LS 启动的 GUI 进程，
/// 其子进程读取 workspace 路径（~/Desktop 下）的裸二进制会被 macOS TCC 拦截
/// （dyld open 阻塞数秒后失败，实测见 T16 运行时验证），而同 bundle 内的
/// sidecar 天然随应用授权。
pub fn sidecar_bin(name: &str) -> PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Ok(cur) = std::env::current_exe() {
        if let Some(dir) = cur.parent() {
            let beside = dir.join(&exe);
            if beside.exists() {
                return beside;
            }
        }
    }
    let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/release")
        .join(&exe);
    if dev.exists() {
        dev
    } else {
        PathBuf::from(&exe)
    }
}

/// 端口回退：无冲突用 base，冲突换 20000-34999 随机端口（基于 PID，稳定可复现）。
pub fn next_port(base: u16, conflict: bool) -> u16 {
    if !conflict {
        base
    } else {
        20000 + (std::process::id() as u16 % 15000)
    }
}

fn is_port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn share_db_path() -> Result<PathBuf, String> {
    Ok(dirs::home_dir()
        .ok_or("无法获取用户主目录")?
        .join(".fan-files/data/index.db"))
}

/// 启动 share（默认 17951，冲突换随机），返回实际端口；失败返回 Err。
///
/// share 启动失败会立即退出（db 缺失/二进制缺失）——spawn 成功不代表进程存活，
/// 所以 spawn 前先做 exists 检查，分别给出友好错误；spawn 后是否存活由
/// wait_healthy 判断，失败方（setup/retry_engine）调用 kill_share 回收子进程。
pub fn start_share(engine: &Engine) -> Result<u16, String> {
    let bin = sidecar_bin("fan-files-share");
    if !bin.exists() {
        return Err("引擎二进制缺失，请重新安装".into());
    }
    let db = share_db_path()?;
    if !db.exists() {
        return Err("尚未扫描——数据索引不存在，请先在首页添加目录并扫描".into());
    }
    // 回收上一个（可能已死亡/僵死）的子进程，避免累积僵尸进程
    kill_share(engine);

    let base = SHARE_PORT.load(Ordering::SeqCst);
    let port = next_port(base, !is_port_free(base));
    SHARE_PORT.store(port, Ordering::SeqCst);
    let child = std::process::Command::new(&bin)
        .arg("--bind")
        .arg(format!("127.0.0.1:{port}"))
        .arg("--database")
        .arg(&db)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("share 启动失败: {e}"))?;
    *engine.share.lock().unwrap() = Some(child);
    Ok(port)
}

/// 杀掉 share 子进程（若在运行）并回收（wait 避免僵尸进程）。
pub fn kill_share(engine: &Engine) {
    if let Some(mut child) = engine.share.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// 轮询 /healthz 直到 2xx（20 次 x 250ms = 最多 5 秒）。
pub async fn wait_healthy(port: u16) -> bool {
    for _ in 0..20 {
        if let Ok(r) = reqwest::get(format!("http://127.0.0.1:{port}/healthz")).await {
            if r.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_port_returns_base_when_no_conflict() {
        assert_eq!(next_port(17951, false), 17951);
    }

    #[test]
    fn next_port_falls_back_on_conflict() {
        let port = next_port(17951, true);
        assert_ne!(port, 17951);
        assert!((20000..35000).contains(&port));
    }
}
