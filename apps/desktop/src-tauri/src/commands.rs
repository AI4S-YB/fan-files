//! Tauri 前端 invoke 的命令层（薄包装，业务逻辑在 config.rs / 插件调用）。
//!
//! 可见性约定（T5/T12 E0255 教训的精确化）：#[tauri::command] 会为命令生成同名隐藏宏，
//! generate_handler! 在 lib.rs（父模块）按 `commands::foo` 路径展开，宏必须对父模块
//! 可见，故命令用 `pub(crate) fn`（此时宏带 #[macro_export]，导出到 crate root）。
//! 但不能在 crate root 用 `pub fn`：宏在根模块定义一次 + #[macro_export] 在根再导出
//! 一次 → 宏命名空间 E0255 冲突（T5/T12 血泪史）。前端 invoke 按字符串名调用，
//! 与 Rust 可见性无关。

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::Emitter;

use crate::config::{
    config_path, configured_receive_dir, default_receive_dir, read_config_at,
    read_transfer_config_at, transfer_cli_params, write_config_at, write_transfer_config_at,
    FanConfig,
};
use crate::engine::{kill_share, start_share, wait_healthy, Engine, SHARE_PORT};
use crate::EngineStatus;

/// 扫描互斥标志（true = discover 子进程正在运行）。
/// scan_now（前端命令 + 托盘菜单）与 lib.rs 的定时循环共用。
pub(crate) static SCANNING: AtomicBool = AtomicBool::new(false);

/// 当前传输子进程（取消用）。
/// event_prefix = "share" / "receive"，cancel_transfer 按它决定发哪个方向的 done 事件。
/// 只保存"最新"的一个传输：新传输启动会 replace 掉旧的（并回收其子进程），
/// 各传输任务结束时凭 child.id()（PID）确认句柄仍是自己的才取回 wait。
struct ActiveTransfer {
    child: tokio::process::Child,
    event_prefix: &'static str,
}

static CURRENT_TRANSFER: Mutex<Option<ActiveTransfer>> = Mutex::new(None);

#[tauri::command]
pub(crate) fn read_config() -> Result<FanConfig, String> {
    read_config_at(&config_path())
}

#[tauri::command]
pub(crate) fn write_config(cfg: FanConfig) -> Result<(), String> {
    write_config_at(&config_path(), cfg)
}

/// 读取 config.toml 的 [transfer] 段（缺失字段填默认：chunk_size_mb=4 /
/// concurrency=4 / receive_dir=null / udp_enabled=true）。
#[tauri::command]
pub(crate) fn read_transfer_config() -> Result<serde_json::Value, String> {
    read_transfer_config_at(&config_path())
}

/// 写 config.toml 的 [transfer] 段（read-modify-write，保留其他节；
/// 只合并提供的键，null 值删除对应键）。
#[tauri::command]
pub(crate) fn write_transfer_config(cfg: serde_json::Value) -> Result<(), String> {
    write_transfer_config_at(&config_path(), &cfg)
}

#[tauri::command]
pub(crate) fn fan_home() -> Result<String, String> {
    Ok(config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_string_lossy()
        .to_string())
}

/// 原生目录选择器。用户取消返回 Ok(None)，其余为 Err。
///
/// 必须是 async 命令：blocking_pick_folder 内部是 sync_channel(0) + rx.recv()，
/// 若在同步命令（主线程）里调用，recv 阻塞主线程 run loop，macOS 模态 sheet
/// 的回调（同样在主线程）永远无法触发 → 死锁。异步命令跑在 async runtime 线程
/// 上，这里用非阻塞 pick_folder 回调 + oneshot 桥接为 await
/// （tauri-plugin-dialog 2.7.2 没有 async pick_folder API）。
#[tauri::command]
pub(crate) async fn pick_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p.map(|f| f.to_string()));
    });
    rx.await
        .map_err(|_| "目录选择对话框未返回结果".to_string())
}

