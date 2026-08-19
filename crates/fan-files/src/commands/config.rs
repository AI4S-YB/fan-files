//! 配置查询子命令（GUI 设置页用，输出 JSON）

/// `fan-files config cc-switch`：输出 CC Switch 当前激活 profile 的 LLM 端点。
///
/// 成功 → `{"api_type","base_url","api_key","model"}` JSON；
/// 无配置 / 格式不支持 → `{"error":"not-found"}` + 退出码 1（GUI 据此提示"未找到 CC Switch 配置"）。
pub fn cc_switch() {
    match fan_core::cc_switch::cc_switch_endpoint() {
        Some(ep) => {
            println!("{}", serde_json::to_string(&ep).unwrap_or_else(|_| r#"{"error":"not-found"}"#.into()));
        }
        None => {
            println!("{}", r#"{"error":"not-found"}"#);
            std::process::exit(1);
        }
    }
}
