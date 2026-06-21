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

pub fn scan_one_inner(name: &str, use_agent: bool) {
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
        if use_agent {
            scan_remote_with_agent(&config, name, &server_cfg);
        } else {
            scan_remote_with_cache(&config, name, &server_cfg);
        }
    }
}

fn scan_remote_with_cache(config: &Config, name: &str, server_cfg: &ServerConfig) {
    println!("Scanning remote server '{}' in {}...", name, server_cfg.scan_root);
    let remote = fan_core::scanner::RemoteScanner::new(
        name.to_string(),
        server_cfg.host.clone(),
        server_cfg.scan_root.clone(),
    );
    match remote.scan(false) {
        Ok(entries) => {
            let index = fan_core::index::open_index(config, fan_core::index::IndexMode::ReadWrite)
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

fn scan_remote_with_agent(config: &Config, name: &str, server_cfg: &ServerConfig) {
    println!("Scanning remote server '{}' with fan-agent...", name);

    let agent_path = "$HOME/.fan-agent/fan-agent";

    // Step 1: Deploy fan-agent if needed
    let check_cmd = format!("test -x {} && echo ok || echo missing", agent_path);
    let check_result = std::process::Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes",
               &server_cfg.host, &check_cmd])
        .output();

    let needs_deploy = match check_result {
        Ok(ref out) if String::from_utf8_lossy(&out.stdout).trim() == "missing" => true,
        Err(_) => {
            eprintln!("  SSH check failed, falling back to cache mode");
            return scan_remote_with_cache(config, name, server_cfg);
        }
        _ => false,
    };

    if needs_deploy {
        println!("  deploying fan-agent...");
        // Find fan-agent binary alongside fan-files
        let agent_src = match std::env::current_exe() {
            Ok(exe) => {
                let dir = exe.parent().unwrap().to_path_buf();
                dir.join("fan-agent-x86_64")
            }
            Err(_) => {
                eprintln!("  cannot locate fan-agent binary, falling back to cache mode");
                return scan_remote_with_cache(config, name, server_cfg);
            }
        };

        if agent_src.exists() {
            let scp_result = std::process::Command::new("scp")
                .args([agent_src.to_str().unwrap(),
                       &format!("{}:~/.fan-agent/fan-agent", server_cfg.host)])
                .output();
            match scp_result {
                Ok(ref out) if out.status.success() => {
                    let _ = std::process::Command::new("ssh")
                        .args(["-o", "BatchMode=yes", &server_cfg.host,
                               "chmod +x ~/.fan-agent/fan-agent"])
                        .output();
                    println!("  fan-agent deployed");
                }
                _ => {
                    eprintln!("  scp failed, falling back to cache mode");
                    return scan_remote_with_cache(config, name, server_cfg);
                }
            }
        } else {
            eprintln!("  fan-agent binary not found, falling back to cache mode");
            return scan_remote_with_cache(config, name, server_cfg);
        }
    }

    // Step 2: Run fan-agent and pipe JSONL
    println!("  scanning...");
    let scan_cmd = format!("{} scan --root '{}' 2>/dev/null",
        agent_path, server_cfg.scan_root.replace('\'', "'\\''"));

    let mut child = match std::process::Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes",
               &server_cfg.host, &scan_cmd])
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  SSH spawn failed: {}, falling back to cache mode", e);
            return scan_remote_with_cache(config, name, server_cfg);
        }
    };

    let index = fan_core::index::open_index(config, fan_core::index::IndexMode::ReadWrite)
        .expect("Failed to open index");
    let mut count = 0u64;

    use std::io::{BufRead, BufReader};
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        if let Ok(line) = line {
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) {
                let path = entry["path"].as_str().unwrap_or("");
                let size = entry["size"].as_u64().unwrap_or(0);
                let mtime = entry["mtime_secs"].as_i64().unwrap_or(0);

                let info = fan_core::types::RawFileInfo {
                    path: std::path::PathBuf::from(path),
                    source_server: name.to_string(),
                    size,
                    mtime_secs: mtime,
                    hash_sha256: None,
                    magic_bytes: Vec::new(),
                    mime_type: mime_guess::from_path(path)
                        .first_or_octet_stream()
                        .to_string(),
                };

                match index.index_file(&info, None) {
                    Ok(_) => count += 1,
                    Err(e) => eprintln!("Failed to index {}: {}", path, e),
                }
            }
        }
    }

    let _ = child.wait();
    index.tantivy.commit().ok();
    println!("Scanned {}: {} files indexed (via fan-agent)", name, count);
}

