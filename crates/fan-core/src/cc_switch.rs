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

/// 读取 CC Switch 当前激活 profile 的 API 配置。
/// 目录：默认 ~/.cc-switch；FAN_CC_SWITCH_DIR 环境变量可覆盖（测试用）。
/// 返回 None = 未找到/格式变化（调用方报"未找到 CC Switch 配置"）。
pub fn cc_switch_endpoint() -> Option<LlmEndpoint> {
    parse_cc_switch_dir(&cc_switch_dir())
}

/// 从指定目录读取 CC Switch 配置（独立于环境变量，便于测试复用）
fn parse_cc_switch_dir(dir: &Path) -> Option<LlmEndpoint> {
    // 1. state.json → 当前激活 profile（还可能有 lastSyncedAt 等字段，忽略）
    let state: serde_json::Value = read_json(&dir.join("state.json"))?;
    let profile = state.get("activeProfile")?.as_str()?;

    // 2. profiles/<profile>/settings.json → 顶层 env 对象（也可能有顶层 model）
    let settings: serde_json::Value =
        read_json(&dir.join("profiles").join(profile).join("settings.json"))?;
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
}

