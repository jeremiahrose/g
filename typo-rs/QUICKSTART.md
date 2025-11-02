# Quick Start Guide

Get Typo running in 5 minutes.

## 1. Install Dependencies

### macOS

```bash
brew install portaudio pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Linux (Ubuntu/Debian)

```bash
sudo apt-get install libasound2-dev pkg-config build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## 2. Build

```bash
cd typo-rs
cargo build --release
```

This will take 3-5 minutes on first build (compiles dependencies).

## 3. Configure

### Set API Key

```bash
export OPENAI_API_KEY="sk-your-key-here"
```

Add to `~/.bashrc` or `~/.zshrc` to persist:

```bash
echo 'export OPENAI_API_KEY="sk-your-key-here"' >> ~/.zshrc
```

### Create System Prompt

```bash
cat > system_prompt.md << 'EOF'
You are a helpful voice assistant. Be concise and friendly.
When the user asks you to do something, use the available tools to help them.
EOF
```

### Create MCP Config (Optional)

```bash
cat > mcp.json << 'EOF'
{
  "mcpServers": {
    "example": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "."]
    }
  }
}
EOF
```

## 4. Run

```bash
cargo run --release --bin typo
```

Or use the compiled binary directly:

```bash
./target/release/typo
```

## 5. Test

Once running, you should see:

```
🐛 typo is here to do your bidding
==================================
```

### Try Voice Input

The app uses Voice Activity Detection (VAD) - just start talking!

### Try a Tool

Say: "What time is it?"

The AI will attempt to call a tool and prompt you for approval:

```
🐛 tool call request: get_current_time
🐛 approve this tool call?
🐛   Right Cmd (or 'y' + Enter) = approve once
🐛   Right Option (or 'n' + Enter) = reject once
🐛   Shift + Right Cmd (or 'a' + Enter) = always allow
🐛   'x' + Enter = never allow
```

Press Right Cmd (⌘) or type `y` then Enter.

## Common Issues

### "No input device available"

Grant microphone access:
- **macOS**: System Preferences → Security & Privacy → Microphone
- **Linux**: Check `arecord -l` to list devices

### "Failed to connect to OpenAI"

- Verify API key is set: `echo $OPENAI_API_KEY`
- Check your OpenAI account has credits
- Ensure internet connectivity

### "MCP server failed to start"

- Verify command path in `mcp.json`
- Test command manually: `/path/to/mcp-server`
- Check MCP server logs

### Build fails with linking errors

Install system dependencies:

```bash
# macOS
brew install portaudio pkg-config

# Linux
sudo apt-get install libasound2-dev pkg-config build-essential
```

## Next Steps

1. **Customize System Prompt**: Edit `system_prompt.md`
2. **Add MCP Servers**: Edit `mcp.json`
3. **Configure Permissions**: Create `.typo/settings.json`
4. **Read Full Docs**: See `README.md` and `ARCHITECTURE.md`

## Keyboard Shortcuts

| Action | Shortcut | CLI Alternative |
|--------|----------|-----------------|
| Approve once | Right Cmd (⌘) | `y` + Enter |
| Reject once | Right Option (⌥) | `n` + Enter |
| Always allow | Shift + Right Cmd | `a` + Enter |
| Never allow | - | `x` + Enter |

## Example Configuration

### Minimal Setup

Just need API key and system prompt:

```bash
export OPENAI_API_KEY="sk-..."
echo "You are a helpful assistant." > system_prompt.md
cargo run --release --bin typo
```

### With MCP Tools

Add filesystem access:

```json
{
  "mcpServers": {
    "fs": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/Users/you/Documents"]
    }
  }
}
```

### With Permissions

Pre-approve safe tools:

```json
{
  "permissions": {
    "allowedTools": [
      "get_current_time",
      "get_weather"
    ],
    "deny": [
      "delete_file",
      "run_command"
    ]
  }
}
```

Save to `.typo/settings.json`.

## Logging

Set log level:

```bash
RUST_LOG=debug cargo run --bin typo
```

Levels: `trace`, `debug`, `info`, `warn`, `error`

## Getting Help

- Check `README.md` for detailed documentation
- See `MIGRATION.md` for Python → Rust differences
- Read `ARCHITECTURE.md` for technical details
- File issues on GitHub
