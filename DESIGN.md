# Hackpi Design Document

A Rust-based coding agent with a virtual bash filesystem, hash-anchored edits, a context-aware ripgrep wrapper, and a full ratatui TUI. Optimized for local DeepSeek V4 Flash via Anthropic-format API.

## Workspace Structure

Six-crate Rust workspace:

```
hackpi/
├── Cargo.toml              # workspace root
├── DESIGN.md               # this file
├── hashline.spec.md         # edit system spec (ref)
├── tui.spec.md              # TUI layout spec (ref)
├── read-tool.spec.md        # read/search_grep spec (ref)
├── write-tool.spec.md       # write_file spec (ref)
├── bash-tool.spec.md        # virtual bash spec (ref)
├── hackpi-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── agent.rs         # agent loop
│       ├── api.rs           # Anthropic client
│       ├── tools.rs         # tool registry
│       └── types.rs         # shared types
├── hackpi-tools/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── bash.rs          # virtual bash + filesystem
│       ├── edit.rs          # hashline edit tool
│       ├── read.rs          # read tool
│       ├── search_grep.rs   # context-aware rg wrapper
│       └── write.rs         # write tool
├── hackpi-tui/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── app.rs           # TUI state machine
│       ├── ui.rs            # ratatui render functions
│       ├── events.rs        # event channels
│       └── input.rs         # text input handling
├── hackpi-guardrails/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── guards.rs        # guard rule types and evaluation
│       └── config.rs        # rule loading and persistence
├── hackpi-tasks/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── task.rs          # Task, NewTask types and store trait
│       ├── json_store.rs    # JSON-backed task storage
│       └── commands.rs      # slash-command parsing and handling
└── hackpi-vcs/
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        ├── git_read.rs      # git status/log read tool
        └── github.rs        # GitHub PR listing tool
```

### Workspace Cargo.toml

```toml
[workspace]
members = ["hackpi-core", "hackpi-tools", "hackpi-tui", "hackpi-guardrails", "hackpi-tasks", "hackpi-vcs"]
resolver = "2"
```

### Key Dependencies (across crates)

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime, channels |
| `serde` / `serde_json` | Message serialization |
| `reqwest` | HTTP client for API calls |
| `xxhash-rust` | xxHash32 for hashline anchors |
| `grep-searcher` / `grep-regex` | In-process ripgrep |
| `ratatui` + `crossterm` | TUI framework |
| `anyhow` | Error handling |

## Agent Loop (`hackpi-core`)

The central orchestrator. Implements the Anthropic `/v1/messages` streaming API.

### Message Loop

```
┌──────────┐     ┌──────────────┐     ┌───────────┐
│  TUI     │────▶│  Agent Loop  │────▶│  API      │
│  Events  │     │  (hackpi-core) │     │  Client   │
└──────────┘     └──────┬───────┘     └───────────┘
                        │
                   ┌────▼───────┐
                   │   Tool     │
                   │  Registry  │
                   └────┬───────┘
                        │
              ┌─────────┼─────────┐
              │         │         │
         ┌────▼───┐ ┌──▼───┐ ┌───▼────┐
         │ bash   │ │ edit │ │ read   │ ...
         │(hackpi-  │ │(hackpi-│ │(hackpi-  │
         │ tools) │ │tools)│ │ tools) │
         └────────┘ └──────┘ └────────┘
```

### Loop Pseudocode

```
loop:
  messages ← conversation history
  POST /v1/messages { messages, tools, system, stream: true }
  for each SSE event:
    if content_block_delta:    emit text to TUI, append to pending message
    if content_block_start:    prepare tool call accumulator
    if content_block_stop:     finalize content block
    if message_delta:          update stop_reason, usage
    if message_stop:
      for each tool_call in tool_calls:
        dispatch tool via registry
        emit result event to TUI
        append result to messages
      if stop_reason == "tool_use":
        continue (next loop iteration)
      else:
        break (response complete)
```

### Turn Limit

Hard cap at 25 tool-use rounds per user request. Each "round" is one assistant response + any tool calls it makes. After 25 rounds, the agent stops and returns whatever it has so far, followed by `[Turn limit reached. Starting fresh on your next request.]`.

### Streaming Tool Results

Tool results are streamed back to the LLM in the SAME turn. The agent loop:

1. Receives a tool_use content block from the API
2. Dispatches the tool
3. If the tool produces output incrementally (e.g., bash stdout), each chunk is:
   - Sent to the TUI for rendering
   - Buffered in the tool result accumulator
4. When the tool completes, the accumulated result is appended to messages
5. The loop immediately sends the next API request with the tool result included

This is the key difference from a naive loop — tools execute and their results feed back to the LLM without a user round-trip.

### Tool Result Size Limit

Each tool result is capped at 256KB. If a tool produces more output, it's truncated with `[Output truncated: ...]` and the full content is written to a temp file that the model can `read` if needed.

### System Prompt Design

```
You are hackpi, a coding agent built with Rust.
You have access to tools for reading, writing, editing, and searching code.

Workflow:
1. Search/read to understand the codebase
2. Plan your approach
3. Write new files with write_file
4. Edit existing files with edit (using hashline anchors from read output)
5. Use bash to compile, run tests, and verify

Rules:
- Always read a file before editing it
- Check for existing tests before writing new ones
- Verify changes compile and pass tests before declaring done
```

The system prompt is decomposed into sections (identity, tools, workflow, rules) so each can be independently tuned. It stays under 500 tokens — DeepSeek V4 Flash is a local model and long system prompts degrade quality.

