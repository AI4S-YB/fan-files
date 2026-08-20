//! CC Switch（macOS LLM 模型切换工具）配置读取：接管其 API 配置用于推理

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 从 CC Switch profile 解析出的 LLM 端点
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmEndpoint {
    pub api_type: String,  // "openai" | "anthropic"
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// profile 摘要（列表用）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileInfo {
    pub name: String,
    pub api_type: String,   // "openai" | "anthropic"；无匹配协议 → 空串
    pub model: String,      // env 模型或顶层 model，无则空
}

/// 读取 CC Switch 当前激活 profile 的 API 配置。
/// 目录：默认 ~/.cc-switch；FAN_CC_SWITCH_DIR 环境变量可覆盖（测试用）。
/// 返回 None = 未找到/格式变化（调用方报"未找到 CC Switch 配置"）。
pub fn cc_switch_endpoint() -> Option<LlmEndpoint> {
    // state.json → 当前激活 profile（还可能有 lastSyncedAt 等字段，忽略）→ 按名读取
    let state: serde_json::Value = read_json(&cc_switch_dir().join("state.json"))?;
    let profile = state.get("activeProfile")?.as_str()?;
    cc_switch_endpoint_for(profile)
}

/// 指定 profile 的完整端点（name 不存在 / 无有效配置 → None）。
/// 协议识别逻辑与默认读取相同（anthropic env 优先 → openai → 顶层兜底）。
pub fn cc_switch_endpoint_for(name: &str) -> Option<LlmEndpoint> {
    let settings =
        read_json(&cc_switch_dir().join("profiles").join(name).join("settings.json"))?;
    parse_profile_settings(&settings)
}

/// 遍历 profiles/ 目录，返回全部 profile 摘要（按目录名排序）。
/// settings.json 无法读取 / 不是 JSON 对象的目录跳过。
pub fn cc_switch_profiles() -> Vec<ProfileInfo> {
    let profiles_dir = cc_switch_dir().join("profiles");
    let Ok(entries) = std::fs::read_dir(&profiles_dir) else { return Vec::new(); };

    // 收集目录名（忽略非目录项 / 非 UTF-8 名称），排序保证输出稳定
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();

    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let Some(settings) = read_json(&profiles_dir.join(&name).join("settings.json")) else { continue };
        let (api_type, model) = profile_summary(&settings);
        out.push(ProfileInfo { name, api_type, model });
    }
    out
}

/// 从单个 profile 的 settings 提取 api_type 与 model（列表摘要用）。
/// 协议判定与 parse_profile_settings 一致；不满足任何协议 → api_type 空串（profile 仍列出）。
fn profile_summary(settings: &serde_json::Value) -> (String, String) {
    let env = settings.get("env").cloned().unwrap_or(serde_json::Value::Null);
    let env_str = |k: &str| env.get(k).and_then(|v| v.as_str()).map(str::to_string);
    // model：env 模型（anthropic/openai 任一）优先，其次顶层 model，无则空
    let model = env_str("ANTHROPIC_MODEL")
        .or_else(|| env_str("OPENAI_MODEL"))
        .filter(|m| !m.is_empty())
        .or_else(|| settings.get("model").and_then(|v| v.as_str()).map(str::to_string))
        .unwrap_or_default();

    let api_type = if env_str("ANTHROPIC_BASE_URL").is_some()
        && env_str("ANTHROPIC_AUTH_TOKEN").is_some()
    {
        "anthropic"
    } else if env_str("OPENAI_API_KEY").is_some() {
        "openai"
    } else if settings.get("baseURL").and_then(|v| v.as_str()).is_some()
        && settings.get("apiKey").and_then(|v| v.as_str()).is_some()
    {
        "openai"
    } else {
        ""
    };
    (api_type.into(), model)
}

/// 从指定目录读取 CC Switch 配置（独立于环境变量，便于测试复用）
fn parse_cc_switch_dir(dir: &Path) -> Option<LlmEndpoint> {
    // 1. state.json → 当前激活 profile（还可能有 lastSyncedAt 等字段，忽略）
    let state: serde_json::Value = read_json(&dir.join("state.json"))?;
    let profile = state.get("activeProfile")?.as_str()?;

    // 2. profiles/<profile>/settings.json → 顶层 env 对象（也可能有顶层 model）
    let settings: serde_json::Value =
        read_json(&dir.join("profiles").join(profile).join("settings.json"))?;
    parse_profile_settings(&settings)
}

