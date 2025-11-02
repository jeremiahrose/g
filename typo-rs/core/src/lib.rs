//! Typo Core Library
//!
//! Core functionality for the Typo voice AI assistant, including:
//! - MCP (Model Context Protocol) client
//! - OpenAI Realtime API integration
//! - Tool permissions management
//! - Configuration handling

pub mod config;
pub mod error;
pub mod mcp;
pub mod openai;
pub mod permissions;

pub use error::{Error, Result};
