use super::{TokenCounter, ToolDef};
use async_trait::async_trait;
use tiktoken_rs::CoreBPE;

const DEFAULT_MODEL: &str = "gpt-4o";

pub struct TiktokenCounter {
    model: String,
    bpe: CoreBPE,
}

impl TiktokenCounter {
    pub fn new(model: Option<String>) -> anyhow::Result<Self> {
        let model = model.unwrap_or_else(|| DEFAULT_MODEL.to_string());

        // Get the appropriate encoding for the model
        let bpe = tiktoken_rs::get_bpe_from_model(&model).unwrap_or_else(|_| {
            // Fall back to cl100k_base for unknown models
            tiktoken_rs::cl100k_base().expect("Failed to load cl100k_base encoding")
        });

        Ok(Self { model, bpe })
    }
}

#[async_trait]
impl TokenCounter for TiktokenCounter {
    async fn count_tools(&self, tools: &[ToolDef]) -> anyhow::Result<i32> {
        if tools.is_empty() {
            return Ok(0);
        }

        // Serialize tools to JSON and count tokens
        // This is an approximation - actual token counts vary by provider
        let tools_json = serde_json::to_string(tools)?;
        let tokens = self.bpe.encode_with_special_tokens(&tools_json);

        Ok(tokens.len() as i32)
    }

    async fn count_text(&self, text: &str) -> anyhow::Result<i32> {
        if text.is_empty() {
            return Ok(0);
        }

        let tokens = self.bpe.encode_with_special_tokens(text);
        Ok(tokens.len() as i32)
    }

    fn name(&self) -> &str {
        "tiktoken"
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_count_text_empty() {
        let counter = TiktokenCounter::new(None).unwrap();
        let count = counter.count_text("").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_count_text_simple() {
        let counter = TiktokenCounter::new(None).unwrap();
        let count = counter.count_text("hello world").await.unwrap();
        assert!(count > 0);
        assert!(count < 10); // "hello world" should be just a few tokens
    }

    #[tokio::test]
    async fn test_count_tools_empty() {
        let counter = TiktokenCounter::new(None).unwrap();
        let count = counter.count_tools(&[]).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_count_tools_simple() {
        let counter = TiktokenCounter::new(None).unwrap();
        let tools = vec![ToolDef {
            name: "test_tool".to_string(),
            description: Some("A test tool".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "arg1": {"type": "string"}
                }
            }),
        }];
        let count = counter.count_tools(&tools).await.unwrap();
        assert!(count > 0);
    }

    #[test]
    fn test_name_and_model() {
        let counter = TiktokenCounter::new(None).unwrap();
        assert_eq!(counter.name(), "tiktoken");
        assert_eq!(counter.model(), "gpt-4o");

        let counter2 = TiktokenCounter::new(Some("gpt-4".to_string())).unwrap();
        assert_eq!(counter2.model(), "gpt-4");
    }
}