/// 用当前 GUI 表单里的 LLM 配置发一个最小请求，验证连通性。
/// 只按 HTTP 状态码判断成功（2xx），非 2xx 返回 Ok(false) 而非 Err，
/// 让前端能区分"连通但认证失败"与"请求本身出错"。
#[tauri::command]
pub(crate) async fn test_connection(cfg: FanConfig) -> Result<bool, String> {
    let body = serde_json::json!({
        "model": cfg.model,
        "messages": [{"role": "user", "content": "reply OK"}],
        "max_tokens": 10
    });
    let resp = reqwest::Client::new()
        .post(&cfg.endpoint)
        .header("Authorization", format!("Bearer {}", cfg.api_key))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(30))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp.status().is_success())
}

/// 用系统文件管理器打开目录（macOS Finder / Windows Explorer / Linux xdg-open）。
#[tauri::command]
pub(crate) async fn open_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

/// 运行 `fan-files update` 并返回其 stdout。
///
/// 二进制定位走 engine::sidecar_bin：dev 用 workspace target/release，
/// 打包后与主程序同目录（避免 macOS GUI 应用 PATH 不含 Homebrew 目录的问题）。
#[tauri::command]
pub(crate) async fn check_update() -> Result<String, String> {
    let bin = crate::engine::sidecar_bin("fan-files");
    let out = tokio::process::Command::new(&bin)
        .arg("update")
        .output()
        .await
        .map_err(|_| format!("无法找到 {}，或执行失败", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "fan-files update 失败（退出码 {}）：{}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// share 实际监听端口（可能因冲突回退，前端用它动态设置 API base）。
#[tauri::command]
pub(crate) fn get_share_port() -> u16 {
    SHARE_PORT.load(Ordering::SeqCst)
}

/// banner 重试：重新拉起 share 并健康检查。成功清空错误状态并返回端口。
/// 失败时把错误持久化到 EngineStatus——否则前端 5 秒轮询 engine_error
/// 会读到旧值（None），把 banner 上的重试错误清掉。
#[tauri::command]
pub(crate) async fn retry_engine(
    engine: tauri::State<'_, Engine>,
    status: tauri::State<'_, EngineStatus>,
) -> Result<u16, String> {
    let port = match start_share(&engine) {
        Ok(port) => port,
        Err(e) => {
            *status.0.lock().unwrap() = Some(e.clone());
            return Err(e);
        }
    };
    if wait_healthy(port).await {
        *status.0.lock().unwrap() = None;
        Ok(port)
    } else {
        kill_share(&engine);
        *status.0.lock().unwrap() = Some("引擎健康检查失败".to_string());
        Err("引擎健康检查失败".into())
    }
}

/// 引擎错误信息（None = 健康）。前端挂载时读取并每 5 秒轮询同步。
#[tauri::command]
pub(crate) fn engine_error(status: tauri::State<'_, EngineStatus>) -> Option<String> {
    status.0.lock().unwrap().clone()
}

/// 触发一次后台扫描（`fan-files discover`）。
///
/// 编排契约（T17）：
/// - SCANNING 做互斥，重复触发立即 Err("already scanning")；
/// - discover 的进度走 stderr（CLI 用 eprintln!），逐行转为 `scan://progress` 事件；
/// - 结束时发 `scan://done`（载荷为退出码，0 = 成功），前端只在 code==0 时刷新统计卡；
/// - spawn 失败发 `scan://error` 并复位互斥。
/// 命令本身立即返回，扫描在 async runtime 上跑，前端靠事件流更新 UI。
#[tauri::command]
pub(crate) async fn scan_now(app: tauri::AppHandle) -> Result<(), String> {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return Err("already scanning".into());
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut child = match tokio::process::Command::new(crate::engine::sidecar_bin("fan-files"))
            .env("FAN_JSON_FORMAT", "1")
            // GUI 桌面壳只扫本机 [scan].include 目录；忽略远程 [servers.*]，
            // 否则会在本地尝试扫配置里的远程路径（见 discover.rs 的 run_local）。
            .arg("discover")
            .arg("--local-only")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = handle.emit("scan://error", e.to_string());
                SCANNING.store(false, Ordering::SeqCst);
                return;
            }
        };
        use tokio::io::AsyncBufReadExt;
        if let Some(stderr) = child.stderr.take() {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = handle.emit("scan://progress", &line);
            }
        }
        let status = child.wait().await;
        // 信号终止时无退出码：-1 而非 0——0 会让前端误判成功并刷新统计卡
        let _ = handle.emit(
            "scan://done",
            status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1),
        );
        SCANNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// 扫描互斥标志的读取端（前端挂载时同步一次，托盘发起的扫描也能反映到 UI）。
