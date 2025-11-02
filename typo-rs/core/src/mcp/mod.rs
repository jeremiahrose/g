//! MCP (Model Context Protocol) client implementation

mod client;
mod types;

pub use client::McpClient;
pub use types::{Tool, ToolCallResult, ToolInput};
