use fan_core::config::{Config, ServerConfig};
use std::io::{self, Write};

pub fn list(config: &Config) {
    let servers = config.enabled_servers();
    if servers.is_empty() {
        println!("No servers configured.");
        println!("Use 'fan-files servers add <name>' to add one.");
        return;
    }
    println!("{:<15} {:<20} {:<40} {}", "Server", "Host", "Scan Root", "Label");
    println!("{}", "-".repeat(90));
    for (name, cfg) in &servers {
        let host = if cfg.host.is_empty() { "localhost" } else { &cfg.host };
        let label = cfg.label.as_deref().unwrap_or("-");
        println!("{:<15} {:<20} {:<40} {}", name, host, cfg.scan_root, label);
    }

    let disabled: Vec<_> = config.servers.servers.iter()
        .filter(|(_, cfg)| !cfg.enabled)
        .collect();
    if !disabled.is_empty() {
        println!();
        println!("Disabled servers:");
        for (name, cfg) in &disabled {
            let host = if cfg.host.is_empty() { "localhost" } else { &cfg.host };
            println!("  {} (host={}, root={})", name, host, cfg.scan_root);
        }
    }
}

pub fn add(name: &str) {
    println!("Adding server: {}", name);
    println!();

    let host = ask(&format!("SSH Host (from ~/.ssh/config, empty for local) [{}]: ", name));
    let host = if host.is_empty() { name.to_string() } else { host };

    let default_root = "/";
    let root_prompt = format!("Scan root directory [{}]: ", default_root);
    let scan_root = ask_with_default(&root_prompt, default_root);

    let label = ask("Label (optional): ");

    if !host.is_empty() {
        print!("Testing SSH connection to {}... ", host);
        io::stdout().flush().ok();
        let result = std::process::Command::new("ssh")
            .args(["-o", "ConnectTimeout=5", "-o", "BatchMode=yes", &host, "echo ok"])
            .output();
        match result {
            Ok(output) if output.status.success() => println!("OK"),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                println!("Failed: {}", stderr.trim());
            }
            Err(e) => println!("Error: {}", e),
        }

        print!("Counting files remotely (find)... ");
        io::stdout().flush().ok();
        let find_cmd = format!("find '{}' -type f 2>/dev/null | wc -l", scan_root.replace('\'', "'\\''"));
        match std::process::Command::new("ssh")
            .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", &host, &find_cmd])
            .output()
        {
            Ok(output) if output.status.success() => {
                let count: String = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("{} files found", count);
            }
            _ => println!("could not count (will retry during scan)"),
        }
    }

    let mut config = Config::load().expect("Failed to load config");
    config.servers.servers.insert(name.to_string(), ServerConfig {
        host,
        scan_root,
        label: if label.is_empty() { None } else { Some(label) },
        enabled: true,
    });

    let config_path = fan_core::config::dirs_fan().join("config.toml");
    std::fs::create_dir_all(fan_core::config::dirs_fan()).ok();
    if let Ok(toml_str) = toml::to_string_pretty(&config) {
        std::fs::write(&config_path, toml_str).ok();
        println!("Server '{}' added to config.", name);
    } else {
        eprintln!("Failed to serialize config.");
    }
}

pub fn remove(name: &str) {
    let mut config = Config::load().expect("Failed to load config");
    if config.servers.servers.remove(name).is_some() {
        let config_path = fan_core::config::dirs_fan().join("config.toml");
        if let Ok(toml_str) = toml::to_string_pretty(&config) {
            std::fs::write(&config_path, toml_str).ok();
            println!("Server '{}' removed.", name);
        }
    } else {
        eprintln!("Server '{}' not found in config.", name);
    }
}

pub fn scan_one(name: &str) {
    let config = Config::load().expect("Failed to load config");
    let server_cfg = match config.servers.servers.get(name) {
        Some(c) if c.enabled => c.clone(),
        Some(_) => {
            eprintln!("Server '{}' is disabled. Enable it first.", name);
            return;
        }
        None => {
            eprintln!("Server '{}' not found. Use 'fan-files servers list'.", name);
            return;
        }
    };

    if server_cfg.host.is_empty() {
        println!("Scanning local server '{}' in {}...", name, server_cfg.scan_root);
        let scanner = fan_core::scanner::Scanner::new(
            vec![server_cfg.scan_root.clone()],
            vec!["/tmp".into(), "*.tmp".into()],
            name.to_string(),
        );
        let index = fan_core::index::open_index(&config, fan_core::index::IndexMode::ReadWrite)
            .expect("Failed to open index");
        let mut count = 0u64;
        for file_info in scanner.scan() {
            match index.index_file(&file_info, None) {
                Ok(_) => count += 1,
                Err(e) => eprintln!("Failed to index {}: {}", file_info.path.display(), e),
            }
        }
        index.tantivy.commit().ok();
        println!("Scanned {}: {} files indexed", name, count);
    } else {
        println!("Scanning remote server '{}' in {}...", name, server_cfg.scan_root);
        let remote = fan_core::scanner::RemoteScanner::new(
            name.to_string(),
            server_cfg.host.clone(),
            server_cfg.scan_root.clone(),
        );
        match remote.scan(false) {
            Ok(entries) => {
                let index = fan_core::index::open_index(&config, fan_core::index::IndexMode::ReadWrite)
                    .expect("Failed to open index");
                let mut count = 0u64;
                for file_info in &entries {
                    match index.index_file(file_info, None) {
                        Ok(_) => count += 1,
                        Err(e) => eprintln!("Failed to index {}: {}", file_info.path.display(), e),
                    }
                }
                index.tantivy.commit().ok();
                println!("Scanned {}: {} files indexed", name, count);
            }
            Err(e) => eprintln!("Remote scan failed: {}", e),
        }
    }
}

fn ask(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().to_string()
}

fn ask_with_default(prompt: &str, default: &str) -> String {
    let input = ask(prompt);
    if input.is_empty() { default.to_string() } else { input }
}