#[tauri::command]
pub(crate) fn scan_state() -> bool {
    SCANNING.load(Ordering::SeqCst)
}

/// 数据集共享（P2P）：spawn `fan-files transfer send <path>`，把配对码和进度
/// 以事件流推给前端。事件契约：
/// - `share://code`     载荷 = 配对码（如 8-purple-hammer）
/// - `share://progress` 载荷 = stdout JSONL 事件行（{"type":"conn"|"progress"|"resume"|"done"|"error",...}），
///   前端 JSON.parse 后按 type 字段分发
/// - `share://done`     载荷 = 退出码（0 成功，-1 = 信号终止/取消）
/// - `share://error`    载荷 = 错误信息（spawn 失败时）
/// 命令立即返回，传输在 async runtime 上跑。子进程句柄存入 CURRENT_TRANSFER，
/// 前端可随时 cancel_transfer。
#[tauri::command]
pub(crate) async fn share_dataset(app: tauri::AppHandle, path: String) -> Result<(), String> {
    // config [transfer] → --chunk-size(字节)/--concurrency（缺省 4MB/4）
    let (chunk_bytes, concurrency) = transfer_cli_params();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut child = match tokio::process::Command::new(crate::engine::sidecar_bin("fan-files"))
            // FAN_JSON_PROGRESS=1：stdout 纯 JSONL 事件（conn/progress/resume/done/error），
            // 人类可读输出（含配对码）走 stderr
            .env("FAN_JSON_PROGRESS", "1")
            .arg("transfer")
            .arg("send")
            .arg(&path)
            .arg("--chunk-size")
            .arg(chunk_bytes.to_string())
            .arg("--concurrency")
            .arg(concurrency.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = handle.emit("share://error", e.to_string());
                return;
            }
        };
        // 管道句柄留在本地读流；Child 本体存入全局供 cancel_transfer 取消
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let my_id = child.id();
        // 新传输取代旧传输（重复触发）时，先回收旧子进程，避免句柄被覆盖后成孤儿。
        // 先取回（guard 随即释放）再 await kill，避免 MutexGuard 跨 await。
        let old = CURRENT_TRANSFER
            .lock()
            .unwrap()
            .replace(ActiveTransfer { child, event_prefix: "share" });
        if let Some(mut old) = old {
            let _ = old.child.kill().await;
        }
        use tokio::io::AsyncBufReadExt;
        // stdout：JSONL 事件行逐行转发（保持原始行，前端 JSON.parse 后按 type 分发）
        let stdout_task = async {
            if let Some(out) = stdout {
                let mut lines = tokio::io::BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = handle.emit("share://progress", &line);
                }
            }
        };
        // stderr：人类可读输出（JSON 模式下配对码只在这里）。提取配对码发 code 事件，
        // 其余行转发原始行（前端 JSON.parse 失败 → 折叠日志，失败原因可见）
        let stderr_task = async {
            if let Some(err) = stderr {
                let mut lines = tokio::io::BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // 从人类输出中提取配对码（格式: 数字-单词-单词-单词）
                    if line.contains("传输码:") {
                        if let Some(code) = line.split("传输码:").nth(1) {
                            let _ = handle.emit("share://code", code.trim().to_string());
                        }
                    }
                    let _ = handle.emit("share://progress", &line);
                }
            }
        };
        tokio::join!(stdout_task, stderr_task);
        // 流读完（子进程已退出）后取回句柄等待退出码；
        // 句柄已被 cancel_transfer（或更新的传输）取走时不再发 done——由对方发
        let at = {
            let mut guard = CURRENT_TRANSFER.lock().unwrap();
            if guard
                .as_ref()
                .map(|a| a.child.id() == my_id)
                .unwrap_or(false)
            {
                guard.take()
            } else {
                None
            }
        };
        if let Some(mut at) = at {
            let status = at.child.wait().await;
            let _ = handle.emit(
                "share://done",
                status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1),
            );
        }
    });
    Ok(())
}