pub fn watch_remote(name: &str) {
    let config = Config::load().expect("Failed to load config");
    let server_cfg = match config.servers.servers.get(name) {
        Some(c) if c.enabled && !c.host.is_empty() => c.clone(),
        Some(_) => {
            eprintln!("Server '{}' is local or disabled. Watch mode only for remote servers.", name);
            return;
        }
        None => {
            eprintln!("Server '{}' not found.", name);
            return;
        }
    };

    let agent_path = "$HOME/.fan-agent/fan-agent";

    // Deploy agent
    let check_cmd = format!("test -x {} && echo ok || echo missing", agent_path);
    let check_result = std::process::Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", &server_cfg.host, &check_cmd])
        .output();

    if let Ok(ref out) = check_result {
        if String::from_utf8_lossy(&out.stdout).trim() == "missing" {
            println!("Deploying fan-agent to {}...", name);
            let agent_src = std::env::current_exe().ok()
                .and_then(|e| e.parent().map(|p| p.join("fan-agent-x86_64")));
            if let Some(src) = agent_src {
                if src.exists() {
                    let _ = std::process::Command::new("scp")
                        .args([src.to_str().unwrap(), &format!("{}:~/.fan-agent/fan-agent", server_cfg.host)])
                        .output();
                    let _ = std::process::Command::new("ssh")
                        .args(["-o", "BatchMode=yes", &server_cfg.host, "chmod +x ~/.fan-agent/fan-agent"])
                        .output();
                }
            }
        }
    }

    println!("Starting real-time watch on {}:{}...", name, server_cfg.scan_root);
    println!("(press Ctrl+C to stop)");
    println!();

    let watch_cmd = format!(
        "{} watch --root '{}' 2>/dev/null",
        agent_path,
        server_cfg.scan_root.replace('\'', "'\\''")
    );

    let mut child = std::process::Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", &server_cfg.host, &watch_cmd])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start SSH watch");

    let index = fan_core::index::open_index(&config, fan_core::index::IndexMode::ReadWrite)
        .expect("Failed to open index");

    use std::io::{BufRead, BufReader};
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut add_count = 0u64;
    let mut del_count = 0u64;

    for line in reader.lines() {
        if let Ok(line) = line {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&line) {
                let event_type = ev["event"].as_str().unwrap_or("");
                let path_str = ev["path"].as_str().unwrap_or("");

                match event_type {
                    "add" | "mod" => {
                        let size = ev["size"].as_u64().unwrap_or(0);
                        let mtime = ev["mtime_secs"].as_i64().unwrap_or(0);
                        let info = fan_core::types::RawFileInfo {
                            path: std::path::PathBuf::from(path_str),
                            source_server: name.to_string(),
                            size,
                            mtime_secs: mtime,
                            hash_sha256: None,
                            magic_bytes: Vec::new(),
                            mime_type: mime_guess::from_path(path_str)
                                .first_or_octet_stream().to_string(),
                        };
                        match index.index_file(&info, None) {
                            Ok(_) => {
                                add_count += 1;
                                println!("  + {}", path_str);
                            }
                            Err(e) => eprintln!("  ! {}: {}", path_str, e),
                        }
                    }
                    "del" => {
                        match index.sqlite.mark_deleted(std::path::Path::new(path_str)) {
                            Ok(_) => {
                                del_count += 1;
                                println!("  - {}", path_str);
                            }
                            Err(e) => eprintln!("  ! {}: {}", path_str, e),
                        }
                    }
                    _ => {}
                }

                // Commit every 100 events
                if (add_count + del_count) % 100 == 0 {
                    index.tantivy.commit().ok();
                    eprintln!("  [{} added, {} deleted]", add_count, del_count);
                }
            }
        }
    }

    index.tantivy.commit().ok();
    let _ = child.wait();
    println!("Watch ended: {} added, {} deleted", add_count, del_count);
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
