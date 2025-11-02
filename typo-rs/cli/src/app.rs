//! Main application logic

use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use typo_core::{
    mcp::McpClient,
    openai::{OutputItem, RealtimeClient, RealtimeEvent, SessionConfig, Tool, TurnDetectionConfig},
    permissions::PermissionManager,
};

use crate::audio::{AudioPlayer, AudioRecorder};
#[cfg(target_os = "macos")]
use crate::keyboard::KeyboardListener;

/// Approval decision from user
#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    ApproveOnce,
    RejectOnce,
    AlwaysAllow,
    NeverAllow,
}

/// Main application
pub struct App {
    openai_client: Arc<RwLock<Option<RealtimeClient>>>,
    mcp_client: Arc<McpClient>,
    permissions: Arc<PermissionManager>,
    audio_player: Arc<AudioPlayer>,
    audio_recorder: Arc<AudioRecorder>,
    pending_approval: Arc<RwLock<Option<(String, Value, mpsc::Sender<ApprovalDecision>)>>>,
}

impl App {
    async fn new(api_key: String, model: String) -> Result<Self> {
        // Initialize MCP client
        let mcp_client = Arc::new(McpClient::new().await?);
        mcp_client.connect_to_servers().await?;

        let tools = mcp_client.list_tools().await;
        tracing::info!("Loaded {} MCP tools", tools.len());

        // Initialize permissions
        let permissions = Arc::new(PermissionManager::new().await?);

        // Initialize audio
        let audio_player = Arc::new(AudioPlayer::new()?);
        let audio_recorder = Arc::new(AudioRecorder::new()?);

        // Connect to OpenAI
        let openai_client = Arc::new(RwLock::new(Some(
            RealtimeClient::connect(api_key, model).await?,
        )));

        Ok(Self {
            openai_client,
            mcp_client,
            permissions,
            audio_player,
            audio_recorder,
            pending_approval: Arc::new(RwLock::new(None)),
        })
    }

