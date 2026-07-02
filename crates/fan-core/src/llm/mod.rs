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
            "temperature": 0.1,
            "max_tokens": 16384
        });

        let json: serde_json::Value = llm_api_call_with_retry(&self.config, &body, 3)?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("No content in LLM response")?;

        info!("LLM raw response: {}", content);
        prompt::parse_llm_response(content)
            .map_err(|e| format!("Failed to parse LLM JSON output: {}", e).into())
    }

    /// Simple LLM call that returns a list of candidate strings
    pub fn infer_candidates(&self, prompt: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.3,
            "max_tokens": 50
        });
        let response = ureq::post(&self.config.endpoint)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| format!("LLM API call failed: {}", e))?;
        let json: serde_json::Value = response.into_json()?;
        let content = json["choices"][0]["message"]["content"]
            .as_str().ok_or("No content")?;
        Ok(content.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
    }
}

/// Call LLM API with retry on 5xx / timeout errors.
fn llm_api_call_with_retry(
    config: &LlmConfig,
    body: &serde_json::Value,
    max_retries: u32,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut last_err = String::new();
    for attempt in 0..max_retries {
        if attempt > 0 {
            let delay = std::time::Duration::from_secs(2u64.pow(attempt));
            std::thread::sleep(delay);
        }
        info!(
            "Calling LLM API at {} (model: {}, attempt {}/{})",
            config.endpoint, config.model, attempt + 1, max_retries
        );
        match ureq::post(&config.endpoint)
            .set("Authorization", &format!("Bearer {}", config.api_key))
            .set("Content-Type", "application/json")
            .send_json(body)
        {
            Ok(response) => {
                let status = response.status();
                if status == 504 || status == 502 || status == 503 {
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
