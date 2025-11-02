//! Tool permissions management

use crate::config::{get_settings_path, load_settings, save_settings, PermissionsConfig, Settings};
use crate::error::{Error, Result};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Permission manager for controlling tool access
#[derive(Clone)]
pub struct PermissionManager {
    settings: Arc<RwLock<Settings>>,
    settings_path: std::path::PathBuf,
}

impl PermissionManager {
    /// Create a new permission manager
    pub async fn new() -> Result<Self> {
        let settings_path = get_settings_path()?;
        let settings = load_settings(&settings_path).await.unwrap_or_default();

        tracing::info!(
            "Loaded permissions: {} allowed, {} denied",
            settings.permissions.allowed_tools.len(),
            settings.permissions.deny.len()
        );

        Ok(Self {
            settings: Arc::new(RwLock::new(settings)),
            settings_path,
        })
    }

    /// Check if a tool is explicitly denied
    pub async fn is_denied(&self, tool_name: &str, _args: Option<&Value>) -> bool {
        let settings = self.settings.read().await;
        let tool_call = self.format_tool_call(tool_name, _args);

        for pattern in &settings.permissions.deny {
            if self.matches_pattern(&tool_call, pattern) {
                tracing::debug!("Tool '{}' denied by pattern '{}'", tool_call, pattern);
                return true;
            }
        }

        false
    }

    /// Check if a tool is pre-approved
    pub async fn is_allowed(&self, tool_name: &str, _args: Option<&Value>) -> bool {
        let settings = self.settings.read().await;
        let tool_call = self.format_tool_call(tool_name, _args);

        for pattern in &settings.permissions.allowed_tools {
            if self.matches_pattern(&tool_call, pattern) {
                tracing::debug!("Tool '{}' allowed by pattern '{}'", tool_call, pattern);
                return true;
            }
        }

        false
    }

    /// Add a tool to the allowed list
    pub async fn add_allowed(&self, tool_name: &str, _args: Option<&Value>) -> Result<()> {
        let tool_pattern = tool_name.to_string();

        let mut settings = self.settings.write().await;
        if !settings.permissions.allowed_tools.contains(&tool_pattern) {
            settings.permissions.allowed_tools.push(tool_pattern.clone());
            drop(settings); // Release write lock before saving

            self.save().await?;
            tracing::info!("Added '{}' to allowed tools", tool_pattern);
        }

        Ok(())
    }

    /// Add a tool to the denied list
    pub async fn add_denied(&self, tool_name: &str, _args: Option<&Value>) -> Result<()> {
        let tool_pattern = tool_name.to_string();

        let mut settings = self.settings.write().await;
        if !settings.permissions.deny.contains(&tool_pattern) {
            settings.permissions.deny.push(tool_pattern.clone());
            drop(settings); // Release write lock before saving

            self.save().await?;
            tracing::info!("Added '{}' to denied tools", tool_pattern);
        }

        Ok(())
    }

    /// Save current settings to disk
    async fn save(&self) -> Result<()> {
        let settings = self.settings.read().await;
        save_settings(&self.settings_path, &settings).await?;
        tracing::debug!("Permissions saved");
        Ok(())
    }

    /// Check if a tool call matches a permission pattern
    fn matches_pattern(&self, tool_call: &str, pattern: &str) -> bool {
        // If pattern has no parentheses, match just the tool name
        if !pattern.contains('(') {
            let tool_name = if tool_call.contains('(') {
                tool_call.split('(').next().unwrap_or(tool_call)
            } else {
                tool_call
            };
            return fnmatch::fnmatch(pattern, tool_name);
        }

        // Pattern has arguments - use full fnmatch
        fnmatch::fnmatch(pattern, tool_call)
    }

    /// Format tool call for pattern matching
    fn format_tool_call(&self, tool_name: &str, _args: Option<&Value>) -> String {
        // For now, just use tool name
        // Could be extended to include argument matching later
        tool_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pattern_matching() {
        let manager = PermissionManager::new().await.unwrap();

        // Simple tool name matching
        assert!(manager.matches_pattern("my_tool", "my_tool"));
        assert!(!manager.matches_pattern("my_tool", "other_tool"));

        // Wildcard matching
        assert!(manager.matches_pattern("my_tool", "my_*"));
        assert!(manager.matches_pattern("my_tool", "*_tool"));
        assert!(manager.matches_pattern("my_tool", "*"));
    }
}
