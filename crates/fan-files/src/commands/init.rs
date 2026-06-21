use fan_core::config::LLM_PROVIDERS;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use fan_core::config::{Config, ServerConfig};

pub fn run(config: &Config) {
    println!();
    println!("  ╔══════════════════════════════════════╗");
    println!("  ║   Fan-Files 初始化配置向导          ║");
    println!("  ╚══════════════════════════════════════╝");
    println!();
    let mut new_config = config.clone();

    // Step 1: Local directories
    run_step_1(&mut new_config);

    // Step 2: Remote servers (NEW)
    run_step_servers(&mut new_config);

    // Step 3: LLM
    run_step_3(&mut new_config);

    // Step 4: Start
    run_step_4(&new_config);
    println!("  配置已保存。");
}

fn ask(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().to_string()
}

fn run_step_1(config: &mut Config) {
    println!("  ▸ 步骤 1/4：本地扫描目录");
    println!();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut dirs: Vec<String> = if config.scan.include.is_empty() {
        vec![home.clone()]
    } else {
        config.scan.include.clone()
    };

    loop {
        println!("  当前扫描目录:");
        for (i, d) in dirs.iter().enumerate() {
            println!("    [{}] {}", i + 1, d);
        }
        println!();
        println!("  [a] 添加目录  [d] 删除目录  [Enter] 完成");
        let input = ask("  请输入: ");

        match input.as_str() {
            "a" | "A" => {
                let path = ask("  请输入目录路径: ");
                if !path.is_empty() && !dirs.contains(&path) {
                    dirs.push(path);
                }
            }
            "d" | "D" => {
                let n = ask("  输入要删除的序号: ");
                if let Ok(idx) = n.parse::<usize>() {
                    if idx > 0 && idx <= dirs.len() {
                        dirs.remove(idx - 1);
                    }
                }
            }
            "" => break,
            _ => println!("  无效输入"),
        }
    }
    // Store local directories under [servers.local]
    if !dirs.is_empty() {
        config.servers.servers.entry("local".to_string()).or_insert(ServerConfig {
            host: String::new(),
            scan_roots: dirs.clone(),
            label: Some("Mac mini 本地".to_string()),
            enabled: true,
        });
    }
    config.scan.include = dirs.clone();
    config.watch.include = dirs;
    println!();
}

fn run_step_servers(config: &mut Config) {
    println!("  ▸ 步骤 2/4：远程服务器（SSH）");
    println!();
    println!("  fan-files 可以通过 SSH 扫描远程服务器上的数据目录。");
    println!("  前提：~/.ssh/config 中已配置对应 Host。");
    println!();

    loop {
        let existing: Vec<String> = config.servers.servers
            .iter()
            .filter(|(n, _)| *n != "local")
            .map(|(n, _)| n.clone())
            .collect();

        if !existing.is_empty() {
            println!("  当前远程服务器:");
            for (i, name) in existing.iter().enumerate() {
                if let Some(cfg) = config.servers.servers.get(name) {
                    println!("    [{}] {} — ssh {} '{}'", i + 1, name, cfg.host, cfg.scan_roots.join(", "));
                }
            }
            println!();
        }

        println!("  [a] 添加服务器  [d] 删除服务器  [Enter] 继续");
        let input = ask("  请输入: ");

        match input.as_str() {
            "a" | "A" => {
                let name = ask("  服务器名称 (如 ai-srv): ");
                if name.is_empty() { continue; }
                let host = ask(&format!("  SSH Host [{}]: ", name));
                let host = if host.is_empty() { name.clone() } else { host };

                let mut roots: Vec<String> = Vec::new();
                loop {
                    let root = ask("  扫描根目录 [/] (空行结束): ");
                    if root.is_empty() {
                        if roots.is_empty() { roots.push("/".to_string()); }
                        break;
                    }
                    roots.push(root);
                }

                let label = ask("  描述 (可选): ");

                // Test SSH
                print!("  测试 SSH 连接... ");
                io::stdout().flush().ok();
                let result = Command::new("ssh")
                    .args(["-o", "ConnectTimeout=5", "-o", "BatchMode=yes", &host, "echo ok"])
                    .output();
                match result {
                    Ok(out) if out.status.success() => println!("OK"),
                    _ => println!("连接失败（可稍后重试）"),
                }

                config.servers.servers.insert(name, ServerConfig {
                    host,
                    scan_roots: roots,
                    label: if label.is_empty() { None } else { Some(label) },
                    enabled: true,
                });
            }
            "d" | "D" => {
                if existing.is_empty() {
                    println!("  没有可删除的服务器");
                    continue;
                }
                let n = ask("  输入要删除的序号: ");
                if let Ok(idx) = n.parse::<usize>() {
                    if idx > 0 && idx <= existing.len() {
                        config.servers.servers.remove(&existing[idx - 1]);
                        println!("  已删除");
                    }
                }
            }
            "" => break,
            _ => println!("  无效输入"),
        }
    }
    println!();
}

