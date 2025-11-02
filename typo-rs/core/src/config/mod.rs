//! Configuration management

mod settings;

pub use settings::{McpConfig, McpServerConfig, PermissionsConfig, Settings};

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// Get the default settings directory (.typo)
pub fn get_settings_dir() -> Result<PathBuf> {
    let current_dir = std::env::current_dir()
        .map_err(|e| Error::Config(format!("Failed to get current directory: {}", e)))?;
    Ok(current_dir.join(".typo"))
}

/// Get the default MCP config path (mcp.json)
pub fn get_mcp_config_path() -> Result<PathBuf> {
    let current_dir = std::env::current_dir()
        .map_err(|e| Error::Config(format!("Failed to get current directory: {}", e)))?;
    Ok(current_dir.join("mcp.json"))
}

/// Get the settings file path (.typo/settings.json)
pub fn get_settings_path() -> Result<PathBuf> {
    Ok(get_settings_dir()?.join("settings.json"))
}

/// Load MCP configuration from file
pub async fn load_mcp_config<P: AsRef<Path>>(path: P) -> Result<McpConfig> {
    let content = tokio::fs::read_to_string(path.as_ref()).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            tracing::warn!("mcp.json not found, using empty configuration");
            return Error::Config("mcp.json not found".to_string());
        }
        Error::Io(e)
    })?;

    serde_json::from_str(&content).map_err(|e| {
        Error::Config(format!("Failed to parse mcp.json: {}", e))
    })
}

/// Load permissions settings from file
pub async fn load_settings<P: AsRef<Path>>(path: P) -> Result<Settings> {
    match tokio::fs::read_to_string(path.as_ref()).await {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| Error::Config(format!("Failed to parse settings.json: {}", e))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Return default settings if file doesn't exist
            Ok(Settings::default())
        }
        Err(e) => Err(Error::Io(e)),
    }
}

/// Save settings to file
pub async fn save_settings<P: AsRef<Path>>(path: P, settings: &Settings) -> Result<()> {
    // Ensure directory exists
    if let Some(parent) = path.as_ref().parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let content = serde_json::to_string_pretty(settings)?;
    tokio::fs::write(path.as_ref(), content).await?;
    Ok(())
}
