use fan_core::config::LLM_PROVIDERS;
use std::io::{self, Write};
use fan_core::config::Config;

pub fn run(config: &Config) {
    println!();
    println!("  ╔══════════════════════════════════════╗");
    println!("  ║   Fan-Files 初始化配置向导          ║");
    println!("  ╚══════════════════════════════════════╝");
    println!();
    let mut new_config = config.clone();

    // Step 1: Scan directories
    run_step_1(&mut new_config);

    // Step 2: LLM
    run_step_2(&mut new_config);

    // Step 3: Start
    run_step_3(&new_config);
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
    println!("  ▸ 步骤 1/3：扫描目录");
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
    config.scan.include = dirs.clone();
    config.watch.include = dirs;
    println!();
}

fn run_step_2(config: &mut Config) {
    println!("  ▸ 步骤 2/3：LLM 元数据推断");
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
                Ok(_) => println!("✅ 连接成功"),
                Err(_) => println!("⚠️ 连接失败（配置已保存，可稍后修改）"),
            }
        }
    }
    println!();
}

fn run_step_3(config: &Config) {
    println!("  ▸ 步骤 3/3：开始扫描");
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
            println!("  已启动后台扫描。'fan-files status' 查看进度。");
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
