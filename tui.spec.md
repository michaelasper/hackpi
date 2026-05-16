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
│  │  ┌─ read src/main.rs ──────────────────┐ ││
│  │  │  1#VR:fn main() {                  │ ││
│  │  │  2#KT:    println!("hello");        │ ││
│  │  │  3#BH:}                             │ ││
│  │  └─────────────────────────────────────┘ ││
│  │                                          ││
│  │  ┌─ edit src/main.rs ───────────────────┐││
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
   - Tool calls render as bordered cards with title
   - Streaming content renders inline as it arrives
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
- Colored tool cards (distinct border colors per tool type)
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
  - `TuiEvent::ToolCall(ToolCall)` — tool started
  - `TuiEvent::ToolResult(ToolResult)` — tool completed
  - `TuiEvent::Error(String)` — error occurred
  - `TuiEvent::Done` — generation complete