/// 从单个 profile 的 settings.json 解析出完整端点（共享协议识别，供默认/指定读取复用）。
/// 识别顺序：anthropic env（BASE_URL + AUTH_TOKEN）→ openai env → 顶层 baseURL/apiKey 兜底
fn parse_profile_settings(settings: &serde_json::Value) -> Option<LlmEndpoint> {
    let env = settings.get("env").cloned().unwrap_or(serde_json::Value::Null);
    let top_model = settings.get("model").and_then(|m| m.as_str()).map(str::to_string);
    let env_str = |k: &str| env.get(k).and_then(|v| v.as_str()).map(str::to_string);

    // 3. 识别协议：anthropic 优先（BASE_URL + AUTH_TOKEN）
    if let (Some(url), Some(key)) = (env_str("ANTHROPIC_BASE_URL"), env_str("ANTHROPIC_AUTH_TOKEN")) {
        // ANTHROPIC_BASE_URL 可能不带 /v1，原样保留由协议层拼
        // ANTHROPIC_MODEL 与顶层 model 均缺失 → 无有效模型，返回 None 让 GUI 提示"无 API 配置"
        let model = match env_str("ANTHROPIC_MODEL").or_else(|| top_model.clone()) {
            Some(m) if !m.is_empty() => m,
            _ => return None,
        };
        return Some(LlmEndpoint {
            api_type: "anthropic".into(),
            base_url: url,
            api_key: key,
            model,
        });
    }

    // 4. openai：BASE_URL + API_KEY；或 API_KEY + MODEL（无 URL 时 base_url 留空）
    if let Some(key) = env_str("OPENAI_API_KEY") {
        let model = env_str("OPENAI_MODEL").or_else(|| top_model.clone()).unwrap_or_default();
        if let Some(url) = env_str("OPENAI_BASE_URL") {
            return Some(LlmEndpoint {
                api_type: "openai".into(),
                base_url: url,
                api_key: key,
                model,
            });
        }
        if !model.is_empty() {
            return Some(LlmEndpoint {
                api_type: "openai".into(),
                base_url: String::new(),
                api_key: key,
                model,
            });
        }
    }

    // 5. 兜底：顶层 baseURL / apiKey / model → openai
    let top = |k: &str| settings.get(k).and_then(|v| v.as_str()).map(str::to_string);
    if let (Some(url), Some(key)) = (top("baseURL"), top("apiKey")) {
        return Some(LlmEndpoint {
            api_type: "openai".into(),
            base_url: url,
            api_key: key,
            model: top("model").or(top_model).unwrap_or_default(),
        });
    }

    None
}

/// 读取并解析 JSON 文件；不存在 / 坏 JSON / 非对象 → None
fn read_json(path: &Path) -> Option<serde_json::Value> {
    let s = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    if !v.is_object() { return None; }
    Some(v)
}

