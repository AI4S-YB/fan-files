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
