use crate::counter::TokenCounter;
use crate::mcp::ServerData;
use serde::{Deserialize, Serialize};

/// Token count for a single item (tool, resource, prompt).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemTokens {
    pub name: String,
    pub tokens: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_tokens: Option<i32>,
}

/// Token counts for a category (tools, resources, prompts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryTokens {
    pub total: i32,
    pub count: usize,
    pub items: Vec<ItemTokens>,
}

/// Complete analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub counter: CounterInfo,
    pub server_info: ServerInfoReport,
    pub total_tokens: i32,
    pub tools: CategoryTokens,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<CategoryTokens>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<CategoryTokens>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterInfo {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfoReport {
    pub name: String,
    pub version: String,
}

/// Analyze MCP server data and count tokens.
pub struct Analyzer<'a> {
    counter: &'a dyn TokenCounter,
}

impl<'a> Analyzer<'a> {
    pub fn new(counter: &'a dyn TokenCounter) -> Self {
        Self { counter }
    }

    pub async fn analyze(&self, data: &ServerData) -> anyhow::Result<AnalysisReport> {
        // Count tokens for all tools combined (this is how they appear in context)
        let tools_total = self.counter.count_tools(&data.tools).await?;

        // Also count individual tools for breakdown
        let mut tool_items = Vec::new();
        for tool in &data.tools {
            let single_tool = vec![tool.clone()];
            let tool_tokens = self.counter.count_tools(&single_tool).await?;

            // Count description separately if available
            let desc_tokens = if let Some(ref desc) = tool.description {
                Some(self.counter.count_text(desc).await?)
            } else {
                None
            };

            // Schema tokens = total - description (approximate)
            let schema_tokens = desc_tokens.map(|d| tool_tokens - d);

            tool_items.push(ItemTokens {
                name: tool.name.clone(),
                tokens: tool_tokens,
                description_tokens: desc_tokens,
                schema_tokens,
            });
        }

        // Sort by token count descending
        tool_items.sort_by(|a, b| b.tokens.cmp(&a.tokens));

        let tools = CategoryTokens {
            total: tools_total,
            count: data.tools.len(),
            items: tool_items,
        };

        // Count resources if present
        let resources = if !data.resources.is_empty() {
            let resource_items: Vec<ItemTokens> =
                futures::future::try_join_all(data.resources.iter().map(|r| async {
                    let text = format!(
                        "{}{}{}",
                        r.name,
                        r.description.as_deref().unwrap_or(""),
                        r.uri
                    );
                    let tokens = self.counter.count_text(&text).await?;
                    Ok::<_, anyhow::Error>(ItemTokens {
                        name: r.name.clone(),
                        tokens,
                        description_tokens: None,
                        schema_tokens: None,
                    })
                }))
                .await?;

            let total: i32 = resource_items.iter().map(|i| i.tokens).sum();
            Some(CategoryTokens {
                total,
                count: data.resources.len(),
                items: resource_items,
            })
        } else {
            None
        };

        // Count prompts if present
        let prompts = if !data.prompts.is_empty() {
            let prompt_items: Vec<ItemTokens> =
                futures::future::try_join_all(data.prompts.iter().map(|p| async {
                    let text = format!("{}{}", p.name, p.description.as_deref().unwrap_or(""));
                    let tokens = self.counter.count_text(&text).await?;
                    Ok::<_, anyhow::Error>(ItemTokens {
                        name: p.name.clone(),
                        tokens,
                        description_tokens: None,
                        schema_tokens: None,
                    })
                }))
                .await?;

            let total: i32 = prompt_items.iter().map(|i| i.tokens).sum();
            Some(CategoryTokens {
                total,
                count: data.prompts.len(),
                items: prompt_items,
            })
        } else {
            None
        };

        let total_tokens = tools.total
            + resources.as_ref().map(|r| r.total).unwrap_or(0)
            + prompts.as_ref().map(|p| p.total).unwrap_or(0);

        Ok(AnalysisReport {
            counter: CounterInfo {
                provider: self.counter.name().to_string(),
                model: self.counter.model().to_string(),
            },
            server_info: ServerInfoReport {
                name: data.server_info.name.clone(),
                version: data.server_info.version.clone(),
            },
            total_tokens,
            tools,
            resources,
            prompts,
        })
    }
}
