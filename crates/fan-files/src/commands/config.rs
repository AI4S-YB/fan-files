//! 配置查询子命令（GUI 设置页用，输出 JSON）

/// `fan-files config cc-switch`：输出 CC Switch 的 LLM 端点配置（JSON）。
///
/// 无参数 → 当前激活 profile 的完整端点 `{"api_type","base_url","api_key","model"}`；
/// `--list` → 全部 profile 摘要 JSON 数组 `[{"name","api_type","model"},...]`（按名排序）；
/// `--profile <name>` → 指定 profile 的完整端点。
/// 无配置 / profile 不存在 → `{"error":"not-found"}` + 退出码 1（GUI 据此提示"未找到 CC Switch 配置"）。
pub fn cc_switch(list: bool, profile: Option<String>) {
    let not_found = || {
        println!("{}", r#"{"error":"not-found"}"#);
        std::process::exit(1);
    };

    // --list：列出全部 profile 摘要（不要求 activeProfile 存在）
    if list {
        let profiles = fan_core::cc_switch::cc_switch_profiles();
        println!(
            "{}",
            serde_json::to_string(&profiles).unwrap_or_else(|_| "[]".into())
        );
        return;
    }

    // --profile <name>：指定 profile；否则默认（当前激活 profile）
    let ep = match profile {
        Some(name) => fan_core::cc_switch::cc_switch_endpoint_for(&name),
        None => fan_core::cc_switch::cc_switch_endpoint(),
    };
    match ep {
        Some(ep) => {
            println!(
                "{}",
                serde_json::to_string(&ep).unwrap_or_else(|_| r#"{"error":"not-found"}"#.into())
            );
        }
        None => not_found(),
    }
}
