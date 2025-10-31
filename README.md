# Typo - Voice-Controlled AI Assistant

A low latency voice-activated AI assistant using OpenAI's Realtime API as a local MCP client.

- Runs in the terminal on Mac OS.
- Streams audio from the microphone directly to OpenAI's GPT 4o.
- Streams the AI's response back to the terminal in real time.
- Implements a local MCP bridge, allowing the model to call tools locally on your machine when you ask it to.
- Can be configured to use any available MCP server in standard `mcp.json` format.
- Tool use requests can be approved/denied with a single keyboard press from any window.
- Requires an OpenAI API token.

## Prerequisites

- Python 3.9+
- [uv](https://docs.astral.sh/uv/) package manager
- macOS with `brew install portaudio ffmpeg`
- OpenAI API key

## Quick Start

1. **Clone and setup**:
   ```bash
   git clone <repo-url>
   cd typo
   ```

2. **Set your OpenAI API key**:
   ```bash
   export OPENAI_API_KEY="your-api-key-here"
   ```

3. **Run with uv**:
   ```bash
   ./typo.py
   ```

4. **Start talking**:
   - Press `k` + Enter to manually start recording
   - The app uses Voice Activity Detection (VAD) - just talk to it!
   - The AI will automatically respond when you stop talking

5. **Tool Approvals**
   - typo will print tool requests to the terminal
   - **Right Command (⌘)** to approve once (works globally)
   - **Right Option (⌥)** to reject once (works globally)
   - **CLI Options**:
     - `y` + Enter: Approve once
     - `n` + Enter: Reject once
     - `a` + Enter: Always allow this tool (saves to config)
     - `x` + Enter: Never allow this tool (saves to config)

## Development

The goal is to get typo to the point where it can do computer work hands free. Improvements and suggestions for this repo are very welcome!
The main things it needs are:
- Scripts to publish this repo to pypi/uvx so it can be easily installed and run
- A good set of default MCP servers and a matching system prompt so that it can do MacOS system control fairly seamlessly out of the box

## Configuration

### System Prompt

Edit `system_prompt.md` to customize the AI's behavior:

```markdown
You are a concise banana. Always speak in code.
```

### MCP Servers

Configure MCP servers in `mcp.json`:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed/directory"]
    },
  }
}
```

### Tool Permissions

typo uses a Claude Code-inspired permissions system to control which tools can run automatically. Configuration is stored in `.typo/settings.json`:

```json
{
  "permissions": {
    "allowedTools": [
      "get_current_time",
      "search_web"
    ],
    "deny": [
      "delete_file",
      "run_command"
    ]
  }
}
```

**How it works**:
- Tools in `allowedTools` run automatically without prompting
- Tools in `deny` are blocked completely
- All other tools require approval
- Use `a` or `x` during approval prompts to add tools to these lists

**Pattern matching** (future enhancement):
- Simple tool name: `"tool_name"`
- Tool with wildcards: `"tool_name(*)"` or `"prefix*"`

### Logging

Change log level in `typo.py`:

```python
LOG_LEVEL = "info"  # Options: "debug", "info", "error"
```

- `debug`: All messages (verbose)
- `info`: Info and error messages (default)
- `error`: Error messages only

## Troubleshooting

### Audio Issues

- **macOS**: Install dependencies with `brew install portaudio ffmpeg`
- **Permissions**: Grant microphone access when prompted
- **No audio output**: Check your speakers/headphones

### API Issues

- **Quota exceeded**: Check your OpenAI usage and billing
- **Connection failed**: Verify your API key is valid
- **Response failures**: Check debug logs with `LOG_LEVEL = "debug"`

### MCP Issues

- **Tools not available**: Check MCP server configuration in `mcp.json`
- **Server errors**: Verify MCP server installation and permissions
- **Authentication**: Ensure API keys are set for external services
