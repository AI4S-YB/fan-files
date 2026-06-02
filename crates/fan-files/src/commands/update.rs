use std::process::Command;

pub fn run() {
    let home = std::env::var("HOME").unwrap_or_default();
    let source_file = format!("{}/.fan-files/install_source", home);
    let source_dir = std::fs::read_to_string(&source_file).unwrap_or_default();
    let source_dir = source_dir.trim();

    if source_dir.is_empty() || !std::path::Path::new(source_dir).exists() {
        eprintln!("无法找到源码目录（可能由包管理器安装）。");
        eprintln!("重新运行安装脚本获取最新版本：");
        eprintln!("  curl -fsSL https://raw.githubusercontent.com/AI4S-YB/fan-files/main/install.sh | bash");
        return;
    }

    println!("▸ 更新源码: {}", source_dir);
    let pull = Command::new("git")
        .args(["-C", source_dir, "pull"])
        .status();

    if pull.is_err() || !pull.unwrap().success() {
        eprintln!("git pull 失败，请检查网络或手动更新");
        return;
    }

    println!("▸ 重新编译...");
    let build = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(source_dir)
        .status();

    if build.is_err() || !build.unwrap().success() {
        eprintln!("编译失败");
        return;
    }

    println!("▸ 安装二进制...");
    let bin_path = format!("{}/target/release/fan-files", source_dir);
    let install = Command::new("sudo")
        .args(["cp", &bin_path, "/usr/local/bin/fan-files"])
        .status();

    if install.is_ok() && install.unwrap().success() {
        // Update skill
        let skill_src = format!("{}/SKILL.md", source_dir);
        let skill_dst = format!("{}/.claude/skills/fan-files.md", home);
        std::fs::copy(&skill_src, &skill_dst).ok();
        println!("✅ fan-files 已升级到最新版本，skill 已同步更新");
    } else {
        eprintln!("安装失败");
    }
}
