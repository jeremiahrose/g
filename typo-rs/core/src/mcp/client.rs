//! MCP client implementation using official rmcp SDK

use super::types::*;
use crate::config::{get_mcp_config_path, load_mcp_config, McpConfig, McpServerConfig};
use crate::error::{Error, Result};
use rmcp::model::{CallToolRequestParam, ClientCapabilities, ClientInfo, Implementation, RawContent};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

type RmcpClient = RunningService<RoleClient, ClientInfo>;

/// MCP client for managing connections to MCP servers
pub struct McpClient {
    clients: Arc<RwLock<HashMap<String, Arc<RmcpClient>>>>,
    tools: Arc<RwLock<Vec<Tool>>>,
}

impl McpClient {
    /// Create a new MCP client
    pub async fn new() -> Result<Self> {
        Ok(Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            tools: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Connect to all MCP servers defined in mcp.json
    pub async fn connect_to_servers(&self) -> Result<()> {
        let config_path = get_mcp_config_path()?;
        let config = match load_mcp_config(&config_path).await {
            Ok(cfg) => cfg,
            Err(_) => {
                tracing::warn!("No mcp.json found, using empty configuration");
                McpConfig::default()
            }
        };

        for (name, server_config) in config.mcp_servers {
            tracing::info!("Connecting to MCP server: {}", name);

            match self.connect_server(&name, &server_config).await {
                Ok(()) => tracing::info!("Successfully connected to MCP server: {}", name),
                Err(e) => tracing::error!("Failed to connect to MCP server {}: {}", name, e),
            }
        }

        // List tools from all connected servers
        self.refresh_tools().await?;

        Ok(())
    }

    /// Connect to a single MCP server
    async fn connect_server(&self, name: &str, config: &McpServerConfig) -> Result<()> {
        let client = if let Some(url) = &config.url {
            // HTTP transport
            tracing::debug!("Connecting to {} via HTTP: {}", name, url);

            let transport = StreamableHttpClientTransport::from_uri(url.as_str());

            let client_info = ClientInfo {
                protocol_version: Default::default(),
                capabilities: ClientCapabilities::default(),
                client_info: Implementation {
                    name: "typo-cli".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    title: Some("Typo Voice AI Assistant".to_string()),
                    website_url: None,
                    icons: None,
                },
            };

            let client = client_info
                .serve(transport)
                .await
                .map_err(|e| Error::Mcp(format!("Failed to connect via HTTP: {}", e)))?;

            tracing::info!("Connected to server");

            Arc::new(client)
        } else if !config.command.is_empty() {
            // Subprocess transport
            return Err(Error::Mcp(
                "Subprocess transport not yet implemented. Please use HTTP for now.".to_string(),
            ));
        } else {
            return Err(Error::Mcp(
                "Server config missing both url and command".to_string(),
            ));
        };

        // Store the client
        let mut clients = self.clients.write().await;
        clients.insert(name.to_string(), client);

        Ok(())
    }

    /// Refresh the list of available tools from all servers
    async fn refresh_tools(&self) -> Result<()> {
        let clients = self.clients.read().await;
        let mut all_tools = Vec::new();

        for (server_name, client) in clients.iter() {
            match self.list_tools_from_server(server_name, client).await {
                Ok(tools) => {
                    tracing::info!("Got {} tools from server '{}'", tools.len(), server_name);
                    all_tools.extend(tools);
                }
                Err(e) => {
                    tracing::error!("Failed to list tools from server '{}': {}", server_name, e);
                }
            }
        }

        let mut tools = self.tools.write().await;
        *tools = all_tools;

        Ok(())
    }

    /// List tools from a specific server
    async fn list_tools_from_server(
        &self,
        _server_name: &str,
        client: &Arc<RmcpClient>,
    ) -> Result<Vec<Tool>> {
        let tools_result = client
            .list_tools(Default::default())
            .await
            .map_err(|e| Error::Mcp(format!("Failed to list tools: {}", e)))?;

        // Convert rmcp tools to our Tool type
        let tools: Vec<Tool> = tools_result
            .tools
            .into_iter()
            .map(|t| Tool {
                name: t.name.to_string(),
                description: t.description.map(|s| s.to_string()),
                input_schema: Some(serde_json::Value::Object((*t.input_schema).clone())),
            })
            .collect();

        Ok(tools)
    }

    /// Get list of all available tools
    pub async fn list_tools(&self) -> Vec<Tool> {
        let tools = self.tools.read().await;
        tools.clone()
    }

    /// Call a tool on the appropriate MCP server
    pub async fn call_tool(&self, tool_name: &str, arguments: &Value) -> Result<ToolCallResult> {
        let clients = self.clients.read().await;

        // Try each server until one succeeds
        for (server_name, client) in clients.iter() {
            tracing::debug!("Trying tool '{}' on server '{}'", tool_name, server_name);

            let result = client
                .call_tool(CallToolRequestParam {
                    name: tool_name.to_string().into(),
                    arguments: arguments.as_object().cloned(),
                })
                .await;

            match result {
                Ok(tool_result) => {
                    // Convert rmcp result to our ToolCallResult
                    let content = tool_result
                        .content
                        .into_iter()
                        .filter_map(|c| match c.raw {
                            RawContent::Text(text_content) => {
                                Some(ContentItem::text(text_content.text))
                            }
                            _ => None,
                        })
                        .collect();

                    return Ok(ToolCallResult {
                        success: !tool_result.is_error.unwrap_or(false),
                        content: Some(content),
                        error: None,
                        is_error: tool_result.is_error.unwrap_or(false),
                    });
                }
                Err(e) => {
                    tracing::debug!("Server '{}' failed: {}", server_name, e);
                    continue;
                }
            }
        }

        Err(Error::ToolExecution(format!(
            "No server could execute tool '{}'",
            tool_name
        )))
    }

    /// Close all server connections
    pub async fn close(&self) -> Result<()> {
        // Clear all clients - they will be dropped and cancelled automatically
        let mut clients = self.clients.write().await;
        clients.clear();
        Ok(())
    }
}
