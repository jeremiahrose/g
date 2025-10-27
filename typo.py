#!/usr/bin/env uv run
#
# /// script
# requires-python = ">=3.9"
# dependencies = [
#     "numpy",
#     "pyaudio",
#     "pydub",
#     "sounddevice",
#     "openai[realtime]",
#     "fastmcp",
#     "pynput",
# ]
#
# ///
from __future__ import annotations
import base64
import asyncio
import json
import sys
from typing import Any, cast
from fastmcp import Client
import os
from audio_util import CHANNELS, SAMPLE_RATE, AudioPlayerAsync
from openai import AsyncOpenAI
from openai.types.beta.realtime.session import Session
from openai.resources.beta.realtime.realtime import AsyncRealtimeConnection
from pynput import keyboard
import threading

# Global log level setting
LOG_LEVEL = "info"  # Options: "debug", "info", "error"

def should_log(level: str) -> bool:
    """Check if we should log at the given level based on current LOG_LEVEL."""
    levels = {"debug": 0, "info": 1, "error": 2}
    current_level_num = levels.get(LOG_LEVEL, 1)
    message_level_num = levels.get(level, 1)
    return message_level_num >= current_level_num

def info(message: str, **kwargs) -> None:
    """Print info message with mascot emoji."""
    if should_log("info"):
        print(f"🐛 {message}", **kwargs)

def debug(message: str) -> None:
    """Print debug message with gear emoji."""
    if should_log("debug"):
        print(f"⚙️ {message}")

def error(message: str) -> None:
    """Print error message with red cross."""
    if should_log("error"):
        print(f"❌ {message}")


def load_system_prompt() -> str:
    """Load system prompt from system_prompt.md file."""
    try:
        with open("system_prompt.md", "r") as f:
            instructions = f.read().strip()
        debug("loaded system prompt from system_prompt.md: " + instructions)
        return instructions
    except FileNotFoundError:
        error("system_prompt.md not found - this file is required")
        raise
    except Exception as e:
        error(f"failed to load system_prompt.md: {e}")
        raise


class MCPClient:
    """MCP client for connecting to and managing MCP servers."""

    def __init__(self):
        self.available_tools: list[dict] = []
        self.client = None

    async def connect_to_mcp_servers(self):
        """Connect to MCP servers defined in configuration."""
        # Load configuration from mcp.json file
        try:
            with open("mcp.json", "r") as f:
                config = json.load(f)
        except FileNotFoundError:
            error("mcp.json not found, not using any MCP servers")
            config = {
                "mcpServers": {}
            }
        except json.JSONDecodeError as e:
            error(f"mcp.json parsing error: {e}")
            raise

        # Create and validate client with FastMCP
        try:
            current_dir = os.getcwd()
            self.client = Client(
                config,
                roots=[f"file://{current_dir}/"]
            )
            debug("loaded mcp.json")
        except Exception as e:
            error(f"mcp.json validation failed: {e}")
            raise

        # Connect and get available tools
        async with self.client as client:
            tools = await client.list_tools()
            self.available_tools = []

            for tool in tools:
                # Convert MCP tool to OpenAI function format
                openai_tool = {
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description or f"MCP tool: {tool.name}",
                    "parameters": tool.inputSchema or {"type": "object", "properties": {}, "required": []}
                }
                self.available_tools.append(openai_tool)

            # Add built-in output_text tool
            self.available_tools.append({
                "type": "function",
                "name": "output_text",
                "description": "Output longer text content to the user's terminal. Use this for any content that can't be spoken in 8 words or less, such as lists, code, detailed information, or multi-line responses.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "The text content to display to the user"
                        }
                    },
                    "required": ["text"]
                }
            })

    async def call_tool(self, tool_name: str, arguments: dict) -> dict:
        """Execute a tool call on the MCP server."""
        if not self.client:
            return {"error": "No MCP server configured"}

        try:
            async with self.client as client:
                result = await client.call_tool(tool_name, arguments)
                return {
                    "success": True,
                    "content": result.content if hasattr(result, 'content') else [{"type": "text", "text": str(result)}],
                    "isError": False
                }
        except Exception as e:
            return {
                "success": False,
                "error": str(e),
                "isError": True
            }

    def serialize_mcp_result(self, result: dict) -> dict:
        """Convert MCP result to JSON-serializable format."""
        serializable_result = {
            "success": result.get("success", False),
            "isError": result.get("isError", False)
        }

        # Convert content objects to strings
        content = result.get("content", [])
        content_strings = []
        for item in content:
            if hasattr(item, 'type') and item.type == "text":
                content_strings.append(item.text)
            elif isinstance(item, dict) and item.get("type") == "text":
                content_strings.append(item.get('text', ''))
            else:
                content_strings.append(str(item))

        serializable_result["content"] = "\n".join(content_strings) if content_strings else ""

        # Add error if present
        if "error" in result:
            serializable_result["error"] = result["error"]

        return serializable_result

    def print_result(self, tool_name: str, args: dict, result: dict) -> None:
        """Print MCP tool execution result to terminal."""

        # Display the result
        if result.get("success"):
            content = result.get("content", [])
            content_strings = []

            for item in content:
                if hasattr(item, 'type') and item.type == "text":
                    content_strings.append(f"  {item.text}")
                elif isinstance(item, dict) and item.get("type") == "text":
                    content_strings.append(f"  {item.get('text', '')}")
                else:
                    content_strings.append(f"  {str(item)}")

            success_msg = "tool response:"
            if content_strings:
                success_msg += "\n" + "\n".join(content_strings)

            debug(success_msg)

            if result.get("isError"):
                error("tool reported an error")
        else:
            err_msg = result.get("error", "Unknown error")
            error(f"failed\n  {err_msg}")

        print()  # Add blank line for spacing

    async def close(self):
        """Close the MCP client connection."""
        # FastMCP Client uses async context managers, no explicit close needed
        pass