    async fn configure_session(&self) -> Result<()> {
        // Load system prompt
        let instructions = tokio::fs::read_to_string("system_prompt.md")
            .await
            .unwrap_or_else(|_| "You are a helpful voice assistant.".to_string());

        // Convert MCP tools to OpenAI format
        let mcp_tools = self.mcp_client.list_tools().await;
        let mut tools: Vec<Tool> = mcp_tools
            .iter()
            .map(|t| {
                Tool::function(
                    &t.name,
                    t.description.as_deref().unwrap_or(&t.name),
                    t.input_schema.clone().unwrap_or(serde_json::json!({
                        "type": "object",
                        "properties": {},
                        "required": []
                    })),
                )
            })
            .collect();

        // Add built-in output_text tool
        tools.push(Tool::function(
            "output_text",
            "Output longer text content to the user's terminal",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The text content to display"
                    }
                },
                "required": ["text"]
            }),
        ));

        let config = SessionConfig {
            modalities: vec!["audio".to_string(), "text".to_string()],
            turn_detection: TurnDetectionConfig {
                type_: "server_vad".to_string(),
            },
            tools,
            tool_choice: "auto".to_string(),
            instructions,
        };

        let client = self.openai_client.read().await;
        if let Some(client) = client.as_ref() {
            client.update_session(config).await?;
        }

        Ok(())
    }

    async fn handle_events(&self) -> Result<()> {
        let mut client_guard = self.openai_client.write().await;
        let client = client_guard.as_mut().context("Client not connected")?;

        while let Some(event) = client.recv().await {
            match event {
                RealtimeEvent::SessionCreated { session } => {
                    tracing::debug!("Session created: {}", session.id);
                }
                RealtimeEvent::SessionUpdated { .. } => {
                    tracing::debug!("Session updated");
                }
                RealtimeEvent::ResponseAudioDelta { delta, .. } => {
                    // Decode and play audio
                    if let Ok(audio_bytes) = base64::decode(&delta) {
                        self.audio_player.play(&audio_bytes).await?;
                    }
                }
                RealtimeEvent::ResponseDone { response } => {
                    tracing::debug!("Response done: {}", response.status);

                    // Handle function calls
                    for item in response.output {
                        if let OutputItem::FunctionCall {
                            call_id,
                            name,
                            arguments,
                            ..
                        } = item
                        {
                            self.handle_function_call(&call_id, &name, &arguments)
                                .await?;
                        }
                    }
                }
                RealtimeEvent::Error { error } => {
                    tracing::error!("OpenAI error: {}", error.message);
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_function_call(&self, call_id: &str, name: &str, arguments: &str) -> Result<()> {
        let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Object(Default::default()));

        // Handle built-in output_text tool
        if name == "output_text" {
            if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
                println!("{}\n", text);
            }

            let client = self.openai_client.read().await;
            if let Some(client) = client.as_ref() {
                client
                    .add_function_result(
                        call_id.to_string(),
                        serde_json::to_string(&serde_json::json!({"success": true}))?,
                    )
                    .await?;
                client.create_response().await?;
            }
            return Ok(());
        }

        // Check permissions
        if self.permissions.is_denied(name, Some(&args)).await {
            tracing::info!("Tool '{}' is blocked by permissions", name);
            // Send error back
            // ... (similar to Python version)
            return Ok(());
        }

        let approved = if self.permissions.is_allowed(name, Some(&args)).await {
            tracing::debug!("Tool '{}' auto-approved", name);
            true
        } else {
            // Get user approval
            self.get_approval(name, &args).await?
        };

        if !approved {
            // Send rejection
            // ... (similar to Python version)
            return Ok(());
        }

        // Execute tool
        match self.mcp_client.call_tool(name, &args).await {
            Ok(result) => {
                tracing::info!("Tool '{}' executed successfully", name);

                let client = self.openai_client.read().await;
                if let Some(client) = client.as_ref() {
                    client
                        .add_function_result(call_id.to_string(), serde_json::to_string(&result)?)
                        .await?;
                    client.create_response().await?;
                }
            }
            Err(e) => {
                tracing::error!("Tool execution failed: {}", e);
            }
        }

        Ok(())
    }

    async fn get_approval(&self, tool_name: &str, args: &Value) -> Result<bool> {
        let (tx, mut rx) = mpsc::channel(1);

        // Set pending approval
        {
            let mut pending = self.pending_approval.write().await;
            *pending = Some((tool_name.to_string(), args.clone(), tx));
        }

        // Show approval prompt
        println!("🐛 tool call request: {}", tool_name);
        if let Some(obj) = args.as_object() {
            for (key, value) in obj {
                println!("   {}: {}", key, value);
            }
        }
        println!("🐛 approve this tool call?");
        println!("🐛   Right Cmd (or 'y' + Enter) = approve once");
        println!("🐛   Right Option (or 'n' + Enter) = reject once");
        println!("🐛   Shift + Right Cmd (or 'a' + Enter) = always allow");
        println!("🐛   'x' + Enter = never allow");

        // Wait for decision
        let decision = rx.recv().await.context("No approval received")?;

        // Clear pending
        {
            let mut pending = self.pending_approval.write().await;
            *pending = None;
        }

        // Handle decision
        match decision {
            ApprovalDecision::ApproveOnce => Ok(true),
            ApprovalDecision::RejectOnce => Ok(false),
            ApprovalDecision::AlwaysAllow => {
                self.permissions.add_allowed(tool_name, Some(args)).await?;
                Ok(true)
            }
            ApprovalDecision::NeverAllow => {
                self.permissions.add_denied(tool_name, Some(args)).await?;
                Ok(false)
            }
        }
    }
}

/// Run the main application
pub async fn run(api_key: String, model: String) -> Result<()> {
    let app = Arc::new(App::new(api_key, model).await?);

    // Configure session
    app.configure_session().await?;

    // Start keyboard listener (macOS only)
    #[cfg(target_os = "macos")]
    {
        let app_clone = Arc::clone(&app);
        tokio::spawn(async move {
            if let Err(e) = KeyboardListener::start(app_clone).await {
                tracing::error!("Keyboard listener error: {}", e);
            }
        });
    }

    // Handle CLI input
    let app_clone = Arc::clone(&app);
    tokio::spawn(async move {
        if let Err(e) = handle_cli_input(app_clone).await {
            tracing::error!("CLI input error: {}", e);
        }
    });

    // Handle OpenAI events
    app.handle_events().await?;

    Ok(())
}

async fn handle_cli_input(app: Arc<App>) -> Result<()> {
    use crossterm::event::{self, Event, KeyCode};

    loop {
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key_event) = event::read()? {
                // Check if we have pending approval
                let pending = app.pending_approval.read().await;
                if let Some((_, _, ref tx)) = *pending {
                    let decision = match key_event.code {
                        KeyCode::Char('y') => Some(ApprovalDecision::ApproveOnce),
                        KeyCode::Char('n') => Some(ApprovalDecision::RejectOnce),
                        KeyCode::Char('a') => Some(ApprovalDecision::AlwaysAllow),
                        KeyCode::Char('x') => Some(ApprovalDecision::NeverAllow),
                        _ => None,
                    };

                    if let Some(decision) = decision {
                        let _ = tx.send(decision).await;
                    }
                }
                drop(pending);
            }
        }
    }
}