## Tool System (`hackpi-tools`)

### Tool Trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

pub struct ToolContext {
    pub workspace_root: PathBuf,
    pub conversation_id: String,
    pub signal: tokio::sync::watch::Receiver<bool>,
}

pub enum ToolResult {
    Success { content: String },
    SystemError { message: String },
    Timeout,
    Cancelled,
}
```

### Tool Registry

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { /* register all tools */ }
    pub fn get(&self, name: &str) -> Option<&dyn Tool>;
    pub fn all_schemas(&self) -> Vec<serde_json::Value>;  // for API
    pub async fn dispatch(&self, name: &str, params: Value, ctx: &ToolContext) -> ToolResult;
}
```

### Registered Tools

| Tool | Crate Location | Spec File |
|---|---|---|
| `read` | `hackpi-tools::read` | [read-tool.spec.md](read-tool.spec.md) |
| `search_grep` | `hackpi-tools::search_grep` | [read-tool.spec.md](read-tool.spec.md) |
| `edit` | `hackpi-tools::edit` | [hashline.spec.md](hashline.spec.md) |
| `write` | `hackpi-tools::write` | [write-tool.spec.md](write-tool.spec.md) |
| `bash` | `hackpi-tools::bash` | [bash-tool.spec.md](bash-tool.spec.md) |

Each tool's implementation details, schema, error handling, and edge cases are in its respective spec. The full spec documents cover:

- **[hashline.spec.md](hashline.spec.md)**: Edit system — LINE#HASH anchoring, read output format, edit operations (replace/append/prepend/replace_text), chained edits, diff preview, hashing algorithm, stale anchor rejection
- **[tui.spec.md](tui.spec.md)**: TUI layout (4 regions), key bindings, slash commands, interaction states, rendering loop, event channels
- **[read-tool.spec.md](read-tool.spec.md)**: search_grep (context-aware ripgrep wrapper with context_lines), read (hashline file reader with offset/limit, large file handling, content type dispatch)
- **[write-tool.spec.md](write-tool.spec.md)**: write_file (new-file-only contract, atomic write, phantom directory handler, path jail, error classification, memory footprint)
- **[bash-tool.spec.md](bash-tool.spec.md)**: Virtual bash (filesystem trait with InMemoryFs/OverlayFs/ReadWriteFs, shell parser, command registry with full v1 command set, execution model, security model)

## Shared Types (`hackpi-core::types`)

```rust
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

pub enum Role { User, Assistant }

pub enum ContentBlock {
    Text(String),
    ToolUse { id: String, name: String, input: Value },
    ToolResult { id: String, content: String },
}

pub struct ApiConfig {
    pub endpoint: String,           // http://127.0.0.1:8000/v1/messages
    pub model: String,              // "ds4"
    pub max_tokens: u32,
    pub temperature: f32,           // 0.0
}

pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}
```

## Data Flow

### User sends a message

```
User types "add fibonacci" → TUI input submit
  → TuiEvent::Submit(String) sent via channel
  → Agent loop receives event
  → Appends user message to conversation
  → POST /v1/messages (streaming)
  → SSE stream → TuiEvent::StreamChunk → TUI renders
  → message_stop event
  → For each tool_use:
    → TuiEvent::ToolCall dispatched → TUI shows card
    → Tool::execute() runs in tokio blocking pool
    → TuiEvent::ToolResult streamed → TUI updates card
    → ToolResult appended to messages
  → If stop_reason=tool_use: loop continues
  → Else: TuiEvent::Done → TUI returns to resting state
```

### Interrupt (Ctrl+C)

```
Ctrl+C keypress → TUI sends signal
  → watch channel set to cancelled
  → In-flight API request aborted (reqwest abort)
  → In-flight tool execution checks signal at yield points
  → Agent loop breaks, returns partial response
  → TUI returns to resting state
```

## Implementation Order

1. **Workspace scaffolding** — Cargo.toml files, crate stubs, dependency resolution
2. **hackpi-core: types + API client** — Shared types, reqwest SSE streaming to `/v1/messages`
3. **hackpi-tui: basic rendering** — 4-region layout, keyboard input, event channels
4. **hackpi-core: agent loop** — Message loop with tool dispatch, streaming, turn limit
5. **hackpi-tools: read + search_grep** — Ripgrep wrapper, hashline file reader
6. **hackpi-tools: write** — Atomic file creation with workspace jail
7. **hackpi-tools: edit** — Hashline edit engine with validation
8. **hackpi-tools: bash** — Virtual filesystem trait, InMemoryFs, shell parser, command registry
9. **Integration testing** — End-to-end workflow tests
10. **Polish** — Error messages, loading states, perf tuning

Steps 1-4 produce a working agent with `read`, `search_grep`, `write`, and `edit`. Step 8 adds the virtual bash. Each step is independently testable.

## Design Constraints

- **Local-first**: Optimized for ds4-server running DeepSeek V4 Flash on localhost. Short system prompts, flat tool schemas, minimal context overhead.
- **Deterministic**: temperature=0, hash-anchored edits, no silent relocation on hash mismatch.
- **Safe by default**: Bash has no network access and no arbitrary exec. Write tool is jailed to workspace. Edit rejects stale anchors.
- **Atomic where it matters**: File writes use temp-file-then-rename. Edit operations validate against a pre-edit snapshot and apply bottom-up.
- **Stream-everything**: Tool results stream to the LLM in the same turn. TUI renders incrementally. No batched delivery.
