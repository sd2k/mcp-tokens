mod anthropic;
mod tiktoken;

pub use anthropic::AnthropicCounter;
pub use tiktoken::TiktokenCounter;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Tool definition matching MCP's tool structure for token counting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: serde_json::Value,
}

/// Token counter trait for different providers.
#[async_trait]
pub trait TokenCounter: Send + Sync {
    /// Count tokens for a list of tools as they would appear in the model's context.
    async fn count_tools(&self, tools: &[ToolDef]) -> anyhow::Result<i32>;

    /// Count tokens for raw text.
    async fn count_text(&self, text: &str) -> anyhow::Result<i32>;

    /// Provider name for reporting.
    fn name(&self) -> &str;

    /// Model used for counting.
    fn model(&self) -> &str;
}

/// Configuration for creating a token counter.
#[derive(Debug, Clone, Default)]
pub struct CounterConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub anthropic_key: Option<String>,
}

/// Normalize an optional API key, treating empty strings as None.
fn normalize_api_key(key: Option<String>) -> Option<String> {
    key.filter(|k| !k.is_empty())
}

/// Create a token counter based on configuration.
pub fn create_counter(config: CounterConfig) -> anyhow::Result<Box<dyn TokenCounter>> {
    let anthropic_key = normalize_api_key(config.anthropic_key);

    // Explicit provider selection
    match config.provider.as_deref() {
        Some("anthropic") => {
            let key = anthropic_key.ok_or_else(|| anyhow::anyhow!("Anthropic API key required"))?;
            return Ok(Box::new(AnthropicCounter::new(key, config.model)));
        }
        Some("tiktoken") => {
            return Ok(Box::new(TiktokenCounter::new(config.model)?));
        }
        Some(p) => {
            anyhow::bail!("Unknown provider: {}", p);
        }
        None => {}
    }

    // Auto-detect from available keys
    if let Some(key) = anthropic_key {
        return Ok(Box::new(AnthropicCounter::new(key, config.model)));
    }

    // Fallback to tiktoken
    Ok(Box::new(TiktokenCounter::new(config.model)?))
}
