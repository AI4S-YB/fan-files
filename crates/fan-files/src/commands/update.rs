use std::io::Read;
use std::process::Command;

const GITHUB_RELEASES: &str = "https://github.com/AI4S-YB/fan-files/releases/download";

pub fn run() {
    let current = crate::version::VERSION;

    // Detect platform
    let target = match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => "aarch64-apple-darwin",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        _ => {
            eprintln!(
                "Unsupported platform: {}-{}",
                std::env::consts::ARCH,
                std::env::consts::OS
            );
            return;
        }
    };

    println!("fan-files update");
    println!("  Current:  {}", current);
    println!("  Platform: {}", target);

    let bin_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Cannot detect binary path: {}", e);
            return;
        }
    };
    let is_global =
        bin_path.starts_with("/usr/local/bin") || bin_path.starts_with("/usr/bin");
    println!(
        "  Install:  {} ({})",
        bin_path.display(),
        if is_global { "global" } else { "user" }
    );

    // Fetch latest release
    println!("  Fetching latest release...");
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(10)))
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = match agent
        .get("https://api.github.com/repos/AI4S-YB/fan-files/releases/latest")
        .header("User-Agent", "fan-files-updater")
        .header("Accept", "application/vnd.github.v3+json")
        .call()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  Failed to fetch release: {}", e);
            return;
        }
    };
    let json: serde_json::Value = match response.body_mut().read_json() {
        Ok(j) => j,
        Err(e) => {
            eprintln!("  Failed to parse release: {}", e);
            return;
        }
    };
    let tag = json["tag_name"].as_str().unwrap_or("unknown");
    println!("  Latest:   {}", tag);

    if tag == current || format!("v{}", current) == tag {
        println!("  ✅ Already up to date.");
        return;
    }

    // Download
    let asset_name = format!("fan-files-{}.tar.gz", target);
    let download_url = format!("{}/{}/{}", GITHUB_RELEASES, tag, asset_name);
    println!("  Downloading {}...", download_url);

    let mut response = match agent.get(&download_url).call() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  Download failed: {}", e);
            return;
        }
    };

    let mut data = Vec::new();
    if let Err(e) = response.body_mut().as_reader().read_to_end(&mut data) {
        eprintln!("  Read failed: {}", e);
        return;
    }
    println!("  Downloaded {} bytes", data.len());

    // Extract
    let temp_dir = std::env::temp_dir().join("fan-files-update");
    std::fs::create_dir_all(&temp_dir).ok();
    let tar_path = temp_dir.join("update.tar.gz");
    std::fs::write(&tar_path, &data).ok();
    let status = Command::new("tar")
        .args([
            "-xzf",
            tar_path.to_str().unwrap(),
            "-C",
            temp_dir.to_str().unwrap(),
        ])
        .status();
    if status.is_err() || !status.unwrap().success() {
        eprintln!("  Failed to extract archive");
        return;
    }

    let new_bin = temp_dir.join("fan-files");
    if !new_bin.exists() {
        eprintln!("  Binary not found in archive");
        return;
    }

    if is_global {
        let status = Command::new("sudo")
            .args([
                "cp",
                new_bin.to_str().unwrap(),
                bin_path.to_str().unwrap(),
            ])
            .status();
        if status.is_ok() && status.unwrap().success() {
            println!("  ✅ Upgraded to {} (global)", tag);
        } else {
            eprintln!("  Failed to install (try: sudo fan-files update)");
        }
    } else {
        std::fs::copy(&new_bin, &bin_path).ok();
        println!("  ✅ Upgraded to {} (user)", tag);
    }

    std::fs::remove_dir_all(&temp_dir).ok();
}
