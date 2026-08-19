pub mod prompt;

use crate::config::LlmConfig;
use prompt::{LlmOutput, system_prompt};
use std::time::Duration;
use tracing::info;

pub struct LlmClient {
    pub config: LlmConfig,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self { config }
    }

    pub fn is_configured(&self) -> bool {
        !self.config.endpoint.is_empty() && !self.config.api_key.is_empty()
    }

    /// Send directory summary to LLM, return parsed project list
    pub fn infer_projects(
        &self,
        dir_summary: &str,
    ) -> Result<LlmOutput, Box<dyn std::error::Error>> {
        let user_msg = format!(
            "{}\n\n请分析以上目录结构，返回 JSON。",
            dir_summary
        );

        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": system_prompt()},
                {"role": "user", "content": user_msg}
            ],
            "response_format": {"type": "json_object"},
            "temperature": 0.1,
            "max_tokens": 16384
        });

        let json: serde_json::Value = llm_api_call_with_retry(&self.config, &body, 3)?;
        let content = extract_llm_text(&self.config, &json)
            .ok_or("No content in LLM response")?;

        info!("LLM raw response: {}", content);
        prompt::parse_llm_response(&content)
            .map_err(|e| format!("Failed to parse LLM JSON output: {}", e).into())
    }

    /// 通用对话补全：传入消息列表，返回助手文本（原样，不做结构拆分）。
    /// body 带 response_format=json_object + max_tokens（openai 协议透传；
    /// anthropic 不接收额外字段，适配层自动丢弃）。chat-search 端点使用。
    pub fn chat(&self, messages: &[serde_json::Value]) -> Result<String, Box<dyn std::error::Error>> {
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": messages,
            "response_format": {"type": "json_object"},
            "temperature": 0.1,
            "max_tokens": 16384
        });
        let json = llm_api_call_with_retry(&self.config, &body, 2)?;
        extract_llm_text(&self.config, &json).ok_or_else(|| "No content in LLM response".into())
    }

    /// Simple LLM call that returns a list of candidate strings
    pub fn infer_candidates(&self, prompt: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let messages = vec![serde_json::json!({"role": "user", "content": prompt})];
        let (url, headers, body) = build_llm_request(&self.config, &messages)
            .map_err(|e| format!("LLM request build failed: {}", e))?;
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(120))
            .build();
        let mut req = agent.post(&url);
        for (k, v) in &headers {
            req = req.set(k, v);
        }
        let response = req.send_json(&body)
            .map_err(|e| format!("LLM API call failed: {}", e))?;
        let json: serde_json::Value = response.into_json()?;
        let content = extract_llm_text(&self.config, &json).ok_or("No content")?;
        Ok(content.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
    }
}

