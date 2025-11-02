//! Settings and configuration types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

/// Root MCP configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

/// Permissions configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsConfig {
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Main settings structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub permissions: PermissionsConfig,
}

impl Settings {
    pub fn new() -> Self {
        Self::default()
    }
}
