use super::{TokenCounter, ToolDef};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages/count_tokens";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-sonnet-4-5-20250929";

pub struct AnthropicCounter {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl AnthropicCounter {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct CountTokensRequest {
    model: String,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: serde_json::Value,
}

#[derive(Deserialize)]
struct CountTokensResponse {
    input_tokens: i32,
}

impl AnthropicCounter {
    async fn do_count_request(&self, request: CountTokensRequest) -> anyhow::Result<i32> {
        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("Content-Type", "application/json")
            .header("X-Api-Key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error (status {}): {}", status, body);
        }

        let result: CountTokensResponse = response.json().await?;
        Ok(result.input_tokens)
    }
}

#[async_trait]
impl TokenCounter for AnthropicCounter {
    async fn count_tools(&self, tools: &[ToolDef]) -> anyhow::Result<i32> {
        if tools.is_empty() {
            return Ok(0);
        }

        // Convert to Anthropic format
        let anthropic_tools: Vec<AnthropicTool> = tools
            .iter()
            .map(|t| {
                let mut input_schema = t.input_schema.clone();
                // Ensure we have a valid object schema
                if input_schema.is_null() {
                    input_schema = serde_json::json!({
                        "type": "object",
                        "properties": {}
                    });
                }
                AnthropicTool {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema,
                }
            })
            .collect();

        // Count tokens with tools
        let request_with_tools = CountTokensRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            tools: anthropic_tools,
        };
        let total_tokens = self.do_count_request(request_with_tools).await?;

        // Subtract baseline (just the message without tools)
        let baseline_request = CountTokensRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            tools: vec![],
        };
        let baseline_tokens = self.do_count_request(baseline_request).await?;

        Ok(total_tokens - baseline_tokens)
    }

    async fn count_text(&self, text: &str) -> anyhow::Result<i32> {
        if text.is_empty() {
            return Ok(0);
        }

        let request = CountTokensRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: text.to_string(),
            }],
            tools: vec![],
        };

        self.do_count_request(request).await
    }

    fn name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
    }
}
