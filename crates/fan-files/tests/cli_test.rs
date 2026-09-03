use assert_cmd::Command;

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("--help").assert().success();
}

#[test]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("--version").assert().success();
}

#[test]
fn test_cli_status() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("status").assert().success();
}

#[test]
fn test_cli_projects() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("projects").assert().success();
}

#[test]
fn test_cli_pending() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("pending").assert().success();
}

#[test]
fn test_cli_search_runs() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("search").arg("xyz_no_match_12345").assert().success();
}

#[test]
fn test_cli_global_flag() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("--global").arg("status").assert();
}

/// `config cc-switch`：FAN_CC_SWITCH_DIR 指向不存在目录 → {"error":"not-found"} + 退出码 1
#[test]
fn test_cli_config_cc_switch_missing_returns_not_found() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.env("FAN_CC_SWITCH_DIR", "/nonexistent/cc-switch-test");
    cmd.arg("config")
        .arg("cc-switch")
        .assert()
        .code(1)
        .stdout(predicates::str::contains("not-found"));
}

/// `config cc-switch`：伪造一个 CC Switch 目录 → 输出 LlmEndpoint JSON（openai）
#[test]
fn test_cli_config_cc_switch_outputs_endpoint_json() {
    let dir = std::env::temp_dir().join(format!("fan-cc-switch-cli-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("profiles/oa")).unwrap();
    std::fs::write(
        dir.join("state.json"),
        r#"{"activeProfile":"oa"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("profiles/oa/settings.json"),
        r#"{
            "env": {
                "OPENAI_BASE_URL": "https://api.deepseek.com/v1",
                "OPENAI_API_KEY": "sk-cli-test",
                "OPENAI_MODEL": "deepseek-chat"
            }
        }"#,
    )
    .unwrap();
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.env("FAN_CC_SWITCH_DIR", &dir);
    cmd.arg("config")
        .arg("cc-switch")
        .assert()
        .success()
        .stdout(predicates::str::contains("\"api_type\":\"openai\""))
        .stdout(predicates::str::contains("\"base_url\":\"https://api.deepseek.com/v1\""))
        .stdout(predicates::str::contains("\"api_key\":\"sk-cli-test\""))
        .stdout(predicates::str::contains("\"model\":\"deepseek-chat\""));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `config cc-switch --list`：输出全部 profile 摘要 JSON 数组（按目录名排序）
#[test]
fn test_cli_config_cc_switch_list_outputs_profiles() {
    let dir = std::env::temp_dir().join(format!("fan-cc-switch-cli-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("profiles/haikou-flash")).unwrap();
    std::fs::create_dir_all(dir.join("profiles/official-pro")).unwrap();
    std::fs::write(
        dir.join("profiles/haikou-flash/settings.json"),
        r#"{
            "model": "claude-sonnet-4-8",
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-a",
                "ANTHROPIC_BASE_URL": "http://10.33.105.218:3200",
                "ANTHROPIC_MODEL": "claude-sonnet-4-8"
            }
        }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("profiles/official-pro/settings.json"),
        r#"{
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-b",
                "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                "ANTHROPIC_MODEL": "deepseek-v3"
            }
        }"#,
    )
    .unwrap();
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.env("FAN_CC_SWITCH_DIR", &dir);
    cmd.arg("config")
        .arg("cc-switch")
        .arg("--list")
        .assert()
        .success()
        .stdout(predicates::str::contains("haikou-flash"))
        .stdout(predicates::str::contains("official-pro"))
        .stdout(predicates::str::contains("\"api_type\":\"anthropic\""))
        .stdout(predicates::str::contains("\"model\":\"claude-sonnet-4-8\""));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `config cc-switch --profile <name>`：指定 profile → LlmEndpoint JSON；
/// 不存在的 profile → {"error":"not-found"} + 退出码 1
#[test]
fn test_cli_config_cc_switch_profile_by_name() {
    let dir = std::env::temp_dir().join(format!("fan-cc-switch-cli-profile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("profiles/oa")).unwrap();
    std::fs::write(
        dir.join("profiles/oa/settings.json"),
        r#"{
            "env": {
                "OPENAI_BASE_URL": "https://api.deepseek.com/v1",
                "OPENAI_API_KEY": "sk-cli-test",
                "OPENAI_MODEL": "deepseek-chat"
            }
        }"#,
    )
    .unwrap();

    // 指定的 profile 存在 → 输出该 profile 端点
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.env("FAN_CC_SWITCH_DIR", &dir);
    cmd.arg("config")
        .arg("cc-switch")
        .arg("--profile")
        .arg("oa")
        .assert()
        .success()
        .stdout(predicates::str::contains("\"api_type\":\"openai\""))
        .stdout(predicates::str::contains("\"model\":\"deepseek-chat\""));

    // 指定的 profile 不存在 → not-found + 退出码 1
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.env("FAN_CC_SWITCH_DIR", &dir);
    cmd.arg("config")
        .arg("cc-switch")
        .arg("--profile")
        .arg("nope")
        .assert()
        .code(1)
        .stdout(predicates::str::contains("not-found"));
    let _ = std::fs::remove_dir_all(&dir);
}