/// 按 api_type 构造 LLM 请求（url/headers/body）
/// openai    → openai_chat_url(endpoint)（CC Switch OPENAI_BASE_URL 可能不含 /v1/chat/completions）
///             headers: Authorization: Bearer {api_key}
///             body: {"model","messages","temperature":0.1}
/// anthropic → anthropic_messages_url(endpoint)（endpoint 无 /v1 时拼）
///             headers: x-api-key: {api_key}, anthropic-version: 2023-06-01
///             body: {"model","messages","max_tokens":4096}
pub fn build_llm_request(
    cfg: &LlmConfig,
    messages: &[serde_json::Value],
) -> Result<(String, Vec<(String, String)>, serde_json::Value), String> {
    if cfg.api_type == "anthropic" {
        let headers = vec![
            ("x-api-key".to_string(), cfg.api_key.clone()),
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        let body = serde_json::json!({
            "model": cfg.model,
            "messages": messages,
            "max_tokens": 4096,
        });
        Ok((anthropic_messages_url(&cfg.endpoint), headers, body))
    } else {
        // openai / 未知类型（兼容旧配置默认）
        let headers = vec![
            ("Authorization".to_string(), format!("Bearer {}", cfg.api_key)),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        let body = serde_json::json!({
            "model": cfg.model,
            "messages": messages,
            "temperature": 0.1,
        });
        Ok((openai_chat_url(&cfg.endpoint), headers, body))
    }
}

/// openai Chat Completions 端点 URL 规范化：
/// - 已含 /chat/completions → 原样
/// - 已含 /v1 → 拼 /chat/completions（CC Switch OPENAI_BASE_URL 形态，如 https://api.deepseek.com/v1）
/// - 否则 → 拼 /v1/chat/completions
fn openai_chat_url(endpoint: &str) -> String {
    let ep = endpoint.trim_end_matches('/');
    if ep.is_empty() {
        return ep.to_string();
    }
    if ep.ends_with("/chat/completions") {
        ep.to_string()
    } else if ep.ends_with("/v1") {
        format!("{}/chat/completions", ep)
    } else {
        format!("{}/v1/chat/completions", ep)
    }
}

/// anthropic Messages 端点 URL：endpoint 无 /v1 时拼 /v1/messages，已有不重复
/// 尾斜杠先去掉，避免拼出 //v1/messages
fn anthropic_messages_url(endpoint: &str) -> String {
    let ep = endpoint.trim_end_matches('/');
    if ep.ends_with("/v1/messages") {
        ep.to_string()
    } else if ep.ends_with("/v1") {
        format!("{}/messages", ep)
    } else {
        format!("{}/v1/messages", ep)
    }
}

/// 按协议从 LLM 响应中提取文本内容
/// openai: choices[0].message.content；anthropic: content[].text
pub fn extract_llm_text(cfg: &LlmConfig, json: &serde_json::Value) -> Option<String> {
    if cfg.api_type == "anthropic" {
        json.get("content")?.as_array()?
            .iter()
            .find(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
            .and_then(|c| c.get("text").and_then(|t| t.as_str()))
            .map(String::from)
    } else {
        json["choices"][0]["message"]["content"].as_str().map(String::from)
    }
}

/// Call LLM API with retry on 5xx / timeout errors.
/// `messages` 接受两种形态：直接 messages 数组，或带 "messages" 字段的旧版 body
/// （兼容 discovery.rs / infer_hierarchical.rs 等调用方），构造统一走 build_llm_request。
pub(crate) fn llm_api_call_with_retry(
    config: &LlmConfig,
    messages: &serde_json::Value,
    max_retries: u32,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    // 归一化 messages：旧 body 对象 → 取其 "messages" 字段
    let msgs: Vec<serde_json::Value> = messages
        .get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_else(|| {
            messages.as_array().cloned().unwrap_or_else(|| vec![messages.clone()])
        });
    let (url, headers, mut body) = build_llm_request(config, &msgs)
        .map_err(|e| format!("LLM request build failed: {}", e))?;

    // openai：保留透传调用方 body 的 response_format / max_tokens（旧语义）。
    // 调用方依赖强制 JSON 输出与长输出上限（discovery.rs / infer_hierarchical.rs），
    // 适配层重建 body 时不得丢弃；anthropic 协议不接收额外字段，不适用。
    if config.api_type != "anthropic" {
        if let Some(v) = messages.get("response_format") {
            body["response_format"] = v.clone();
        }
        if let Some(v) = messages.get("max_tokens") {
            body["max_tokens"] = v.clone();
        }
    }

    let mut last_err = String::new();
    for attempt in 0..max_retries {
        if attempt > 0 {
            let delay = std::time::Duration::from_secs(2u64.pow(attempt));
            std::thread::sleep(delay);
        }
        info!(
            "Calling LLM API at {} (model: {}, attempt {}/{})",
            url, config.model, attempt + 1, max_retries
        );
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(1800))
            .build();
        let mut req = agent.post(&url);
        for (k, v) in &headers {
            req = req.set(k, v);
        }
        match req.send_json(&body)
        {
            Ok(response) => {
                let status = response.status();
                // 网关错误重试；429/529 为 anthropic 限流/过载，同样重试
                if status == 504 || status == 502 || status == 503
                    || status == 429 || status == 529
                {
                    last_err = format!("status code {}", status);
                    continue; // retry on gateway errors
                }
                return response.into_json()
                    .map_err(|e| format!("Failed to parse LLM response: {}", e).into());
            }
            Err(ureq::Error::Transport(e)) => {
                last_err = format!("transport: {}", e);
                continue; // retry on connection errors
            }
            Err(e) => {
                last_err = format!("{}", e);
                continue;
            }
        }
    }
    Err(format!("LLM API call failed after {} retries: {}", max_retries, last_err).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn openai_cfg() -> LlmConfig {
        LlmConfig {
            endpoint: "https://api.example.com/v1/chat/completions".into(),
            api_key: "sk-test".into(),
            model: "gpt-4o-mini".into(),
            api_type: "openai".into(),
        }
    }

    fn anthropic_cfg() -> LlmConfig {
        LlmConfig {
            endpoint: "http://10.33.105.218:3200".into(),
            api_key: "sk-anth".into(),
            model: "claude-sonnet-4-8".into(),
            api_type: "anthropic".into(),
        }
    }

    fn msgs() -> Vec<serde_json::Value> {
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap()
    }

    /// openai：url 原样、Authorization Bearer、body model/messages/temperature
    #[test]
    fn build_request_openai() {
        let (url, headers, body) = build_llm_request(&openai_cfg(), &msgs()).unwrap();
        assert_eq!(url, "https://api.example.com/v1/chat/completions");
        assert!(headers.contains(&("Authorization".to_string(), "Bearer sk-test".to_string())));
        assert!(headers.contains(&("Content-Type".to_string(), "application/json".to_string())));
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["messages"][0]["content"], "hi");
        assert_eq!(body["temperature"], 0.1);
        assert!(body.get("max_tokens").is_none(), "openai 按规格不带 max_tokens");
    }

    /// anthropic：url 拼 /v1/messages、x-api-key + anthropic-version、body max_tokens
    #[test]
    fn build_request_anthropic() {
        let (url, headers, body) = build_llm_request(&anthropic_cfg(), &msgs()).unwrap();
        assert_eq!(url, "http://10.33.105.218:3200/v1/messages");
        assert!(headers.contains(&("x-api-key".to_string(), "sk-anth".to_string())));
        assert!(headers.contains(&("anthropic-version".to_string(), "2023-06-01".to_string())));
        assert_eq!(body["model"], "claude-sonnet-4-8");
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["messages"][0]["content"], "hi");
        assert!(body.get("temperature").is_none(), "anthropic 不带 temperature");
    }

    /// endpoint 已含 /v1 或 /v1/messages → 不重复拼
    #[test]
    fn build_request_anthropic_no_duplicate_v1() {
        let mut cfg = anthropic_cfg();
        cfg.endpoint = "https://api.anthropic.com/v1".into();
        let (url, _, _) = build_llm_request(&cfg, &msgs()).unwrap();
        assert_eq!(url, "https://api.anthropic.com/v1/messages");

        cfg.endpoint = "https://api.anthropic.com/v1/messages".into();
        let (url, _, _) = build_llm_request(&cfg, &msgs()).unwrap();
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
    }

    /// openai_chat_url 三种形态：已含 /chat/completions → 原样；含 /v1 → 拼；
    /// 否则 → 拼 /v1/chat/completions；尾斜杠不产生 //。
    #[test]
    fn openai_chat_url_three_forms() {
        // CC Switch OPENAI_BASE_URL 形态
        assert_eq!(
            openai_chat_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        // 已含完整路径 → 原样
        assert_eq!(
            openai_chat_url("https://api.example.com/v1/chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );
        // 裸主机 → 拼 /v1/chat/completions
        assert_eq!(
            openai_chat_url("https://api.example.com"),
            "https://api.example.com/v1/chat/completions"
        );
        // 尾斜杠
        assert_eq!(
            openai_chat_url("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    /// anthropic_messages_url：尾斜杠 endpoint 不拼出 //v1/messages
    #[test]
    fn anthropic_messages_url_trailing_slash() {
        assert_eq!(
            anthropic_messages_url("http://x:3200/v1/"),
            "http://x:3200/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("http://x:3200/"),
            "http://x:3200/v1/messages"
        );
        // 完整端点带尾斜杠 → 去掉尾斜杠保持原样
        assert_eq!(
            anthropic_messages_url("http://x:3200/v1/messages/"),
            "http://x:3200/v1/messages"
        );
    }

    /// extract_llm_text：openai choices[0].message.content
    #[test]
    fn extract_text_openai() {
        let json = serde_json::json!({"choices":[{"message":{"content":"abc"}}]});
        assert_eq!(extract_llm_text(&openai_cfg(), &json).as_deref(), Some("abc"));
        // 空响应 → None
        assert_eq!(extract_llm_text(&openai_cfg(), &serde_json::Value::Null), None);
    }

    /// extract_llm_text：anthropic content[].text
    #[test]
    fn extract_text_anthropic() {
        let json = serde_json::json!({"content":[{"type":"text","text":"abc"}],"stop_reason":"end_turn"});
        assert_eq!(extract_llm_text(&anthropic_cfg(), &json).as_deref(), Some("abc"));
        // 无文本块（tool_use）→ None
        let no_text = serde_json::json!({"content":[{"type":"tool_use","name":"x"}]});
        assert_eq!(extract_llm_text(&anthropic_cfg(), &no_text), None);
    }

    /// chat()：消息数组直传 + response_format 保留 + 文本原样提取（openai 协议端到端）
    #[test]
    fn chat_returns_text_with_messages_and_json_mode() {
        let resp = r#"{"choices":[{"message":{"content":"{\"keywords\":[\"水稻\"]}"}}]}"#;
        let (result, req) = with_llm_server(resp, |base| {
            let cfg = LlmConfig {
                endpoint: format!("{base}/v1/chat/completions"),
                api_key: "sk-test".into(),
                model: "gpt-4o-mini".into(),
                api_type: "openai".into(),
            };
            LlmClient::new(cfg)
                .chat(&[serde_json::json!({"role": "user", "content": "hi"})])
                .unwrap()
        });
        assert_eq!(result, "{\"keywords\":[\"水稻\"]}");
        assert!(req.contains("\"messages\":["), "req: {}", req);
        assert!(
            req.contains("\"response_format\":{\"type\":\"json_object\"}"),
            "req: {}",
            req
        );
    }

    /// 端到端：llm_api_call_with_retry 走 openai 协议（真实 HTTP 环回，旧式 body 兼容）
    #[test]
    fn retry_sends_openai_request() {
        let resp = r#"{"choices":[{"message":{"content":"a, b"}}]}"#;
        let (result, req) = with_llm_server(resp, |base| {
            let cfg = LlmConfig {
                endpoint: format!("{}/v1/chat/completions", base),
                api_key: "sk-test".into(),
                model: "gpt-4o-mini".into(),
                api_type: "openai".into(),
            };
            // 旧调用形态：带 model/messages/response_format/max_tokens 的整体 body
            llm_api_call_with_retry(&cfg, &serde_json::json!({
                "model": "gpt-4o-mini",
                "messages": [{"role": "user", "content": "hi"}],
                "response_format": {"type": "json_object"},
                "temperature": 0.1,
                "max_tokens": 8192,
            }), 1).unwrap()
        });
        assert!(req.contains("POST /v1/chat/completions HTTP/1.1"), "req: {}", req);
        let lower = req.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer sk-test"), "req: {}", req);
        assert!(lower.contains("content-type: application/json"), "req: {}", req);
        assert!(req.contains("\"model\":\"gpt-4o-mini\""), "req: {}", req);
        assert!(req.contains("\"messages\":["), "req: {}", req);
        // P1 回归：调用方 body 的 response_format / max_tokens 必须原样透传
        assert!(req.contains("\"response_format\":{\"type\":\"json_object\"}"), "req: {}", req);
        assert!(req.contains("\"max_tokens\":8192"), "req: {}", req);
        assert_eq!(result["choices"][0]["message"]["content"], "a, b");
    }

    /// 端到端：llm_api_call_with_retry 走 anthropic 协议（messages 数组直传形态）
    #[test]
    fn retry_sends_anthropic_request() {
        let resp = r#"{"content":[{"type":"text","text":"c, d"}],"role":"assistant","stop_reason":"end_turn"}"#;
        let (result, req) = with_llm_server(resp, |base| {
            let cfg = LlmConfig {
                endpoint: base, // 无 /v1
                api_key: "sk-anth".into(),
                model: "claude-sonnet-4-8".into(),
                api_type: "anthropic".into(),
            };
            llm_api_call_with_retry(&cfg, &serde_json::json!([{"role": "user", "content": "hi"}]), 1).unwrap()
        });
        assert!(req.contains("POST /v1/messages HTTP/1.1"), "req: {}", req);
        let lower = req.to_ascii_lowercase();
        assert!(lower.contains("x-api-key: sk-anth"), "req: {}", req);
        assert!(lower.contains("anthropic-version: 2023-06-01"), "req: {}", req);
        assert!(req.contains("\"max_tokens\":4096"), "req: {}", req);
        assert!(req.contains("\"model\":\"claude-sonnet-4-8\""), "req: {}", req);
        assert_eq!(result["content"][0]["text"], "c, d");
    }

    // ---- 测试辅助：一次性 HTTP 服务器，返回 (闭包结果, 捕获的原始请求) ----
    fn with_llm_server<F, T>(resp_body: &str, f: F) -> (T, String)
    where
        F: FnOnce(String) -> T,
    {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{}", addr);
        let body_owned = resp_body.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut total = Vec::new();
            let mut tmp = [0u8; 8192];
            let mut clen: usize = 0;
            loop {
                let n = stream.read(&mut tmp).unwrap();
                if n == 0 { break; }
                total.extend_from_slice(&tmp[..n]);
                if let Some(pos) = total.windows(4).position(|w| w == b"\r\n\r\n") {
                    if clen == 0 {
                        let head = String::from_utf8_lossy(&total[..pos]).to_string();
                        clen = head.lines()
                            .find_map(|l| {
                                let lower = l.to_ascii_lowercase();
                                lower.strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                    }
                    if clen > 0 && total.len() >= pos + 4 + clen { break; }
                }
            }
            let req = String::from_utf8_lossy(&total).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body_owned.len(), body_owned
            );
            stream.write_all(response.as_bytes()).unwrap();
            req
        });
        let result = f(base);
        let captured = handle.join().unwrap();
        (result, captured)
    }
}
