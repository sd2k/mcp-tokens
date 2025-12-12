use crate::counter::ToolDef;
use anyhow::Result;
use rmcp::{
    ServiceExt,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::process::Command;

/// Metadata about the MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// Resource from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Prompt from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<PromptArgument>,
}

/// Argument for an MCP prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// All data retrieved from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerData {
    pub server_info: ServerInfo,
    pub tools: Vec<ToolDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<Resource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<Prompt>,
}

/// MCP client wrapper for token analysis.
pub struct Client {
    timeout: Duration,
}

impl Client {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Connect to an MCP server and fetch all its metadata.
    pub async fn fetch_server_data(&self, command: &str, args: &[String]) -> Result<ServerData> {
        // Build the command
        let args_clone = args.to_vec();
        let transport = TokioChildProcess::new(Command::new(command).configure(move |cmd| {
            for arg in &args_clone {
                cmd.arg(arg);
            }
        }))?;

        // Connect with timeout
        let client = tokio::time::timeout(self.timeout, ().serve(transport)).await??;

        // Get server info
        let server_info = if let Some(info) = client.peer_info() {
            ServerInfo {
                name: info.server_info.name.clone(),
                version: info.server_info.version.clone(),
            }
        } else {
            ServerInfo {
                name: "unknown".to_string(),
                version: "unknown".to_string(),
            }
        };

        // Fetch tools
        let tools_result = client.list_tools(Default::default()).await;
        let tools: Vec<ToolDef> = match tools_result {
            Ok(result) => result
                .tools
                .into_iter()
                .map(|t| {
                    // Convert Arc<Map> to Value by serializing
                    let schema_value = serde_json::to_value(&*t.input_schema)
                        .unwrap_or(serde_json::json!({"type": "object", "properties": {}}));
                    ToolDef {
                        name: t.name.to_string(),
                        description: t.description.map(|d| d.to_string()),
                        input_schema: schema_value,
                    }
                })
                .collect(),
            Err(e) => {
                eprintln!("Warning: failed to list tools: {}", e);
                vec![]
            }
        };

        // Fetch resources
        let resources_result = client.list_resources(Default::default()).await;
        let resources: Vec<Resource> = match resources_result {
            Ok(result) => result
                .resources
                .into_iter()
                .map(|r| Resource {
                    uri: r.uri.to_string(),
                    name: r.name.to_string(),
                    description: r.description.as_ref().map(|d| d.to_string()),
                    mime_type: r.mime_type.as_ref().map(|m| m.to_string()),
                })
                .collect(),
            Err(_) => vec![],
        };

        // Fetch prompts
        let prompts_result = client.list_prompts(Default::default()).await;
        let prompts: Vec<Prompt> = match prompts_result {
            Ok(result) => result
                .prompts
                .into_iter()
                .map(|p| Prompt {
                    name: p.name.to_string(),
                    description: p.description.map(|d| d.to_string()),
                    arguments: p
                        .arguments
                        .unwrap_or_default()
                        .into_iter()
                        .map(|a| PromptArgument {
                            name: a.name.to_string(),
                            description: a.description.map(|d| d.to_string()),
                            required: a.required.unwrap_or(false),
                        })
                        .collect(),
                })
                .collect(),
            Err(_) => vec![],
        };

        Ok(ServerData {
            server_info,
            tools,
            resources,
            prompts,
        })
    }
}
