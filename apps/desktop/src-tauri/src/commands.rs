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
    let (url, headers, body) = test_connection_request(&cfg);
    let mut req = reqwest::Client::new()
        .post(&url)
        .timeout(std::time::Duration::from_secs(30))
        .json(&body);
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    Ok(resp.status().is_success())
}

/// 按 api_type 构造 test_connection 的最小请求（url/headers/body）。
/// 语义与引擎 fan-core build_llm_request 对齐（NR-T4 P1 修复：接管 anthropic
/// profile 后测试连接必须走 Messages 协议，否则 base_url 无 /v1 时 404 误报失败）。
/// openai    → openai_chat_url(endpoint)、Authorization: Bearer {key}、
///             body {"model","messages","max_tokens":1}
/// anthropic → anthropic_messages_url(endpoint)、x-api-key: {key} +
///             anthropic-version: 2023-06-01、body 同构
fn test_connection_request(cfg: &FanConfig) -> (String, Vec<(String, String)>, serde_json::Value) {
    let body = serde_json::json!({
        "model": cfg.model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1,
    });
    if cfg.api_type == "anthropic" {
        let headers = vec![
            ("x-api-key".to_string(), cfg.api_key.clone()),
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        (anthropic_messages_url(&cfg.endpoint), headers, body)
    } else {
        // openai / 未知类型（兼容旧配置默认）
        let headers = vec![
            ("Authorization".to_string(), format!("Bearer {}", cfg.api_key)),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        (openai_chat_url(&cfg.endpoint), headers, body)
    }
}

/// openai Chat Completions 端点 URL 规范化（与 fan-core build_llm_request 语义一致）：
/// - 已含 /chat/completions → 原样
/// - 已含 /v1 → 拼 /chat/completions（CC Switch OPENAI_BASE_URL 形态，如 https://api.deepseek.com/v1）
/// - 否则 → 拼 /v1/chat/completions
fn openai_chat_url(endpoint: &str) -> String {
    let ep = endpoint.trim_end_matches('/');
    if ep.is_empty() {
        return ep.to_string();
    }
    if ep.ends_with("/chat/completions") {
        ep.to_string()
    } else if ep.ends_with("/v1") {
        format!("{}/chat/completions", ep)
    } else {
        format!("{}/v1/chat/completions", ep)
    }
}

/// anthropic Messages 端点 URL：endpoint 无 /v1 时拼 /v1/messages，已有不重复。
/// 尾斜杠先去掉，避免拼出 //v1/messages
fn anthropic_messages_url(endpoint: &str) -> String {
    let ep = endpoint.trim_end_matches('/');
    if ep.ends_with("/v1/messages") {
        ep.to_string()
    } else if ep.ends_with("/v1") {
        format!("{}/messages", ep)
    } else {
        format!("{}/v1/messages", ep)
    }
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

/// 读取 CC Switch 当前激活 profile 的 LLM 端点（spawn `fan-files config cc-switch`）。
/// 返回 {"api_type","base_url","api_key","model"}；未找到配置（引擎退出码 1）→
/// Err("未找到 CC Switch 配置")，前端显示在"从 CC Switch 接管"按钮旁。
/// 二进制定位走 engine::sidecar_bin（与 check_update 同模式）。
#[tauri::command]
pub(crate) fn read_cc_switch() -> Result<serde_json::Value, String> {
    let out = std::process::Command::new(crate::engine::sidecar_bin("fan-files"))
        .args(["config", "cc-switch"])
        .output()
        .map_err(|e| format!("读取 CC Switch 配置失败: {e}"))?;
    if !out.status.success() {
        return Err("未找到 CC Switch 配置".into());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("解析 CC Switch 配置失败: {e}"))
}

/// 内部：spawn 任意 fan-files 二进制的 `config cc-switch <args>` 子命令并解析 stdout JSON。
/// `exit_failure_is_missing=true`：子进程非 0 退出 → Err("未找到 CC Switch 配置")
/// （引擎 --profile 未找到时输出 {"error":"not-found"} 且退出码 1）；
/// false → 忽略退出码只解析 stdout（--list 恒 0 退出，含空数组 []）。
/// 二进制路径注入参数：测试用假脚本验证 argv/退出码/解析语义，不依赖真实二进制。
fn run_cc_switch_json(
    bin: &Path,
    args: &[&str],
    exit_failure_is_missing: bool,
) -> Result<serde_json::Value, String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("config").arg("cc-switch");
    for a in args {
        cmd.arg(a);
    }
    let out = cmd.output().map_err(|e| format!("读取 CC Switch 配置失败: {e}"))?;
    if exit_failure_is_missing && !out.status.success() {
        return Err("未找到 CC Switch 配置".into());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("解析 CC Switch 配置失败: {e}"))
}

/// 列出 CC Switch 全部 profile 摘要（spawn `fan-files config cc-switch --list`）。
/// 返回 [{"name","api_type","model"},...]（按目录名排序）；无配置 → 空数组，
/// 前端据此分流：0 个提示 / 1 个直接接管 / 多个弹窗选择。
#[tauri::command]
pub(crate) fn list_cc_switch_profiles() -> Result<Vec<serde_json::Value>, String> {
    let value = run_cc_switch_json(
        &crate::engine::sidecar_bin("fan-files"),
        &["--list"],
        false,
    )?;
    serde_json::from_value(value).map_err(|e| format!("解析 CC Switch 配置失败: {e}"))
}

/// 读取指定 CC Switch profile 的 LLM 端点（spawn `fan-files config cc-switch --profile <name>`）。
/// 返回 {"api_type","base_url","api_key","model"}；profile 不存在（引擎退出码 1）→
/// Err("未找到 CC Switch 配置")。供前端弹窗选中后填充表单。
#[tauri::command]
pub(crate) fn read_cc_switch_profile(name: String) -> Result<serde_json::Value, String> {
    run_cc_switch_json(
        &crate::engine::sidecar_bin("fan-files"),
        &["--profile", name.as_str()],
        true,
    )
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

/// [transfer].udp_enabled=false 时的传输子进程环境注入：返回 FAN_NO_UDP=1
/// （引擎 transfer.rs 的开关：跳过 UDP 打洞相位，强制 relay）；true → 空（默认）。
/// 返回键值对而非直接操作 Command，便于单元测试断言注入内容。
fn udp_envs(udp_enabled: bool) -> Vec<(&'static str, &'static str)> {
    if udp_enabled {
        Vec::new()
    } else {
        vec![("FAN_NO_UDP", "1")]
    }
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
pub(crate) async fn share_dataset(
    app: tauri::AppHandle,
    path: String,
    // NR-T5: 配对码有效期（小时），来自 GUI 共享弹层的有效期选择（24h/7天/自定义）。
    // None（旧前端）→ 不传 --ttl-hours，引擎用默认 168h（7 天）。
    ttl_hours: Option<u64>,
) -> Result<(), String> {
    // config [transfer] → --chunk-size(字节)/--concurrency（缺省 4MB/4）+ udp_enabled
    let (chunk_bytes, concurrency, udp_enabled) = transfer_cli_params();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut cmd = tokio::process::Command::new(crate::engine::sidecar_bin("fan-files"));
        // FAN_JSON_PROGRESS=1：stdout 纯 JSONL 事件（conn/progress/resume/done/error），
        // 人类可读输出（含配对码）走 stderr
        cmd.env("FAN_JSON_PROGRESS", "1");
        // GUI-T3 修复：设置页 UDP toggle 关闭 → FAN_NO_UDP=1，让配置真正生效
        for (k, v) in udp_envs(udp_enabled) {
            cmd.env(k, v);
        }
        cmd.arg("transfer")
            .arg("send")
            .arg(&path)
            .arg("--chunk-size")
            .arg(chunk_bytes.to_string())
            .arg("--concurrency")
            .arg(concurrency.to_string());
        // NR-T5: 共享弹层选择的有效期 → --ttl-hours（引擎 transfer send 的码有效期）
        if let Some(ttl) = ttl_hours {
            cmd.arg("--ttl-hours").arg(ttl.to_string());
        }
        let mut child = match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
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

/// 数据集接收（P2P）：spawn `fan-files transfer get <code>`。
/// 返回实际使用的接收目录（前端"打开接收目录"按钮用）。
/// 事件契约（与共享对称）：
/// - `receive://progress` 载荷 = stdout JSONL 事件行（前端 JSON.parse 分发）
/// - `receive://done`     载荷 = 退出码（0 成功，-1 = 信号终止/取消）
/// - `receive://error`    载荷 = 错误信息（spawn 失败时）
/// 输出目录解析：显式 output > config [transfer].receive_dir > ~/Downloads/fan-received
/// （GUI-T3 修复：前端不再传 output，统一走后两级，让设置页配置的接收目录真正生效）。
/// 命令立即返回（已解析目录随返回值给出），接收在 async runtime 上跑。
#[tauri::command]
pub(crate) async fn receive_dataset(
    app: tauri::AppHandle,
    code: String,
    output: Option<String>,
) -> Result<String, String> {
    let out_dir = match output.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(o) => o.to_string(),
        None => match configured_receive_dir() {
            Some(d) => d,
            None => default_receive_dir(),
        },
    };
    // config [transfer] → --chunk-size(字节)/--concurrency（缺省 4MB/4；
    // 接收方 chunk_size 仅记录，实际块大小由发送方 FileMeta 决定）+ udp_enabled
    let (chunk_bytes, concurrency, udp_enabled) = transfer_cli_params();
    let handle = app.clone();
    let out_arg = out_dir.clone();
    tauri::async_runtime::spawn(async move {
        let mut cmd = tokio::process::Command::new(crate::engine::sidecar_bin("fan-files"));
        cmd.env("FAN_JSON_PROGRESS", "1");
        // GUI-T3 修复：设置页 UDP toggle 关闭 → FAN_NO_UDP=1，让配置真正生效
        for (k, v) in udp_envs(udp_enabled) {
            cmd.env(k, v);
        }
        let mut child = match cmd
            .arg("transfer")
            .arg("get")
            .arg(&code)
            .arg("--output")
            .arg(out_arg)
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
    Ok(out_dir)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// GUI-T3 修复：UDP toggle 关闭 → 子进程环境注入 FAN_NO_UDP=1；
    /// 开启（默认）→ 不注入。
    #[test]
    fn udp_envs_inject_fan_no_udp_when_disabled() {
        assert_eq!(udp_envs(false), vec![("FAN_NO_UDP", "1")]);
        assert!(udp_envs(true).is_empty());
    }

    // ---------- NR-T4 P1 修复：test_connection 按 api_type 分支 ----------
    // 构造语义与引擎 fan-core build_llm_request 对齐（src-tauri 独立实现）。

    fn openai_cfg() -> FanConfig {
        FanConfig {
            threads: None,
            include: vec![],
            exclude: vec![],
            endpoint: "https://api.example.com/v1/chat/completions".into(),
            api_key: "sk-test".into(),
            model: "gpt-4o-mini".into(),
            api_type: "openai".into(),
        }
    }

    fn anthropic_cfg() -> FanConfig {
        FanConfig {
            threads: None,
            include: vec![],
            exclude: vec![],
            endpoint: "http://10.33.105.218:3200".into(),
            api_key: "sk-anth".into(),
            model: "claude-sonnet-4-8".into(),
            api_type: "anthropic".into(),
        }
    }

    /// api_type=openai：url 原样、Authorization Bearer、body model/messages/max_tokens
    #[test]
    fn test_connection_request_openai() {
        let (url, headers, body) = test_connection_request(&openai_cfg());
        assert_eq!(url, "https://api.example.com/v1/chat/completions");
        assert!(headers.contains(&("Authorization".to_string(), "Bearer sk-test".to_string())));
        assert!(headers.contains(&("Content-Type".to_string(), "application/json".to_string())));
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["messages"][0]["content"], "ping");
        assert_eq!(body["max_tokens"], 1);
    }

    /// api_type=anthropic：url 拼 /v1/messages、x-api-key + anthropic-version、
    /// body model/messages/max_tokens（无 temperature / Authorization）
    #[test]
    fn test_connection_request_anthropic() {
        let (url, headers, body) = test_connection_request(&anthropic_cfg());
        assert_eq!(url, "http://10.33.105.218:3200/v1/messages");
        assert!(headers.contains(&("x-api-key".to_string(), "sk-anth".to_string())));
        assert!(headers.contains(&("anthropic-version".to_string(), "2023-06-01".to_string())));
        assert!(!headers.iter().any(|(k, _)| k == "Authorization"));
        assert_eq!(body["model"], "claude-sonnet-4-8");
        assert_eq!(body["messages"][0]["content"], "ping");
        assert_eq!(body["max_tokens"], 1);
    }

    /// openai_chat_url 三种形态：已含 /chat/completions → 原样；含 /v1 → 拼；
    /// 否则 → 拼 /v1/chat/completions；尾斜杠不产生 //。
    #[test]
    fn openai_chat_url_three_forms() {
        // CC Switch OPENAI_BASE_URL 形态
        assert_eq!(
            openai_chat_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        // 已含完整路径 → 原样
        assert_eq!(
            openai_chat_url("https://api.example.com/v1/chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );
        // 裸主机 → 拼 /v1/chat/completions
        assert_eq!(
            openai_chat_url("https://api.example.com"),
            "https://api.example.com/v1/chat/completions"
        );
        // 尾斜杠
        assert_eq!(
            openai_chat_url("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    /// anthropic_messages_url：无 /v1 → 拼 /v1/messages；含 /v1 → 拼 /messages；
    /// 已含完整端点 → 原样；尾斜杠不拼出 //v1/messages
    #[test]
    fn anthropic_messages_url_three_forms() {
        // 真实接管形态：base_url 无 /v1
        assert_eq!(
            anthropic_messages_url("http://10.33.105.218:3200"),
            "http://10.33.105.218:3200/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("http://x:3200/v1/"),
            "http://x:3200/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("http://x:3200/"),
            "http://x:3200/v1/messages"
        );
    }

    // ---------- SF-T4: cc-switch profile 列表与按名读取（run_cc_switch_json 层）----------
    // 不 spawn 真实 fan-files 二进制：临时目录写一个模拟脚本，验证 argv 构造、
    // 退出码→Err 语义与 JSON 解析。脚本按 $3 分发：--list 恒 0 退出；--profile
    // 有值 0 退出 / None 时输出 not-found 并退出 1（与引擎 config.rs cc_switch 对齐）。

    #[cfg(unix)]
    fn write_fake_cc_switch_bin(
        dir: &std::path::Path,
        list_json: &str,
        profile_json: Option<&str>,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("fan-files-fake");
        let profile_branch = match profile_json {
            Some(j) => format!("printf '%s' '{}'; exit 0", j),
            None => "printf '%s' '{\"error\":\"not-found\"}'; exit 1".to_string(),
        };
        let script = format!(
            "#!/bin/sh\ncase \"$3\" in\n  --list) printf '%s' '{}'; exit 0 ;;\n  --profile) {} ;;\n  *) exit 1 ;;\nesac\n",
            list_json, profile_branch
        );
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    #[cfg(unix)]
    fn fake_cc_switch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fan-cc-fake-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// --list：2 个 profile → 解析出 name/api_type/model 数组
    #[cfg(unix)]
    #[test]
    fn list_cc_switch_profiles_parses_summary_array() {
        let dir = fake_cc_switch_dir("list");
        let bin = write_fake_cc_switch_bin(
            &dir,
            r#"[{"name":"haikou-flash","api_type":"anthropic","model":"claude-sonnet-4-8"},{"name":"official-pro","api_type":"openai","model":"deepseek-chat"}]"#,
            Some("{}"),
        );
        let value = run_cc_switch_json(&bin, &["--list"], false).unwrap();
        let list: Vec<serde_json::Value> = serde_json::from_value(value).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["name"].as_str(), Some("haikou-flash"));
        assert_eq!(list[0]["api_type"].as_str(), Some("anthropic"));
        assert_eq!(list[0]["model"].as_str(), Some("claude-sonnet-4-8"));
        assert_eq!(list[1]["name"].as_str(), Some("official-pro"));
        assert_eq!(list[1]["api_type"].as_str(), Some("openai"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// --list：0 个 profile → 空数组且 Ok（引擎 --list 恒 0 退出，不做 not-found 判定）
    #[cfg(unix)]
    #[test]
    fn list_cc_switch_profiles_empty_is_ok() {
        let dir = fake_cc_switch_dir("empty");
        let bin = write_fake_cc_switch_bin(&dir, "[]", Some("{}"));
        let value = run_cc_switch_json(&bin, &["--list"], false).unwrap();
        assert_eq!(value.as_array().unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// --profile <name>：解析出完整 LlmEndpoint（api_type/base_url/api_key/model）
    #[cfg(unix)]
    #[test]
    fn read_cc_switch_profile_parses_endpoint() {
        let dir = fake_cc_switch_dir("profile");
        let bin = write_fake_cc_switch_bin(
            &dir,
            "[]",
            Some(r#"{"api_type":"openai","base_url":"https://api.deepseek.com/v1","api_key":"sk-x","model":"deepseek-chat"}"#),
        );
        let value = run_cc_switch_json(&bin, &["--profile", "official-pro"], true).unwrap();
        assert_eq!(value["api_type"].as_str(), Some("openai"));
        assert_eq!(value["base_url"].as_str(), Some("https://api.deepseek.com/v1"));
        assert_eq!(value["api_key"].as_str(), Some("sk-x"));
        assert_eq!(value["model"].as_str(), Some("deepseek-chat"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// --profile <name> 不存在（退出码 1）→ Err("未找到 CC Switch 配置")
    #[cfg(unix)]
    #[test]
    fn read_cc_switch_profile_missing_is_err() {
        let dir = fake_cc_switch_dir("missing");
        let bin = write_fake_cc_switch_bin(&dir, "[]", None);
        let err = run_cc_switch_json(&bin, &["--profile", "nope"], true).unwrap_err();
        assert_eq!(err, "未找到 CC Switch 配置");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// --profile 0 退出但 stdout 非 JSON → 解析错误（不误报 not-found）
    #[cfg(unix)]
    #[test]
    fn read_cc_switch_profile_bad_json_is_parse_err() {
        let dir = fake_cc_switch_dir("badjson");
        let bin = write_fake_cc_switch_bin(&dir, "[]", Some("not json"));
        let err = run_cc_switch_json(&bin, &["--profile", "x"], true).unwrap_err();
        assert!(err.starts_with("解析 CC Switch 配置失败"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
