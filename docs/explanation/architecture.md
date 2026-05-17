# Architecture Overview

hackpi is a Rust workspace with four crates, streaming communication, and a virtual bash environment.

## Crate structure

```
hackpi/
├── hackpi-core/       # Agent loop, API client, tool registry, shared types
├── hackpi-tools/      # read, search_grep, edit, write, bash
├── hackpi-tui/        # ratatui terminal interface
└── hackpi-guardrails/ # Permission checking and path validation
```

### Dependency graph

```
hackpi-tui
├── hackpi-core
│   └── hackpi-guardrails
└── hackpi-tools
    └── hackpi-core
        └── hackpi-guardrails
```

`hackpi-core` is the shared foundation. `hackpi-guardrails` sits at the bottom with no internal dependencies. `hackpi-tui` pulls in everything to wire the agent loop, tools, and UI together.

## Data flow

The core loop is event-driven, using tokio channels:

```
User types a message
  → TUI sends TuiEvent::Submit(text)
  → Agent loop appends to conversation history
  → Agent posts to /v1/messages (streaming)
  → SSE events flow back:
    → text deltas render in the TUI
    → tool_use blocks dispatch to the tool registry
    → tool results stream back to the LLM in the same turn
  → If stop_reason == "tool_use": continue the loop
  → If stop_reason != "tool_use": done, return to input
```

### Streaming tool results

This is the key architectural difference from a naive request/response loop. Tool results feed back to the LLM within the same turn. The agent loop:

1. Receives a `tool_use` content block from the API
2. Dispatches the tool
3. Streams each output chunk to the TUI and buffers it
4. When the tool completes, appends the result to messages
5. Immediately sends the next API request with the tool result

No user round-trip is required for multi-step tool use.

### Interruption

`Ctrl+C` sets a watch channel to `true`. The in-flight API request is aborted. In-flight tool executions check the signal at yield points. The agent loop breaks and returns a partial response.

## Tool system

All tools implement the `Tool` trait:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}
```

The `ToolRegistry` holds all registered tools and provides:

- `get()` — look up a tool by name
- `all_schemas()` — collect schemas for the API request
- `dispatch()` — route a tool call to the right implementation

Each tool receives a `ToolContext` with the workspace root path, conversation ID, and a cancellation signal receiver.

## Turn limit

The agent has a hard cap of 25 tool-use rounds per user request. After 25 rounds, it stops and returns whatever it has, followed by:

```
[Turn limit reached. Starting fresh on your next request.]
```

## Tool result size

Each tool result is capped at 256 KB. If a tool produces more output, it is truncated with a message pointing to a temp file. The agent can `read` the temp file if it needs the full content.

## System prompt

The system prompt is deliberately short (under 500 tokens). Local models like DeepSeek V4 Flash have limited context windows, and long system prompts degrade output quality. The prompt covers identity, tool access, workflow, and rules — nothing more.