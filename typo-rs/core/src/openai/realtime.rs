//! OpenAI Realtime API client implementation

use super::types::*;
use crate::error::{Error, Result};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const REALTIME_API_URL: &str = "wss://api.openai.com/v1/realtime";

/// Client for OpenAI Realtime API
pub struct RealtimeClient {
    event_rx: mpsc::UnboundedReceiver<RealtimeEvent>,
    message_tx: mpsc::UnboundedSender<RealtimeMessage>,
}

impl RealtimeClient {
    /// Connect to the OpenAI Realtime API
    pub async fn connect(api_key: String, model: String) -> Result<Self> {
        let url = format!("{}?model={}", REALTIME_API_URL, model);

        tracing::info!("Connecting to OpenAI Realtime API...");

        // Create WebSocket request with proper headers
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = url.into_client_request()
            .map_err(|e| Error::OpenAI(format!("Failed to create request: {}", e)))?;

        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", api_key).parse().unwrap()
        );
        request.headers_mut().insert(
            "OpenAI-Beta",
            "realtime=v1".parse().unwrap()
        );

        let (ws_stream, _) = connect_async(request)
            .await
            .map_err(|e| Error::Connection(format!("Failed to connect to OpenAI: {}", e)))?;

        tracing::info!("Connected to OpenAI Realtime API");

        let (mut write, mut read) = ws_stream.split();

        // Create channels
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (message_tx, mut message_rx) = mpsc::unbounded_channel();

        // Spawn task to read events
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        match serde_json::from_str::<RealtimeEvent>(&text) {
                            Ok(event) => {
                                if event_tx.send(event).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse event: {}, raw: {}", e, text);
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        tracing::info!("WebSocket closed");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });

        // Spawn task to write messages
        tokio::spawn(async move {
            while let Some(msg) = message_rx.recv().await {
                let json = match serde_json::to_string(&msg) {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::error!("Failed to serialize message: {}", e);
                        continue;
                    }
                };

                if let Err(e) = write.send(Message::Text(json)).await {
                    tracing::error!("Failed to send message: {}", e);
                    break;
                }
            }
        });

        Ok(Self {
            event_rx,
            message_tx,
        })
    }

    /// Receive the next event from the API
    pub async fn recv(&mut self) -> Option<RealtimeEvent> {
        self.event_rx.recv().await
    }

    /// Send a message to the API
    pub async fn send(&self, message: RealtimeMessage) -> Result<()> {
        self.message_tx
            .send(message)
            .map_err(|_| Error::Connection("Failed to send message".to_string()))
    }

    /// Update the session configuration
    pub async fn update_session(&self, config: SessionConfig) -> Result<()> {
        self.send(RealtimeMessage::SessionUpdate { session: config })
            .await
    }

    /// Append audio to the input buffer
    pub async fn append_audio(&self, audio: String) -> Result<()> {
        self.send(RealtimeMessage::InputAudioBufferAppend { audio })
            .await
    }

    /// Commit the input audio buffer
    pub async fn commit_audio(&self) -> Result<()> {
        self.send(RealtimeMessage::InputAudioBufferCommit).await
    }

    /// Create a response
    pub async fn create_response(&self) -> Result<()> {
        self.send(RealtimeMessage::ResponseCreate).await
    }

    /// Add a function call result to the conversation
    pub async fn add_function_result(&self, call_id: String, output: String) -> Result<()> {
        self.send(RealtimeMessage::ConversationItemCreate {
            item: ConversationItemInput {
                type_: "function_call_output".to_string(),
                call_id: Some(call_id),
                output: Some(output),
            },
        })
        .await
    }
}