/// 数据集接收（P2P）：spawn `fan-files transfer get <code> --output <dir>`。
/// 事件契约（与共享对称）：
/// - `receive://progress` 载荷 = stdout JSONL 事件行（前端 JSON.parse 分发）
/// - `receive://done`     载荷 = 退出码（0 成功，-1 = 信号终止/取消）
/// - `receive://error`    载荷 = 错误信息（spawn 失败时）
/// 输出目录解析：显式 output > config [transfer].receive_dir > ~/Downloads/fan-received。
/// 命令立即返回，接收在 async runtime 上跑。子进程句柄存入 CURRENT_TRANSFER。
#[tauri::command]
pub(crate) async fn receive_dataset(
    app: tauri::AppHandle,
    code: String,
    output: Option<String>,
) -> Result<(), String> {
    let out_dir = match output.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(o) => o.to_string(),
        None => match configured_receive_dir() {
            Some(d) => d,
            None => default_receive_dir(),
        },
    };
    // config [transfer] → --chunk-size(字节)/--concurrency（缺省 4MB/4；
    // 接收方 chunk_size 仅记录，实际块大小由发送方 FileMeta 决定）
    let (chunk_bytes, concurrency) = transfer_cli_params();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut child = match tokio::process::Command::new(crate::engine::sidecar_bin("fan-files"))
            .env("FAN_JSON_PROGRESS", "1")
            .arg("transfer")
            .arg("get")
            .arg(&code)
            .arg("--output")
            .arg(&out_dir)
            .arg("--chunk-size")
            .arg(chunk_bytes.to_string())
            .arg("--concurrency")
            .arg(concurrency.to_string())
            .stdout(Stdio::piped())
            // stderr：人类可读输出/失败原因 → 逐行转发原始行（前端 JSON.parse
            // 失败 → 折叠日志）。此前 Stdio::null() 丢弃，接收失败时看不到原因。
            // 双管道并发读（tokio::join!）避免管道满阻塞子进程。
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = handle.emit("receive://error", e.to_string());
                return;
            }
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let my_id = child.id();
        let old = CURRENT_TRANSFER
            .lock()
            .unwrap()
            .replace(ActiveTransfer { child, event_prefix: "receive" });
        if let Some(mut old) = old {
            let _ = old.child.kill().await;
        }
        use tokio::io::AsyncBufReadExt;
        // stdout：JSONL 事件行逐行转发（与共享侧对称）
        let stdout_task = async {
            if let Some(out) = stdout {
                let mut lines = tokio::io::BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = handle.emit("receive://progress", &line);
                }
            }
        };
        // stderr：人类可读输出（失败原因等）逐行转发原始行
        let stderr_task = async {
            if let Some(err) = stderr {
                let mut lines = tokio::io::BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = handle.emit("receive://progress", &line);
                }
            }
        };
        tokio::join!(stdout_task, stderr_task);
        let at = {
            let mut guard = CURRENT_TRANSFER.lock().unwrap();
            if guard
                .as_ref()
                .map(|a| a.child.id() == my_id)
                .unwrap_or(false)
            {
                guard.take()
            } else {
                None
            }
        };
        if let Some(mut at) = at {
            let status = at.child.wait().await;
            let _ = handle.emit(
                "receive://done",
                status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1),
            );
        }
    });
    Ok(())
}

/// 取消当前传输（share_dataset / receive_dataset 的子进程）。
/// 杀掉子进程并回收句柄（kill + wait 避免僵尸进程），
/// 向对应事件流发 done（载荷 -1 = 信号终止，前端视为取消/失败）。
#[tauri::command]
pub(crate) async fn cancel_transfer(app: tauri::AppHandle) -> Result<(), String> {
    let at = CURRENT_TRANSFER.lock().unwrap().take();
    if let Some(mut at) = at {
        let _ = at.child.kill().await;
        let _ = app.emit(&format!("{}://done", at.event_prefix), -1);
    }
    Ok(())
}

/// 读取 P2P 传输历史（审计表），返回最近记录。
/// 复用 CLI 的 `transfer log --json`（避免桌面端引入 rusqlite 依赖）。
#[tauri::command]
pub(crate) fn transfer_history() -> Result<Vec<serde_json::Value>, String> {
    let out = std::process::Command::new(crate::engine::sidecar_bin("fan-files"))
        .args(["transfer", "log", "--json"])
        .output()
        .map_err(|e| format!("读取传输历史失败: {}", e))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout)
        .map_err(|e| format!("解析传输历史失败: {}", e))
}
