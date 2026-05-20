# TUI Spec (v1)

Terminal UI for the hackpi coding agent, built with ratatui + crossterm.

## Layout

```
┌──────────────────────────────────────────────┐
│  [Tab] Conversation    [Tab] Tasks    [Tab] Graph  hackpi v0.1.0 · 0↑ 0↓│
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
   - Left: `[Tab] Conversation    [Tab] Tasks    [Tab] Graph`
   - Right: `hackpi v{version} · {input_tokens}↑ {output_tokens}↓`
   - Active tab is underlined; inactive tabs are muted

2. **Main content area** (remaining space - 3 lines)

   **Conversation view** — scrollable list of messages:
   - Each user message: prefixed with `○ me:`
   - Each assistant message: prefixed with `● assistant:`
   - Tool calls render as bordered action cards with structured summary
   - Streaming content renders inline as it arrives
   - Error entries render as bordered error cards with severity tag and optional recovery hint
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

   **Task board view** — grouped list of tasks by state with counts:
   - Section headers: `── In Progress (3) ──...──`
   - Each task: `▸ TSK-001 [In Progress] Implement auth`
   - Blocked-by sub-entries shown indented below task
   - Empty state: "No tasks yet. Press 'n' to create one."

   **Task detail view** — full task information:
   - Title, State, Priority, Workflow, Created/Updated timestamps
   - Assignee, Labels, Blocked-by, Blocking relationships
   - Description section
   - Usage hint at bottom: `/task move {id} done`

   **Task graph view** — dependency visualization:
   - Shows selected task as focal point
   - Blocked-by section (tasks this depends on)
   - Blocks section (tasks that depend on this)
   - Helpful messages for empty cache or no selection

3. **Input bar** (1-2 lines)
   - `> ` prefix
   - Multi-line input (wraps)
   - Shows placeholder text when empty

4. **Status bar** (1 line)
   - Left: dynamic key binding hints derived from the current context
   - Center: UI status indicator (Generating…, Running bash…, Loading tasks…, [ERR] message)
   - Right: connection health indicator (API: connected, API: error, API: offline, API: unknown)

### Key Bindings

The TUI uses an explicit focus model. Key bindings depend on the active context:

| Context | Key | Action |
|---------|-----|--------|
| Global | `Ctrl+C` | Interrupt current generation |
| | `Ctrl+L` | Clear conversation |
| | `Ctrl+D` | Exit hackpi |
| | `?` | Show context help overlay |
| | `Tab` | Cycle views |
| Composer | `Enter` | Submit message |
| | `Shift+Enter` | Newline in input |
| | `Esc` | Clear input |
| | `/` | Start slash command |
| Conversation | `Up` / `Down` | Scroll |
| | `PgUp` / `PgDn` | Scroll faster |
| | `Home` | Scroll to top |
| | `End` | Scroll to bottom |
| Task board | `Up` / `Down` | Navigate tasks |
| | `Enter` | View task detail |
| | `n` | Create task |
| | `Esc` | Go back to conversation |
| Task detail | `Up` / `Down` | Navigate fields |
| | `Esc` | Go back to task board |

### Slash Commands

| Command | Description |
|---|---|
| `/help` | Show help |
| `/clear` | Clear conversation |
| `/quit` | Exit |
| `/guardrails:status` | Show guardrails status |
| `/guardrails:clean` | Clear session cache |
| `/guardrails:onboarding [preset]` | Write preset guardrails config |
| `/git:status` | Show git status |
| `/git:log` | Show recent git log |
| `/github:pr-list` | List open pull requests |
| `/task` | Manage tasks |
| `/tasks` | Alias for /task list |
| `/export [path]` | Export conversation to text file |

## Rendering

- ratatui with crossterm backend
- Differential rendering (ratatui handles this)
- Spinner animation (braille character sequence) during LLM response streaming, tool execution, and task loading
- Tool action cards with:
  - Distinct border colors per tool type (tool-type color for card frame)
  - Semantic status colors for result content (green=success, red=error, yellow=running/warning)
  - Bordered cards adapt to conversation area width (`area.width`)
  - Content lines are wrapped as `│ {line}` with status-appropriate coloring
- Error cards bordered in red with severity tag (`ERROR`, `WARNING`, `INFO`) and optional recovery hint
- Task board: grouped by state with section headers and separator lines
- Task detail: labeled field layout with colored state/priority badges
- Autocomplete: popover above input area with scrollable filtered command list
- Permission modal: centered overlay with decision groups (This request, This session, Persistent rule)
- Responsive modals: scale with terminal size, capped at preferred dimensions
- Minimum terminal size gate (80x24)

## Interaction Model

1. **Resting**: showing conversation + input prompt
2. **Generating**: streaming response, tool cards appearing with spinner
3. **Interrupted**: Ctrl+C stops generation, returns to input, shows "Generation interrupted." message
4. **Waiting for permission**: modal overlay traps all input until decision
5. **Error**: error message rendered in conversation area with bordered card
6. **Loading tasks**: status bar shows spinner while task store is queried

## Future

Items documented in the original spec that are not yet implemented:

- Model name in header bar
- `/model` slash command — track via [COR-187](https://linear.app/corruptbytes/issue/COR-187)
- `/ctx` slash command — track via [COR-187](https://linear.app/corruptbytes/issue/COR-187)
- Fixed 60fps render loop (16ms tick rate) — currently uses event-driven rendering on ratatui's tick
- Syntax highlighting in file content (tool card output)
- Bordered tool card frames with per-tool-type border colors (currently uses a single card frame style per status)
- Graph view showing full dependency DAG (current graph view shows selected task only)

## Implementation

- Tokio main loop with ratatui on the main thread
- LLM client runs in tokio task
- Tool execution dispatched to tokio blocking pool
- Channel-based communication between LLM task and TUI
  - `TuiEvent::StreamChunk(String)` — new response text
  - `TuiEvent::ToolCall { id, name, input }` — tool started (carries optional JSON `input` for summary derivation)
  - `TuiEvent::ToolResult { id, result }` — tool completed
  - `TuiEvent::Usage(Usage)` — token usage update
  - `TuiEvent::PermissionRequest { id, reason, response }` — guardrails permission prompt
  - `TuiEvent::Error(String)` — error occurred
  - `TuiEvent::Done` — generation complete
