//! fan-files desktop shell（Tauri 2）。
//!
//! 结构：config.rs（FanConfig 与 config.toml 读写）、commands.rs（前端 invoke 命令层）、
//! 本文件只做模块声明与 Tauri 应用组装（含托盘常驻，见 run() 的 setup）。

mod commands;
mod config;

pub use config::FanConfig;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::read_config,
            commands::write_config,
            commands::fan_home,
            commands::pick_directory,
            commands::test_connection,
            commands::open_path,
            commands::check_update
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
