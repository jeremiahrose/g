# Architecture Documentation

## Overview

Typo is built as a modular Rust application using a workspace structure that separates core functionality from UI concerns. This design enables code reuse across multiple interfaces (CLI, GUI, web).

## Workspace Structure

```
typo-rs/
├── Cargo.toml          # Workspace manifest
├── core/               # Core library (platform-agnostic)
└── cli/                # CLI binary (platform-specific)
```

## Core Library (`typo-core`)

The core library provides all business logic and external integrations. It's designed to be reusable across different frontends.

### Module Organization

```
core/src/
├── lib.rs              # Public API
├── error.rs            # Error types
├── config/             # Configuration management
│   ├── mod.rs
│   └── settings.rs
├── mcp/                # MCP client
│   ├── mod.rs
│   ├── client.rs
│   └── types.rs
├── openai/             # OpenAI Realtime API
│   ├── mod.rs
│   ├── realtime.rs
│   └── types.rs
└── permissions/        # Permission system
    └── mod.rs
```

### Key Components

#### 1. MCP Client (`mcp/`)

Manages connections to MCP servers via subprocess communication.

**Responsibilities:**
- Spawn and manage MCP server processes
- Send JSON-RPC requests over stdin
- Receive JSON-RPC responses from stdout
- Tool discovery and execution
- Connection lifecycle management

**Key Types:**
- `McpClient` - Main client interface
- `Tool` - Tool definition
- `ToolCallResult` - Execution result
- `ServerConnection` - Per-server state

**Example:**
```rust
let client = McpClient::new().await?;
client.connect_to_servers().await?;
let tools = client.list_tools().await;
let result = client.call_tool("my_tool", &args).await?;
```

#### 2. OpenAI Realtime Client (`openai/`)

WebSocket-based client for OpenAI's Realtime API.

**Responsibilities:**
- WebSocket connection management
- Event streaming
- Session configuration
- Audio buffer handling
- Function call management

**Key Types:**
- `RealtimeClient` - Main client interface
- `RealtimeEvent` - Incoming events
- `RealtimeMessage` - Outgoing messages
- `SessionConfig` - Session settings

**Example:**
```rust
let client = RealtimeClient::connect(api_key, model).await?;
client.update_session(config).await?;

while let Some(event) = client.recv().await {
    match event {
        RealtimeEvent::ResponseAudioDelta { delta, .. } => {
            // Handle audio
        }
        _ => {}
    }
}
```

#### 3. Permission Manager (`permissions/`)

Controls tool access based on user preferences.

**Responsibilities:**
- Load/save permission configuration
- Pattern matching for tool names
- Allow/deny list management
- Thread-safe concurrent access

**Key Types:**
- `PermissionManager` - Main interface
- `Settings` - Persistent configuration
- `PermissionsConfig` - Allow/deny lists

**Example:**
```rust
let manager = PermissionManager::new().await?;

if manager.is_denied("dangerous_tool", None).await {
    return Err("Tool blocked");
}

if manager.is_allowed("safe_tool", None).await {
    // Auto-approve
} else {
    // Prompt user
}
```

#### 4. Configuration (`config/`)

Handles loading and saving configuration files.

**Responsibilities:**
- Parse `mcp.json` (MCP server config)
- Parse `.typo/settings.json` (permissions)
- File I/O with async support
- Default configuration generation

**Key Types:**
- `McpConfig` - MCP server definitions
- `Settings` - App settings
- `PermissionsConfig` - Tool permissions

## CLI Application (`typo-cli`)

Platform-specific implementation with audio I/O and keyboard support.

### Module Organization

```
cli/src/
├── main.rs             # Entry point, CLI parsing
├── app.rs              # Main application logic
├── audio/              # Audio I/O
│   ├── mod.rs
│   ├── player.rs       # Audio output
│   └── recorder.rs     # Audio input
└── keyboard.rs         # Global keyboard listener (macOS)
```

### Key Components

#### 1. Main App (`app.rs`)

Orchestrates all components and handles the main event loop.

**Responsibilities:**
- Initialize all subsystems
- Configure OpenAI session
- Handle incoming events
- Manage tool approval flow
- Coordinate audio streaming

**Key Types:**
- `App` - Main application state
- `ApprovalDecision` - User approval choices

**Flow:**
```
1. Initialize MCP client
2. Initialize permissions
3. Initialize audio I/O
4. Connect to OpenAI
5. Configure session with tools
6. Start event loop:
   - Receive OpenAI events
   - Handle audio deltas
   - Process function calls
   - Manage approvals
```

#### 2. Audio (`audio/`)

Cross-platform audio input/output using `cpal`.

**Player (`player.rs`):**
- Output stream to system audio device
- Queue-based buffering
- PCM16 24kHz format
- Automatic device selection

**Recorder (`recorder.rs`):**
- Input stream from microphone
- Channel-based audio forwarding
- Start/stop control
- Format conversion

**Example:**
```rust
let player = AudioPlayer::new()?;
player.play(&audio_bytes).await?;

let recorder = AudioRecorder::new()?;
let mut rx = recorder.start_recording().await;
while let Some(audio) = rx.recv().await {
    // Process audio
}
```