class GlobalKeyboardListener:
    """Global keyboard listener for tool approval using function keys."""

    def __init__(self, app):
        self.app = app
        self.listener = None
        self.running = False

    def start(self):
        """Start the global keyboard listener in a separate thread."""
        if self.running:
            return

        self.running = True
        self.listener = keyboard.Listener(on_press=self.on_key_press)
        self.listener.start()
        debug("global keyboard listener started (Right Cmd=approve, Right Option=reject)")

    def stop(self):
        """Stop the global keyboard listener."""
        if self.listener:
            self.listener.stop()
            self.running = False
            debug("global keyboard listener stopped")

    def on_key_press(self, key):
        """Handle key press events."""
        try:
            if not self.app.pending_tool_approval:
                return

            if key == keyboard.Key.cmd_r:
                # Right Command = Approve
                self.app.approve_pending_tool()
            elif key == keyboard.Key.alt_r:
                # Right Option/Alt = Reject
                self.app.reject_pending_tool()

        except Exception as e:
            debug(f"keyboard listener error: {e}")


class RealtimeApp:

    def __init__(self) -> None:
        self.connection = None
        self.session = None

        # Initialize OpenAI client with debugging
        try:
            self.client = AsyncOpenAI()
            debug("OpenAI client initialized successfully")
        except Exception as e:
            error(f"failed to initialize OpenAI client: {e}")
            raise

        self.audio_player = AudioPlayerAsync()
        self.last_audio_item_id = None
        self.should_send_audio = asyncio.Event()
        self.connected = asyncio.Event()
        self.mcp_client = MCPClient()
        self.is_recording = False
        self.response_started = False
        self.pending_tool_approval = None  # (tool_name, args, future)
        self.keyboard_listener = GlobalKeyboardListener(self)


    async def start(self) -> None:
        """Start the application."""
        # Check for OpenAI API key
        api_key = os.getenv("OPENAI_API_KEY")
        if not api_key:
            error("OPENAI_API_KEY environment variable not set")
            return
        debug(f"OpenAI API key found (length: {len(api_key)})")

        info("typo is here to do your bidding")
        print("" + "="*34)

        # Start keyboard listener for tool approvals
        self.keyboard_listener.start()

        # Start background tasks and keep references
        self.realtime_task = asyncio.create_task(self.handle_realtime_connection())
        self.audio_task = asyncio.create_task(self.send_mic_audio())

        # Initialize MCP first, then start input handling
        await self.initialize_mcp()

        try:
            # Handle user input (after MCP is ready)
            await self.handle_input()
        except KeyboardInterrupt:
            print("\n"); debug("shutting down...")
        finally:
            await self.cleanup()

    async def cleanup(self) -> None:
        """Clean up background tasks and connections."""
        print("\n"); info("cleaning up...")

        # Cancel background tasks
        if hasattr(self, 'realtime_task'):
            self.realtime_task.cancel()
        if hasattr(self, 'audio_task'):
            self.audio_task.cancel()

        # Stop keyboard listener
        self.keyboard_listener.stop()

        # Close MCP client
        await self.mcp_client.close()

        # Wait for tasks to finish cancelling
        tasks = []
        if hasattr(self, 'realtime_task'):
            tasks.append(self.realtime_task)
        if hasattr(self, 'audio_task'):
            tasks.append(self.audio_task)

        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)

    async def initialize_mcp(self) -> None:
        """Initialize MCP client connection."""
        try:
            await self.mcp_client.connect_to_mcp_servers()
            debug(f"mcp servers connected with {len(self.mcp_client.available_tools)} tools")
        except Exception as e:
            error(f"MCP connection failed: {e}")
            # Don't let MCP failure stop the app

    async def handle_realtime_connection(self) -> None:
        try:
            debug("attempting to connect to OpenAI Realtime API...")
            async with self.client.beta.realtime.connect(model="gpt-realtime-2025-08-28") as conn:
                debug("successfully connected to OpenAI Realtime API")
                self.connection = conn
                self.connected.set()

                # Wait for MCP client to be ready
                max_wait = 10  # seconds
                wait_count = 0
                while not self.mcp_client.available_tools and wait_count < max_wait:
                    await asyncio.sleep(1)
                    wait_count += 1

                # Configure session with MCP tools
                tools = self.mcp_client.available_tools
                debug(f"configuring session with {len(tools)} tools")

                try:
                    await conn.session.update(session={
                        "modalities": ["audio", "text"],
                        "turn_detection": {"type": "server_vad"},
                        "tools": tools,
                        "tool_choice": "auto",
                        "instructions": load_system_prompt()
                    })
                    debug("session configuration successful")
                except Exception as e:
                    error(f"session configuration failed: {e}")
                    raise

                acc_items: dict[str, Any] = {}
                debug("starting event loop...")

                async for event in conn:
                    # debug(f"received event: {event.type}")

                    if event.type == "session.created":
                        debug(f"session created: {event.session.id}")
                        self.session = event.session
                        continue

                    if event.type == "session.updated":
                        debug("session updated successfully")
                        debug(str(event.session))
                        self.session = event.session
                        continue

                    if event.type == "error":
                        error(f"OpenAI API error: {getattr(event, 'error', 'unknown error')}")
                        if hasattr(event, 'error') and hasattr(event.error, 'message'):
                            error(f"error details: {event.error.message}")
                        continue

                    if event.type == "response.created":
                        response_id = getattr(event.response, 'id', 'unknown') if hasattr(event, 'response') else 'unknown'
                        debug(f"response created: {response_id}")
                        continue

                    if event.type == "response.audio.delta":
                        if event.item_id != self.last_audio_item_id:
                            # debug(f"new audio item: {event.item_id}")
                            self.audio_player.reset_frame_count()
                            self.last_audio_item_id = event.item_id

                        bytes_data = base64.b64decode(event.delta)
                        self.audio_player.add_data(bytes_data)
                        continue

                    if event.type == "response.done":
                        debug("response completed")

                        # Debug the response contents
                        if hasattr(event, 'response'):
                            response = event.response
                            status = getattr(response, 'status', 'unknown')
                            debug(f"response status: {status}")

                            # If response failed, look for error details
                            if status == 'failed':
                                error("Response failed!")
                                if hasattr(response, 'status_details'):
                                    error(f"failure reason: {response.status_details}")
                                if hasattr(response, 'error'):
                                    error(f"response error: {response.error}")

                            if hasattr(response, 'output'):
                                debug(f"response has {len(response.output)} output items")
                                for i, item in enumerate(response.output):
                                    item_type = getattr(item, 'type', 'unknown')
                                    debug(f"output item {i}: type={item_type}")
                                    if hasattr(item, 'content'):
                                        debug(f"  content: {getattr(item.content, 'text', 'no text') if hasattr(item, 'content') else 'no content'}")
                            else:
                                debug("response has no output")
                        else:
                            debug("event has no response object")

                        # Print newline after response is complete
                        if self.response_started:
                            print()  # Move to new line after streaming is complete
                            self.response_started = False

                        # Check if response contains function calls
                        if hasattr(event, 'response') and hasattr(event.response, 'output'):
                            for output_item in event.response.output:
                                if hasattr(output_item, 'type') and output_item.type == "function_call":
                                    debug(f"function call detected: {output_item.name}")
                                    await self.handle_function_call(output_item)
                        continue

                    if event.type == "input_audio_buffer.committed":
                        debug("audio buffer committed")
                        continue

                    if event.type == "input_audio_buffer.speech_started":
                        debug("speech started detected")
                        continue

                    if event.type == "input_audio_buffer.speech_stopped":
                        debug("speech stopped detected")
                        continue

                    if event.type == "conversation.item.created":
                        if hasattr(event, 'item'):
                            item = event.item
                            item_type = getattr(item, 'type', 'unknown')
                            item_id = getattr(item, 'id', 'unknown')
                            debug(f"conversation item created: type={item_type}, id={item_id}")

                            # Check if it's a message with content
                            if item_type == 'message' and hasattr(item, 'content'):
                                debug(f"message content length: {len(item.content)} items")
                                for i, content in enumerate(item.content):
                                    content_type = getattr(content, 'type', 'unknown')
                                    debug(f"  content {i}: type={content_type}")
                                    if content_type == 'input_audio' and hasattr(content, 'audio'):
                                        audio_len = len(content.audio) if content.audio else 0
                                        debug(f"    audio data length: {audio_len}")
                        else:
                            debug("conversation item created: no item details")
                        continue

                    # Check for any response-related events we might be missing
                    if "response" in event.type:
                        debug(f"unhandled response event: {event.type}")
                        if hasattr(event, 'item_id'):
                            debug(f"  item_id: {event.item_id}")
                        if hasattr(event, 'content_index'):
                            debug(f"  content_index: {event.content_index}")
                        # Look for any error information in response events
                        if hasattr(event, 'error'):
                            error(f"error in {event.type}: {event.error}")

                    # Log any unhandled event types
                    debug(f"unhandled event type: {event.type}")
        except asyncio.CancelledError:
            # Task was cancelled, exit gracefully
            pass
        except Exception as e:
            error(f"realtime connection error: {e}")

    async def _get_connection(self) -> AsyncRealtimeConnection:
        await self.connected.wait()
        assert self.connection is not None
        return self.connection

    def approve_pending_tool(self):
        """Approve the pending tool call (called by keyboard listener)."""
        if self.pending_tool_approval:
            tool_name, args, future = self.pending_tool_approval
            if not future.done():
                future.set_result(True)
                info("tool call approved (Right Cmd)")

    def reject_pending_tool(self):
        """Reject the pending tool call (called by keyboard listener)."""
        if self.pending_tool_approval:
            tool_name, args, future = self.pending_tool_approval
            if not future.done():
                future.set_result(False)
                info("tool call rejected (Right Option)")

    async def get_user_approval(self, tool_name: str, args: dict) -> bool:
        """Get user approval for tool execution via main input loop."""
        # Set up pending approval and wait for result
        future = asyncio.Future()
        self.pending_tool_approval = (tool_name, args, future)

        # Display the tool request
        tool_msg = f"tool call request: {tool_name}"
        if args:
            for key, value in args.items():
                tool_msg += f"\n   {key}: {value}"
        info(tool_msg)
        info("approve this tool call? Press Right Cmd to approve, Right Option to reject (or 'y'/'n' + Enter)")

        # Wait for keyboard listener or CLI input to resolve this
        return await future

    async def handle_function_call(self, function_call_item: Any) -> None:
        """Handle function calls from the model."""
        tool_name = function_call_item.name

        # Parse the function arguments
        try:
            args = json.loads(function_call_item.arguments)
        except json.JSONDecodeError:
            args = {}

        # Handle built-in output_text tool without approval
        if tool_name == "output_text":
            text = args.get("text", "")
            # Print directly to stdout without emoji prefix
            print(text)
            print()  # Add blank line for spacing

            # Send success result back to the model
            connection = await self._get_connection()
            await connection.conversation.item.create(
                item={
                    "type": "function_call_output",
                    "call_id": function_call_item.call_id,
                    "output": json.dumps({"success": True, "displayed": True})
                }
            )
            await connection.response.create()
            return

        # Get user approval for tool execution
        approved = await self.get_user_approval(tool_name, args)

        if not approved:
            # Send denial result back to the model
            connection = await self._get_connection()
            await connection.conversation.item.create(
                item={
                    "type": "function_call_output",
                    "call_id": function_call_item.call_id,
                    "output": json.dumps({"error": "Tool call denied by user"})
                }
            )
            await connection.response.create()
            return

        # Handle MCP tool calls
        # Execute the MCP tool
        result = await self.mcp_client.call_tool(tool_name, args)

        # Display the result
        self.mcp_client.print_result(tool_name, args, result)

        # Send function call result back to the model
        connection = await self._get_connection()
        await connection.conversation.item.create(
            item={
                "type": "function_call_output",
                "call_id": function_call_item.call_id,
                "output": json.dumps(self.mcp_client.serialize_mcp_result(result))
            }
        )

        # Generate a response from the model
        await connection.response.create()

    async def send_mic_audio(self) -> None:
        import sounddevice as sd  # type: ignore

        sent_audio = False

        # Query devices but don't print the list
        sd.query_devices()

        read_size = int(SAMPLE_RATE * 0.02)

        stream = sd.InputStream(
            channels=CHANNELS,
            samplerate=SAMPLE_RATE,
            dtype="int16",
        )
        stream.start()

        try:
            while True:
                if stream.read_available < read_size:
                    await asyncio.sleep(0)
                    continue

                await self.should_send_audio.wait()
                self.is_recording = True

                data, _ = stream.read(read_size)

                connection = await self._get_connection()
                if not sent_audio:
                    # Only try to cancel if there might be an active response
                    # Skip the cancel on first audio to avoid the error
                    debug("starting to send audio (skipping response.cancel on first audio)")
                    sent_audio = True

                try:
                    await connection.input_audio_buffer.append(audio=base64.b64encode(cast(Any, data)).decode("utf-8"))
                except Exception as e:
                    error(f"failed to append audio data: {e}")
                    if "1000" in str(e):
                        error("connection closed normally - likely due to end of conversation")
                    else:
                        error("unexpected audio connection error")
                    break

                await asyncio.sleep(0)
        except asyncio.CancelledError:
            # Task was cancelled, exit gracefully
            pass
        except KeyboardInterrupt:
            pass
        except Exception as e:
            error(f"audio error: {e}")
        finally:
            stream.stop()
            stream.close()

    async def handle_input(self) -> None:
        """Handle user input from terminal."""
        import asyncio

        loop = asyncio.get_event_loop()

        def get_input():
            return input()

        # Show initial recording prompt
        recording_prompt = "press 'k' + Enter to start recording ('q' + Enter to quit)"
        info(f"{recording_prompt}")

        try:
            while True:
                try:
                    user_input = await loop.run_in_executor(None, get_input)

                    # Handle tool approval if pending
                    if self.pending_tool_approval:
                        tool_name, args, future = self.pending_tool_approval
                        if user_input.lower() in ['y', 'yes']:
                            future.set_result(True)
                            debug("tool call approved")
                        elif user_input.lower() in ['n', 'no']:
                            future.set_result(False)
                            debug("tool call denied")
                        else:
                            print("please enter 'y' for yes or 'n' for no:")
                            continue

                        self.pending_tool_approval = None
                        continue

                    if user_input == "q":
                        info("goodbye!")
                        return

                    if user_input == "k":
                        if self.is_recording:
                            self.should_send_audio.clear()
                            self.is_recording = False
                            info("recording stopped")
                            info(recording_prompt)

                            if self.session and self.session.turn_detection is None:
                                # The default in the API is that the model will automatically detect when the user has
                                # stopped talking and then start responding itself.
                                #
                                # However if we're in manual `turn_detection` mode then we need to
                                # manually tell the model to commit the audio buffer and start responding.
                                try:
                                    conn = await self._get_connection()
                                    await conn.input_audio_buffer.commit()
                                    await conn.response.create()
                                    debug("manually triggered response in manual turn detection mode")
                                except Exception as e:
                                    error(f"failed to trigger manual response: {e}")
                        else:
                            self.should_send_audio.set()
                            self.is_recording = True
                            info("recording started... (press 'k' + enter to stop)")

                except EOFError:
                    break

        except KeyboardInterrupt:
            print("\n"); info("goodbye!")


async def main():
    app = RealtimeApp()
    try:
        await app.start()
    except KeyboardInterrupt:
        print("\n"); info("goodbye!")

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass  # Graceful shutdown