fn cc_switch_dir() -> PathBuf {
    std::env::var("FAN_CC_SWITCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".cc-switch"))
                .unwrap_or_else(|_| PathBuf::from(".cc-switch"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 环境变量在并行测试间是共享的：所有读写 FAN_CC_SWITCH_DIR 的测试串行执行
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
    }

    /// anthropic profile：env 有 ANTHROPIC_BASE_URL + AUTH_TOKEN + MODEL（真实 Mac 配置形态）
    #[test]
    fn parses_anthropic_profile() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json",
              r#"{"activeProfile":"haikou-flash","lastSyncedAt":"2026-08-18T06:24:20.885Z"}"#);
        write(dir.path(), "profiles/haikou-flash/settings.json", r#"{
            "model": "claude-sonnet-4-8",
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-test",
                "ANTHROPIC_BASE_URL": "http://10.33.105.218:3200",
                "ANTHROPIC_MODEL": "claude-sonnet-4-8"
            }
        }"#);
        let ep = parse_cc_switch_dir(dir.path()).expect("应解析出 anthropic 端点");
        assert_eq!(ep.api_type, "anthropic");
        assert_eq!(ep.base_url, "http://10.33.105.218:3200");
        assert_eq!(ep.api_key, "sk-test");
        assert_eq!(ep.model, "claude-sonnet-4-8");
    }

    /// anthropic profile 缺 ANTHROPIC_MODEL → 回退顶层 model
    #[test]
    fn anthropic_model_falls_back_to_top_level() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", r#"{"activeProfile":"p1"}"#);
        write(dir.path(), "profiles/p1/settings.json", r#"{
            "model": "claude-sonnet-4-8",
            "env": {"ANTHROPIC_AUTH_TOKEN": "sk-test", "ANTHROPIC_BASE_URL": "http://x:1"}
        }"#);
        let ep = parse_cc_switch_dir(dir.path()).unwrap();
        assert_eq!(ep.api_type, "anthropic");
        assert_eq!(ep.model, "claude-sonnet-4-8");
    }

    /// anthropic：ANTHROPIC_MODEL 与顶层 model 均缺失 → None
    /// （让 GUI 提示"无 API 配置"而非给出空 model）
    #[test]
    fn anthropic_without_model_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", r#"{"activeProfile":"p"}"#);
        write(dir.path(), "profiles/p/settings.json", r#"{
            "env": {"ANTHROPIC_AUTH_TOKEN": "sk-test", "ANTHROPIC_BASE_URL": "http://x:1"}
        }"#);
        assert!(parse_cc_switch_dir(dir.path()).is_none());
    }

    /// openai profile：env 有 OPENAI_BASE_URL + API_KEY + MODEL
    #[test]
    fn parses_openai_profile() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", r#"{"activeProfile":"oa"}"#);
        write(dir.path(), "profiles/oa/settings.json", r#"{
            "env": {
                "OPENAI_BASE_URL": "https://api.deepseek.com/v1",
                "OPENAI_API_KEY": "sk-oa",
                "OPENAI_MODEL": "deepseek-chat"
            }
        }"#);
        let ep = parse_cc_switch_dir(dir.path()).unwrap();
        assert_eq!(ep.api_type, "openai");
        assert_eq!(ep.base_url, "https://api.deepseek.com/v1");
        assert_eq!(ep.api_key, "sk-oa");
        assert_eq!(ep.model, "deepseek-chat");
    }

    /// openai：只有 API_KEY + MODEL（无 BASE_URL）→ openai，base_url 留空
    #[test]
    fn openai_key_and_model_without_url() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", r#"{"activeProfile":"oa2"}"#);
        write(dir.path(), "profiles/oa2/settings.json", r#"{
            "env": {"OPENAI_API_KEY": "sk-oa", "OPENAI_MODEL": "gpt-4o-mini"}
        }"#);
        let ep = parse_cc_switch_dir(dir.path()).unwrap();
        assert_eq!(ep.api_type, "openai");
        assert_eq!(ep.base_url, "");
        assert_eq!(ep.api_key, "sk-oa");
        assert_eq!(ep.model, "gpt-4o-mini");
    }

    /// 兜底：顶层 baseURL/apiKey/model → openai
    #[test]
    fn falls_back_to_top_level_fields() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", r#"{"activeProfile":"t"}"#);
        write(dir.path(), "profiles/t/settings.json", r#"{
            "baseURL": "https://api.example.com/v1/chat/completions",
            "apiKey": "sk-top",
            "model": "gpt-4o-mini"
        }"#);
        let ep = parse_cc_switch_dir(dir.path()).unwrap();
        assert_eq!(ep.api_type, "openai");
        assert_eq!(ep.base_url, "https://api.example.com/v1/chat/completions");
        assert_eq!(ep.api_key, "sk-top");
        assert_eq!(ep.model, "gpt-4o-mini");
    }

    /// 目录不存在 → None
    #[test]
    fn missing_dir_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(parse_cc_switch_dir(&dir.path().join("nope")).is_none());
    }

    /// 坏 JSON（state 或 settings）→ None
    #[test]
    fn bad_json_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", "not json{{");
        assert!(parse_cc_switch_dir(dir.path()).is_none());

        let dir2 = tempfile::tempdir().unwrap();
        write(dir2.path(), "state.json", r#"{"activeProfile":"p"}"#);
        write(dir2.path(), "profiles/p/settings.json", "!!bad");
        assert!(parse_cc_switch_dir(dir2.path()).is_none());
    }

    /// env 缺字段 / activeProfile 缺失 / env 类型错误 → None
    #[test]
    fn missing_fields_return_none() {
        // anthropic 有 URL 无 TOKEN → 不满足任何协议
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", r#"{"activeProfile":"p"}"#);
        write(dir.path(), "profiles/p/settings.json",
              r#"{"env":{"ANTHROPIC_BASE_URL":"http://x:1"}}"#);
        assert!(parse_cc_switch_dir(dir.path()).is_none());

        // state.json 无 activeProfile
        let dir2 = tempfile::tempdir().unwrap();
        write(dir2.path(), "state.json", r#"{"lastSyncedAt":"x"}"#);
        assert!(parse_cc_switch_dir(dir2.path()).is_none());

        // env 是字符串（格式变化）
        let dir3 = tempfile::tempdir().unwrap();
        write(dir3.path(), "state.json", r#"{"activeProfile":"p"}"#);
        write(dir3.path(), "profiles/p/settings.json", r#"{"env": "oops"}"#);
        assert!(parse_cc_switch_dir(dir3.path()).is_none());
    }

    /// FAN_CC_SWITCH_DIR 环境变量覆盖默认目录（默认路径逻辑）
    #[test]
    fn env_var_overrides_default_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", r#"{"activeProfile":"p"}"#);
        write(dir.path(), "profiles/p/settings.json", r#"{
            "env": {"OPENAI_BASE_URL": "https://x/v1", "OPENAI_API_KEY": "k", "OPENAI_MODEL": "m"}
        }"#);
        unsafe {
            std::env::set_var("FAN_CC_SWITCH_DIR", dir.path());
        }
        let ep = cc_switch_endpoint().expect("env 目录应被读取");
        assert_eq!(ep.api_type, "openai");
        assert_eq!(ep.base_url, "https://x/v1");
        unsafe {
            std::env::remove_var("FAN_CC_SWITCH_DIR");
        }
    }

    /// 遍历 profiles/ 目录：按目录名排序，返回全部 profile 摘要（api_type/model 正确）
    #[test]
    fn list_profiles_enumerates_all() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // 真实环境形态：两个 anthropic profile（中转 haikou-flash + 直连 official-pro）
        write(dir.path(), "state.json", r#"{"activeProfile":"haikou-flash"}"#);
        write(dir.path(), "profiles/haikou-flash/settings.json", r#"{
            "model": "claude-sonnet-4-8",
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-a",
                "ANTHROPIC_BASE_URL": "http://10.33.105.218:3200",
                "ANTHROPIC_MODEL": "claude-sonnet-4-8"
            }
        }"#);
        write(dir.path(), "profiles/official-pro/settings.json", r#"{
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-b",
                "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                "ANTHROPIC_MODEL": "deepseek-v3"
            }
        }"#);
        unsafe { std::env::set_var("FAN_CC_SWITCH_DIR", dir.path()); }

        let list = cc_switch_profiles();
        assert_eq!(list.len(), 2, "应枚举出 2 个 profile");
        assert_eq!(list[0].name, "haikou-flash");
        assert_eq!(list[0].api_type, "anthropic");
        assert_eq!(list[0].model, "claude-sonnet-4-8");
        assert_eq!(list[1].name, "official-pro");
        assert_eq!(list[1].api_type, "anthropic");
        assert_eq!(list[1].model, "deepseek-v3");
        unsafe { std::env::remove_var("FAN_CC_SWITCH_DIR"); }
    }

    /// 指定 profile 读取：cc_switch_endpoint_for(name) → 该 profile 端点；不存在 → None
    #[test]
    fn endpoint_for_named_profile() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "profiles/official-pro/settings.json", r#"{
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-b",
                "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                "ANTHROPIC_MODEL": "deepseek-v3"
            }
        }"#);
        unsafe { std::env::set_var("FAN_CC_SWITCH_DIR", dir.path()); }

        let ep = cc_switch_endpoint_for("official-pro").expect("应读到 official-pro 端点");
        assert_eq!(ep.api_type, "anthropic");
        assert_eq!(ep.base_url, "https://api.deepseek.com/anthropic");
        assert_eq!(ep.api_key, "sk-b");
        assert_eq!(ep.model, "deepseek-v3");
        // 不存在的 profile → None
        assert!(cc_switch_endpoint_for("nonexistent").is_none());
        unsafe { std::env::remove_var("FAN_CC_SWITCH_DIR"); }
    }

    /// 默认读取 = 激活 profile：cc_switch_endpoint() == cc_switch_endpoint_for(activeProfile)
    #[test]
    fn endpoint_defaults_to_active() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "state.json", r#"{"activeProfile":"haikou-flash"}"#);
        write(dir.path(), "profiles/haikou-flash/settings.json", r#"{
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-a",
                "ANTHROPIC_BASE_URL": "http://10.33.105.218:3200",
                "ANTHROPIC_MODEL": "claude-sonnet-4-8"
            }
        }"#);
        unsafe { std::env::set_var("FAN_CC_SWITCH_DIR", dir.path()); }

        assert_eq!(
            cc_switch_endpoint(),
            cc_switch_endpoint_for("haikou-flash"),
            "默认端点应等于 activeProfile 指定读取"
        );
        unsafe { std::env::remove_var("FAN_CC_SWITCH_DIR"); }
    }
}

