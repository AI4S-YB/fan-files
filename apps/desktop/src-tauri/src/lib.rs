//! fan-files desktop shell（Tauri 2）。
//!
//! 结构：config.rs（FanConfig 与 config.toml 读写）、commands.rs（前端 invoke 命令层）、
//! 本文件只做模块声明与 Tauri 应用组装。

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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
