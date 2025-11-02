//! MCP client implementation

use super::types::*;
use crate::config::{get_mcp_config_path, load_mcp_config, McpConfig};
use crate::error::{Error, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex, RwLock};

/// MCP client for managing connections to MCP servers
pub struct McpClient {
    servers: Arc<RwLock<HashMap<String, ServerConnection>>>,
    tools: Arc<RwLock<Vec<Tool>>>,
    next_id: Arc<AtomicU64>,
}

struct ServerConnection {
    _child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    #[allow(dead_code)]
    response_rx: mpsc::UnboundedReceiver<JsonRpcResponse>,
}

impl McpClient {
    /// Create a new MCP client
    pub async fn new() -> Result<Self> {
        Ok(Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            tools: Arc::new(RwLock::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(1)),
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

            match self.connect_server(&name, &server_config.command, server_config.args.as_deref()).await {
                Ok(()) => tracing::info!("Successfully connected to MCP server: {}", name),
                Err(e) => tracing::error!("Failed to connect to MCP server {}: {}", name, e),
            }
        }

        // List tools from all connected servers
        self.refresh_tools().await?;

        Ok(())
    }

    /// Connect to a single MCP server
    async fn connect_server(&self, name: &str, command: &str, args: Option<&[String]>) -> Result<()> {
        // Spawn the MCP server process
        let mut cmd = Command::new(command);
        if let Some(args) = args {
            cmd.args(args);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            Error::Mcp(format!("Failed to spawn MCP server '{}': {}", name, e))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            Error::Mcp("Failed to get stdin handle".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            Error::Mcp("Failed to get stdout handle".to_string())
        })?;

        // Create channel for responses
        let (response_tx, response_rx) = mpsc::unbounded_channel();

        // Spawn task to read responses
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&line) {
                    if response_tx.send(response).is_err() {
                        break;
                    }
                }
            }
        });

        let connection = ServerConnection {
            _child: child,
            stdin: Arc::new(Mutex::new(stdin)),
            response_rx,
        };

        // Initialize the server
        self.initialize_server(&connection).await?;

        // Store the connection
        let mut servers = self.servers.write().await;
        servers.insert(name.to_string(), connection);

        Ok(())
    }

    /// Initialize an MCP server connection
    async fn initialize_server(&self, connection: &ServerConnection) -> Result<()> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: self.next_id.fetch_add(1, Ordering::SeqCst),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "roots": {
                        "listChanged": true
                    }
                },
                "clientInfo": {
                    "name": "typo",
                    "version": "0.1.0"
                }
            })),
        };

        self.send_request(connection, &request).await?;
        Ok(())
    }

    /// Send a JSON-RPC request to a server
    async fn send_request(&self, connection: &ServerConnection, request: &JsonRpcRequest) -> Result<Value> {
        let mut stdin = connection.stdin.lock().await;
        let json = serde_json::to_string(request)?;
        stdin.write_all(json.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;

        // TODO: Properly wait for and match response by ID
        // For now, this is simplified
        Ok(Value::Null)
    }

    /// Refresh the list of available tools from all servers
    async fn refresh_tools(&self) -> Result<()> {
        let servers = self.servers.read().await;
        let mut all_tools = Vec::new();

        for (server_name, connection) in servers.iter() {
            match self.list_tools_from_server(connection).await {
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
    async fn list_tools_from_server(&self, connection: &ServerConnection) -> Result<Vec<Tool>> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: self.next_id.fetch_add(1, Ordering::SeqCst),
            method: "tools/list".to_string(),
            params: None,
        };

        // Send request
        let result = self.send_request(connection, &request).await?;

        // Parse tools from result
        if let Some(tools_array) = result.get("tools").and_then(|v| v.as_array()) {
            let tools: Vec<Tool> = tools_array.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
            Ok(tools)
        } else {
            Ok(Vec::new())
        }
    }

    /// Get list of all available tools
    pub async fn list_tools(&self) -> Vec<Tool> {
        let tools = self.tools.read().await;
        tools.clone()
    }

    /// Call a tool on the appropriate MCP server
    pub async fn call_tool(&self, tool_name: &str, arguments: &Value) -> Result<ToolCallResult> {
        let servers = self.servers.read().await;

        // For simplicity, try each server until one succeeds
        // In a real implementation, we'd track which server provides which tool
        for (server_name, connection) in servers.iter() {
            let request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: self.next_id.fetch_add(1, Ordering::SeqCst),
                method: "tools/call".to_string(),
                params: Some(serde_json::json!({
                    "name": tool_name,
                    "arguments": arguments,
                })),
            };

            match self.send_request(connection, &request).await {
                Ok(result) => {
                    // Parse the result into ToolCallResult
                    if let Ok(tool_result) = serde_json::from_value::<ToolCallResult>(result.clone()) {
                        return Ok(tool_result);
                    }

                    // Fallback: create a success result from raw value
                    return Ok(ToolCallResult {
                        success: true,
                        content: Some(vec![ContentItem::text(result.to_string())]),
                        error: None,
                        is_error: false,
                    });
                }
                Err(e) => {
                    tracing::debug!("Server '{}' failed to execute tool: {}", server_name, e);
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
        let mut servers = self.servers.write().await;
        servers.clear();
        Ok(())
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Best effort cleanup
        // The child processes will be terminated when dropped
    }
}
