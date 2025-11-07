//! OpenAI Realtime API types

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// OpenAI Realtime API events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RealtimeEvent {
    #[serde(rename = "session.created")]
    SessionCreated { session: Session },

    #[serde(rename = "session.updated")]
    SessionUpdated { session: Session },

    #[serde(rename = "response.created")]
    ResponseCreated { response: Response },

    #[serde(rename = "response.done")]
    ResponseDone { response: Response },

    #[serde(rename = "response.audio.delta")]
    ResponseAudioDelta {
        item_id: String,
        content_index: u32,
        delta: String, // base64 encoded audio
    },

    #[serde(rename = "conversation.item.created")]
    ConversationItemCreated { item: ConversationItem },

    #[serde(rename = "input_audio_buffer.committed")]
    InputAudioBufferCommitted,

    #[serde(rename = "input_audio_buffer.speech_started")]
    InputAudioBufferSpeechStarted,

    #[serde(rename = "input_audio_buffer.speech_stopped")]
    InputAudioBufferSpeechStopped,

    #[serde(rename = "response.audio_transcript.done")]
    ResponseAudioTranscriptDone {
        item_id: String,
        content_index: u32,
        transcript: String,
    },

    #[serde(rename = "conversation.item.input_audio_transcription.completed")]
    InputAudioTranscriptionCompleted {
        item_id: String,
        content_index: u32,
        transcript: String,
    },

    #[serde(rename = "error")]
    Error { error: ErrorInfo },

    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_detection: Option<TurnDetection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnDetection {
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub output: Vec<OutputItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItem {
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(rename = "message")]
    Message {
        id: String,
        content: Vec<ContentPart>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "audio")]
    Audio { audio: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationItem {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Session configuration for OpenAI Realtime API
#[derive(Debug, Clone, Serialize)]
pub struct SessionConfig {
    pub modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_audio_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_audio_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_detection: Option<TurnDetectionConfig>,
    pub tools: Vec<Tool>,
    pub tool_choice: String,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnDetectionConfig {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_padding_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence_duration_ms: Option<u32>,
}

/// Tool definition for OpenAI Realtime API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub type_: String,
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl Tool {
    pub fn function(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            type_: "function".to_string(),
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// Message to send to OpenAI Realtime API
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum RealtimeMessage {
    #[serde(rename = "session.update")]
    SessionUpdate { session: SessionConfig },

    #[serde(rename = "input_audio_buffer.append")]
    InputAudioBufferAppend { audio: String },

    #[serde(rename = "input_audio_buffer.commit")]
    InputAudioBufferCommit,

    #[serde(rename = "response.create")]
    ResponseCreate,

    #[serde(rename = "response.cancel")]
    ResponseCancel,

    #[serde(rename = "conversation.item.create")]
    ConversationItemCreate { item: ConversationItemInput },
}

#[derive(Debug, Serialize)]
pub struct ConversationItemInput {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}
