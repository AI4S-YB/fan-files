use std::io::{self, Write};
use std::process::Command;

pub fn run() {
    println!();
    println!("  ⚠ 即将卸载 fan-files");
    println!();
    println!("  [1] 仅卸载程序");
    println!("      删除: /usr/local/bin/fan-files + 源码 + skill");
    println!("      保留: ~/.fan-files/ (数据库、配置、模型、插件)");
    println!();
    println!("  [2] 完全卸载");
    println!("      删除: 程序 + 源码 + skill + ~/.fan-files/ 全部数据");
    println!();
    println!("  [q] 取消");
    println!();
    print!("  请输入: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();

    let home = std::env::var("HOME").unwrap_or_default();
    let source_file = format!("{}/.fan-files/install_source", home);
    let source_dir = std::fs::read_to_string(&source_file).unwrap_or_default().trim().to_string();

    match input.trim() {
        "1" => {
            // Remove binary
            println!("  删除 /usr/local/bin/fan-files...");
            let _ = Command::new("sudo").args(["rm", "/usr/local/bin/fan-files"]).status();

            // Remove source
            if !source_dir.is_empty() && std::path::Path::new(&source_dir).exists() {
                println!("  删除源码: {}", source_dir);
                let _ = std::fs::remove_dir_all(&source_dir);
            }

            // Remove skill
            let skill_path = format!("{}/.claude/skills/fan-files.md", home);
            if std::path::Path::new(&skill_path).exists() {
                println!("  删除 Claude Code Skill...");
                let _ = std::fs::remove_file(&skill_path);
            }

            println!();
            println!("  ✅ fan-files 程序已卸载");
            println!("  数据保留在 ~/.fan-files/，重新安装后可直接使用");
        }
        "2" => {
            // Full uninstall
            println!("  删除 /usr/local/bin/fan-files...");
            let _ = Command::new("sudo").args(["rm", "/usr/local/bin/fan-files"]).status();

            if !source_dir.is_empty() && std::path::Path::new(&source_dir).exists() {
                println!("  删除源码: {}", source_dir);
                let _ = std::fs::remove_dir_all(&source_dir);
            }

            let skill_path = format!("{}/.claude/skills/fan-files.md", home);
            if std::path::Path::new(&skill_path).exists() {
                println!("  删除 Claude Code Skill...");
                let _ = std::fs::remove_file(&skill_path);
            }

            let fan_dir = format!("{}/.fan-files", home);
            if std::path::Path::new(&fan_dir).exists() {
                println!("  删除 ~/.fan-files/ (数据库、配置、模型)...");
                let _ = std::fs::remove_dir_all(&fan_dir);
            }

            println!();
            println!("  ✅ fan-files 已完全卸载");
        }
        _ => {
            println!("  已取消");
        }
    }
}
