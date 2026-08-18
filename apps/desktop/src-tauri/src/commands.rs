//! Tauri 前端 invoke 的命令层（薄包装，业务逻辑在 config.rs / 插件调用）。
//!
//! 可见性约定（T5/T12 E0255 教训的精确化）：#[tauri::command] 会为命令生成同名隐藏宏，
//! generate_handler! 在 lib.rs（父模块）按 `commands::foo` 路径展开，宏必须对父模块
//! 可见，故命令用 `pub(crate) fn`（此时宏带 #[macro_export]，导出到 crate root）。
//! 但不能在 crate root 用 `pub fn`：宏在根模块定义一次 + #[macro_export] 在根再导出
//! 一次 → 宏命名空间 E0255 冲突（T5/T12 血泪史）。前端 invoke 按字符串名调用，
//! 与 Rust 可见性无关。

use std::path::Path;
use std::sync::atomic::Ordering;

use crate::config::{config_path, read_config_at, write_config_at, FanConfig};
use crate::engine::{kill_share, start_share, wait_healthy, Engine, SHARE_PORT};
use crate::EngineStatus;

#[tauri::command]
pub(crate) fn read_config() -> Result<FanConfig, String> {
    read_config_at(&config_path())
}

#[tauri::command]
pub(crate) fn write_config(cfg: FanConfig) -> Result<(), String> {
    write_config_at(&config_path(), cfg)
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
#[tauri::command]
pub(crate) async fn retry_engine(
    engine: tauri::State<'_, Engine>,
    status: tauri::State<'_, EngineStatus>,
) -> Result<u16, String> {
    let port = start_share(&engine)?;
    if wait_healthy(port).await {
        *status.0.lock().unwrap() = None;
        Ok(port)
    } else {
        kill_share(&engine);
        Err("引擎健康检查失败".into())
    }
}

/// 引擎错误信息（None = 健康）。前端挂载时读取并每 5 秒轮询同步。
#[tauri::command]
pub(crate) fn engine_error(status: tauri::State<'_, EngineStatus>) -> Option<String> {
    status.0.lock().unwrap().clone()
}
