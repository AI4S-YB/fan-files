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