#### 3. Keyboard Listener (`keyboard.rs`)

Global keyboard event handling for macOS.

**Responsibilities:**
- Listen for system-wide key events
- Track modifier key state (Shift, Cmd)
- Send approval decisions
- Run in background thread

**Supported Shortcuts:**
- Right Cmd: Approve once
- Right Option: Reject once
- Shift + Right Cmd: Always allow

## Data Flow

### Tool Execution Flow

```
1. User speaks request
   ↓
2. Audio → OpenAI Realtime API
   ↓
3. OpenAI responds with function_call
   ↓
4. App checks permissions
   ├─ Denied → Send error to OpenAI
   ├─ Allowed → Execute immediately
   └─ Unknown → Request approval
      ├─ User approves → Execute
      └─ User rejects → Send error
   ↓
5. MCP client executes tool
   ↓
6. Result → OpenAI
   ↓
7. OpenAI responds with audio
   ↓
8. Audio → Speaker
```

### Configuration Flow

```
Startup:
  ├─ Load mcp.json → McpConfig
  ├─ Load .typo/settings.json → Settings
  └─ Load system_prompt.md → String

Runtime:
  └─ User approves with "always allow"
     └─ Update Settings
        └─ Save to .typo/settings.json
```

## Concurrency Model

### Tokio Runtime

All async code runs on the Tokio runtime with multiple threads.

**Key Patterns:**

1. **Channels** - Inter-task communication
   ```rust
   let (tx, rx) = mpsc::unbounded_channel();
   tokio::spawn(async move {
       tx.send(data).unwrap();
   });
   ```

2. **RwLock** - Shared state with reader-writer lock
   ```rust
   let state = Arc::new(RwLock::new(State::new()));
   let reader = state.read().await;
   let mut writer = state.write().await;
   ```

3. **Spawned Tasks** - Background processing
   ```rust
   tokio::spawn(async move {
       // Background work
   });
   ```

### Thread Safety

- `Arc<RwLock<T>>` for shared mutable state
- `Arc<T>` for shared immutable state
- Message passing via channels for coordination
- No unsafe code in core logic

## Future Extensions

### Tauri GUI

Add a new workspace member:

```toml
[workspace]
members = ["core", "cli", "tray"]
```

```rust
// tray/src/main.rs
use typo_core::{McpClient, PermissionManager, RealtimeClient};

#[tauri::command]
async fn execute_tool(name: String, args: Value) -> Result<String> {
    let client = get_mcp_client();
    let result = client.call_tool(&name, &args).await?;
    Ok(serde_json::to_string(&result)?)
}
```

### Web Assembly

Compile `core` to WASM:

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "0.2"
```

### Plugin System

Dynamic tool loading:

```rust
trait ToolProvider {
    fn list_tools(&self) -> Vec<Tool>;
    fn execute(&self, name: &str, args: &Value) -> Result<Value>;
}

impl McpClient {
    pub fn register_provider(&mut self, provider: Box<dyn ToolProvider>) {
        self.providers.push(provider);
    }
}
```

## Testing Strategy

### Unit Tests

Test individual components in isolation:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_permission_matching() {
        let manager = PermissionManager::new().await.unwrap();
        assert!(manager.matches_pattern("my_tool", "my_*"));
    }
}
```

### Integration Tests

Test component interactions:

```rust
#[tokio::test]
async fn test_mcp_client() {
    let client = McpClient::new().await.unwrap();
    client.connect_to_servers().await.unwrap();
    let tools = client.list_tools().await;
    assert!(!tools.is_empty());
}
```

### Performance Tests

Benchmark critical paths:

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_permission_check(c: &mut Criterion) {
    c.bench_function("permission_check", |b| {
        b.iter(|| manager.is_allowed(black_box("tool"), None))
    });
}
```

## Deployment

### Release Build

```bash
cargo build --release
strip target/release/typo  # Optional: reduce binary size
```

### Distribution

- macOS: Code sign and notarize
- Linux: AppImage or .deb package
- Cross-compile for other platforms

## Performance Considerations

### Memory

- Use `Arc` for shared ownership
- Prefer stack allocation
- Stream large data (don't buffer)
- Limit audio queue size

### CPU

- Async I/O prevents blocking
- Audio processing in dedicated streams
- Minimal copies in hot paths

### Latency

- Direct WebSocket for OpenAI (no proxies)
- Audio queue tuning for low latency
- Fast permission checks (in-memory)

## Security

### Sandboxing

- MCP servers run as separate processes
- Permissions prevent unauthorized access
- User approval for all tools (by default)

### Secrets

- API keys from environment only
- No logging of sensitive data
- Secure WebSocket (TLS)

## Dependencies

### Core

- `tokio` - Async runtime
- `serde` - Serialization
- `reqwest` - HTTP client
- `tokio-tungstenite` - WebSocket
- `thiserror` - Error handling

### CLI

- `cpal` - Audio I/O
- `rdev` - Keyboard events
- `clap` - CLI parsing
- `crossterm` - Terminal I/O