fn run_step_3(config: &mut Config) {
    println!("  ▸ 步骤 3/4：LLM 元数据推断");
    println!();
    println!("  LLM 可自动识别项目、物种、实验类型。请选择:");
    for (i, p) in LLM_PROVIDERS.iter().enumerate() {
        println!("  [{}] {} — {}", i + 1, p.name, p.description);
    }
    println!("  [s] 暂时跳过");

    let input = ask("  请输入: ");

    if let Ok(idx) = input.parse::<usize>() {
        if idx > 0 && idx <= LLM_PROVIDERS.len() {
            let provider = &LLM_PROVIDERS[idx - 1];
            if !provider.endpoint.is_empty() {
                config.llm.endpoint = provider.endpoint.to_string();
            } else {
                let ep = ask("  endpoint: ");
                config.llm.endpoint = ep;
            }
            config.llm.model = provider.default_model.to_string();
            let key = ask("  API Key: ");
            config.llm.api_key = key;

            // Quick connection test
            print!("  测试连接... ");
            io::stdout().flush().ok();
            let client = fan_core::llm::LlmClient::new(config.llm.clone());
            match client.infer_candidates("test") {
                Ok(_) => println!("连接成功"),
                Err(_) => println!("连接失败（配置已保存，可稍后修改）"),
            }
        }
    }
    println!();
}

fn run_step_4(config: &Config) {
    println!("  ▸ 步骤 4/4：开始扫描");
    println!();
    println!("  是否现在开始扫描和推断？");
    println!("  [1] 后台运行（推荐）");
    println!("  [2] 前台运行");
    println!("  [3] 稍后手动 'fan-files daemon'");

    let input = ask("  请输入: ");

    // Save config first
    let config_path = fan_core::config::dirs_fan().join("config.toml");
    std::fs::create_dir_all(fan_core::config::dirs_fan()).ok();
    if let Ok(toml_str) = toml::to_string_pretty(config) {
        std::fs::write(&config_path, toml_str).ok();
    }

    match input.as_str() {
        "1" => {
            println!("  启动后台扫描（含远程服务器）...");
            let log_path = fan_core::config::dirs_fan().join("daemon.log");

            match std::env::current_exe() {
                Ok(bin) => {
                    let log_file = std::fs::File::create(&log_path)
                        .expect("Failed to create daemon log");
                    let result = Command::new(&bin)
                        .arg("daemon")
                        .stdin(Stdio::null())
                        .stdout(Stdio::from(log_file))
                        .stderr(Stdio::null())
                        .spawn();

                    match result {
                        Ok(child) => {
                            println!("  后台扫描已启动 (PID: {})", child.id());
                            println!("  日志: {}", log_path.display());
                            println!("  'fan-files status' 查看进度");
                        }
                        Err(e) => {
                            eprintln!("  启动失败: {}", e);
                            eprintln!("  请手动运行 'fan-files daemon'");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  找不到可执行文件: {}", e);
                    eprintln!("  请手动运行 'fan-files daemon'");
                }
            }
        }
        "2" => {
            println!("  启动扫描（Ctrl+C 停止）...");
            crate::commands::daemon::run(config);
        }
        _ => {
            println!("  完成。运行 'fan-files daemon' 开始扫描。");
        }
    }
}
