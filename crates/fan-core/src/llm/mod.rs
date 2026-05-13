pub mod prompt;

use crate::config::LlmConfig;
use prompt::{LlmOutput, system_prompt};
use tracing::info;

pub struct LlmClient {
    config: LlmConfig,
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
            "temperature": 0.1
        });

        info!(
            "Calling LLM API at {} (model: {})",
            self.config.endpoint, self.config.model
        );
        let response = ureq::post(&self.config.endpoint)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| format!("LLM API call failed: {}", e))?;

        let json: serde_json::Value = response
            .into_json()
            .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("No content in LLM response")?;

        prompt::parse_llm_response(content)
            .map_err(|e| format!("Failed to parse LLM JSON output: {}", e).into())
    }
}
