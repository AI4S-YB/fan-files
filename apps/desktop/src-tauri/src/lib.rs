//! fan-files desktop shell（Tauri 2）。
//!
//! 结构：config.rs（FanConfig 与 config.toml 读写）、engine.rs（sidecar 生命周期管理）、
//! commands.rs（前端 invoke 命令层）、本文件只做模块声明与 Tauri 应用组装
//! （含托盘常驻与 share 启动，见 run() 的 setup）。

mod commands;
mod config;
mod engine;

use std::sync::Mutex;

use tauri::Manager;

pub use config::FanConfig;

/// 引擎错误信息（None = 健康）。setup 异步启动 share 的结果与前端 retry 都写入这里，
/// 前端通过 engine_error 命令读取。
pub struct EngineStatus(pub Mutex<Option<String>>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::read_config,
            commands::write_config,
            commands::fan_home,
            commands::pick_directory,
            commands::test_connection,
            commands::open_path,
            commands::check_update,
            commands::get_share_port,
            commands::retry_engine,
            commands::engine_error
        ])
        .setup(|app| {
            use tauri::{
                menu::{Menu, MenuItem},
                tray::TrayIconBuilder,
                Manager,
            };

            let open = MenuItem::with_id(app, "open", "打开窗口", true, None::<&str>)?;
            let scan = MenuItem::with_id(app, "scan", "立即扫描", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &scan, &quit])?;

            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    // Task 17: 触发扫描（sidecar 生命周期管理器就绪后接入）
                    "scan" => {}
                    "quit" => app.exit(0),
                    _ => {}
                });
            // 托盘图标用默认应用图标（tauri.conf.json bundle.icon）
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;

            // sidecar 生命周期：异步拉起 share（不阻塞窗口），结果写入 EngineStatus
            app.manage(engine::Engine::new());
            app.manage(EngineStatus(Mutex::new(None)));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let engine = handle.state::<engine::Engine>();
                match engine::start_share(&engine) {
                    Ok(port) => {
                        if !engine::wait_healthy(port).await {
                            engine::kill_share(&engine);
                            *handle.state::<EngineStatus>().0.lock().unwrap() =
                                Some("引擎未运行".into());
                        }
                    }
                    Err(e) => *handle.state::<EngineStatus>().0.lock().unwrap() = Some(e),
                }
            });
            Ok(())
        })
        // Windows 关闭按钮收托盘；macOS/Linux 保持默认关闭行为
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                #[cfg(target_os = "windows")]
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
                #[cfg(not(target_os = "windows"))]
                let _ = (window, api);
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    // 退出时回收 share 子进程，避免孤儿进程残留占住端口
    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(engine) = app_handle.try_state::<engine::Engine>() {
                engine::kill_share(&engine);
            }
        }
    });
}
