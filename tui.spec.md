# TUI Spec (v1)

Terminal UI for the hackpi coding agent, built with ratatui + crossterm.

## Layout

```
┌──────────────────────────────────────────────┐
│  hackpi v0.1.0 · ds4 · 0tks ↑ 0tks ↓           │
├──────────────────────────────────────────────┤
│                                              │
│  ┌──────────────────────────────────────────┐│
│  │  ○ me: add a fibonacci function to cli   ││
│  │                                          ││
│  │  ● assistant:                           ││
│  │  Let me look at the current code...      ││
│  │                                          ││
 │  │  ┌─ ✓ read  src/main.rs [Success] ──────┐ ││
 │  │  │  1#VR:fn main() {                  │ ││
 │  │  │  2#KT:    println!("hello");        │ ││
 │  │  │  3#BH:}                             │ ││
 │  │  └─────────────────────────────────────┘ ││
 │  │                                          ││
 │  │  ┌─ ✓ edit  src/main.rs  (1 op) [Success]┐││
 │  │  │  replace 1#VR → 4 lines               ││
 │  │  │  ✓ Accepted                           ││
 │  │  └──────────────────────────────────────┘││
│  │                                          ││
│  │  Done. Added fibonacci function and      ││
│  │  integrated it into the CLI handler.     ││
│  └──────────────────────────────────────────┘│
│                                              │
│  ┌──────────────────────────────────────────┐│
│  │  > add a fibonacci function              ││
│  └──────────────────────────────────────────┘│
├──────────────────────────────────────────────┤
│  Ctrl+C interrupt · Ctrl+L clear · /help     │
└──────────────────────────────────────────────┘
```

### Regions

1. **Header bar** (1 line)
   - Left: `hackpi v{version}` · `{model_name}`
   - Right: `{input_tokens}↑ {output_tokens}↓`

2. **Conversation area** (remaining space - 3 lines)
   - Scrollable list of messages
   - Each user message: prefixed with `○ me:`
   - Each assistant message: prefixed with `● assistant:`
   - Tool calls render as bordered action cards with structured summary
   - Streaming content renders inline as it arrives
   - Card format:
     ```text
     ┌─ {status_symbol} {tool_title} [{status_label}] ──┐
     │ {content lines}                                   │
     └───────────────────────────────────────────────────┘
     ```
   - Status symbols (three-channel differentiation: glyph + label + color):
     - `✓ [Success]` — green: tool completed successfully
     - `✗ [Failed]` — red: tool returned an error
     - `⚠ [Timeout]` — yellow: tool timed out
     - `⊘ [Cancelled]` — muted: tool was cancelled
     - `⋯ [Running]` — yellow: tool is still executing
   - Tool titles are structured summaries derived from tool name + JSON input:
     - `read  src/main.rs` — shows file path, optional offset/limit
     - `edit  src/main.rs  (2 ops)` — shows path and operation count
     - `bash  cargo test` — shows command (truncated at 60 chars)
     - `search  fn main` — shows search pattern
     - `write  /path/to/file` — shows write target
     - `git  status` — shows git operation
     - `github  PR list` — shows github operation
     - `task  do_something` — shows task command
   - Card types:
     - `read` — shows file content with hashline prefixes
     - `edit` — shows operation, affected lines, accept/reject status
     - `bash` — shows command and output
     - `search` — shows results with file:line matches
     - `write` — shows path and byte count

3. **Input bar** (1-2 lines)
   - `> ` prefix
   - Multi-line input (wraps)
   - Shows placeholder text when empty

4. **Status bar** (1 line)
   - Left: key binding hints
   - Right: connection status indicator

### Key Bindings

| Key | Action |
|---|---|
| `Enter` | Submit prompt |
| `Shift+Enter` | Newline in input |
| `Ctrl+C` | Interrupt current generation |
| `Ctrl+L` | Clear conversation |
| `Ctrl+D` | Exit |
| `PgUp` | Scroll conversation up |
| `PgDn` | Scroll conversation down |
| `Home` | Scroll to top |
| `End` | Scroll to bottom |
| `/` | Start slash command |

### Slash Commands

| Command | Description |
|---|---|
| `/help` | Show help |
| `/clear` | Clear conversation |
| `/model` | Show active model info |
| `/ctx` | Show context usage |
| `/quit` | Exit |

## Rendering

- ratatui with crossterm backend
- 60fps render loop (16ms tick rate)
- Differential rendering (ratatui handles this)
- Spinner animation during LLM response streaming
- Tool action cards with:
  - Distinct border colors per tool type (tool-type color for card frame)
  - Semantic status colors for result content (green=success, red=error, yellow=running/warning)
  - Bordered cards adapt to conversation area width (`area.width`)
  - Content lines are wrapped as `│ {line}` with status-appropriate coloring
- Syntax highlighting in file content (future)

## Interaction Model

1. **Resting**: showing conversation + input prompt
2. **Generating**: streaming response, tool cards appearing
3. **Interrupted**: Ctrl+C stops generation, returns to input
4. **Error**: error message rendered in conversation area

## Implementation

- Tokio main loop with ratatui on the main thread
- LLM client runs in tokio task
- Tool execution dispatched to tokio blocking pool
- Channel-based communication between LLM task and TUI
  - `TuiEvent::StreamChunk(String)` — new response text
  - `TuiEvent::ToolCall(ToolCall)` — tool started (carries optional JSON `input` for summary derivation)
  - `TuiEvent::ToolResult(ToolResult)` — tool completed  
  - `TuiEvent::Error(String)` — error occurred
  - `TuiEvent::Done` — generation complete
